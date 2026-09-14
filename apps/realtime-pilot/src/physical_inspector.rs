//! Native physical exploration. Geometry and movement remain library consumers.
#[path = "physical_inspector_ui.rs"]
mod ui;
use crate::{
    physical_gpu::{ExplorationBackend, GpuBridge, GpuCamera},
    physical_gpu_render::{GpuTerrainPlugin, GpuView},
    physical_jobs::{Jobs, TerrainKind, TerrainRequest, TerrainResult},
    physical_render::{Coloring, PhysicalUploads, UPLOAD_LIMIT, core, vector},
};
use bevy::{
    camera::{CameraOutputMode, visibility::RenderLayers},
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    prelude::*,
    render::render_resource::BlendState,
};
use bevy_egui::{
    EguiContexts, EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext,
};
use procgen_realtime_pilot::{
    MeterPosition, PhysicalWalker, PlanetDesignConfig, PlanetDesignField, VoxelCollision,
    VoxelPosition,
};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc::TryRecvError},
    time::Instant,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Navigation {
    Orbit,
    Fly,
    Walk,
}
struct MeshEntity {
    entity: Entity,
    mesh: Handle<Mesh>,
}
struct Staging {
    result: TerrainResult,
    entities: Vec<MeshEntity>,
}
#[derive(Component)]
struct WorldOrigin(VoxelPosition);
#[derive(Component)]
struct PhysicalCamera;
const MAX_ORBIT_CLEARANCE_RADII: f32 = 6.0;

struct Inspector {
    record: Option<crate::physical_record::PhysicalRecord>,
    gpu: Option<GpuBridge>,
    field: Arc<PlanetDesignField>,
    jobs: Jobs,
    path: String,
    eye: MeterPosition,
    rotation: Quat,
    mode: Navigation,
    walker: Option<PhysicalWalker>,
    landing: bool,
    descent: bool,
    speed_factor: f32,
    coloring: Coloring,
    collision: Option<VoxelCollision>,
    collision_busy: bool,
    collision_seconds: f32,
    terrain_busy: bool,
    last_request: Option<TerrainRequest>,
    staging: Option<Staging>,
    active: Vec<MeshEntity>,
    retiring: bool,
    generation_seconds: f32,
    source_bytes: usize,
    mesh_bytes: usize,
    display_bytes: usize,
    triangles: usize,
    upload_bytes: usize,
    upload_ms: f32,
    peak_upload_ms: f32,
    frame_ms: f32,
    peak_frame_ms: f32,
    query_ms: f32,
    ground_clearance: Option<f32>,
    last_query: Instant,
    status: String,
}
impl Inspector {
    fn up(&self) -> Vec3 {
        vector(self.eye.direction())
    }
    fn height(&self) -> f32 {
        self.field
            .elevation_m(self.eye.direction(), 0.0)
            .expect("camera direction")
    }
    fn clearance_estimate(&self) -> f32 {
        self.eye.altitude_m(self.field.config().radius_m) as f32 - self.height()
    }
    fn set_radial(&mut self, clearance: f32) {
        let delta = clearance - self.clearance_estimate();
        self.eye = self.eye.translated(self.eye.direction() * delta);
    }
    fn face_ground(&mut self) {
        let radial = self.up();
        let camera_up = self.rotation * Vec3::Y;
        let tangent = camera_up - radial * camera_up.dot(radial);
        let up = if tangent.length_squared() > 0.01 {
            tangent.normalize()
        } else {
            radial.any_orthonormal_vector()
        };
        self.rotation = Transform::IDENTITY.looking_to(-radial, up).rotation;
    }
    fn face_horizon(&mut self) {
        let up = self.up();
        let preferred = self.rotation * Vec3::Y;
        let tangent = preferred - up * preferred.dot(up);
        let forward = if tangent.length_squared() > 0.01 {
            tangent.normalize()
        } else {
            up.any_orthonormal_vector()
        };
        self.rotation = Transform::IDENTITY
            .looking_to(forward - up * 0.15, up)
            .rotation;
    }
    fn orbit(&mut self) {
        self.walker = None;
        self.landing = false;
        self.descent = false;
        self.mode = Navigation::Orbit;
        self.set_radial(self.field.config().radius_m * 2.0);
        self.face_ground();
    }
    fn near_ground(&mut self) {
        self.walker = None;
        self.mode = Navigation::Fly;
        self.descent = false;
        self.set_radial(5.0);
        self.face_horizon();
    }
}
pub fn run(
    config: PlanetDesignConfig,
    path: Option<PathBuf>,
    backend: ExplorationBackend,
    record: Option<crate::physical_record::PhysicalRecord>,
) {
    let field = Arc::new(config.validate().expect("validated design"));
    let radius = field.config().radius_m;
    let mut state = Inspector {
        record,
        gpu: None,
        jobs: Jobs::start(Arc::clone(&field), backend),
        field,
        path: path
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "Starter design".into()),
        eye: MeterPosition::new(
            VoxelPosition {
                x_m: (radius * 3.0) as i32,
                y_m: 0,
                z_m: 0,
            },
            procgen_core::Vec3::ZERO,
        ),
        rotation: Quat::IDENTITY,
        mode: Navigation::Orbit,
        walker: None,
        landing: false,
        descent: false,
        speed_factor: 1.0,
        coloring: Coloring::Neutral,
        collision: None,
        collision_busy: false,
        collision_seconds: 0.0,
        terrain_busy: true,
        last_request: None,
        staging: None,
        active: Vec::new(),
        retiring: false,
        generation_seconds: 0.0,
        source_bytes: 0,
        mesh_bytes: 0,
        display_bytes: 0,
        triangles: 0,
        upload_bytes: 0,
        upload_ms: 0.0,
        peak_upload_ms: 0.0,
        frame_ms: 0.0,
        peak_frame_ms: 0.0,
        query_ms: 0.0,
        ground_clearance: None,
        last_query: Instant::now(),
        status: "Building distant overview…".into(),
    };
    state.face_ground();
    if backend == ExplorationBackend::Gpu {
        state.gpu = Some(GpuBridge::new(
            Arc::clone(&state.field),
            GpuCamera { eye: state.eye },
        ));
        state.status = "GPU exploration · collision uses the CPU reference".into();
    }
    let gpu = state.gpu.clone();
    let mut app = App::new();
    app.insert_non_send_resource(state)
        .insert_resource(ClearColor(Color::srgb(0.025, 0.03, 0.04)))
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 500.0,
            ..default()
        })
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Real-time world pilot — physical exploration".into(),
                resolution: (1440, 1000).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((EguiPlugin::default(), PhysicalUploads::default()))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (receive, record_route, movement, stream, position_scene).chain(),
        )
        .add_systems(EguiPrimaryContextPass, ui::panel);
    if let Some(bridge) = gpu {
        app.add_plugins(GpuTerrainPlugin(bridge));
    }
    app.run();
}
fn setup(
    state: NonSend<Inspector>,
    mut commands: Commands,
    mut settings: ResMut<EguiGlobalSettings>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    settings.auto_create_primary_context = false;
    commands.spawn((
        PrimaryEguiContext,
        Camera2d,
        RenderLayers::none(),
        Camera {
            order: 1,
            output_mode: CameraOutputMode::Write {
                blend_state: Some(BlendState::ALPHA_BLENDING),
                clear_color: ClearColorConfig::None,
            },
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
    ));
    let mut camera = commands.spawn((
        Camera3d::default(),
        PhysicalCamera,
        bevy::camera::Exposure::OVERCAST,
        bevy::core_pipeline::tonemapping::Tonemapping::Reinhard,
        Projection::Perspective(PerspectiveProjection {
            near: 0.05,
            far: 100_000_000.0,
            ..default()
        }),
        Transform::default(),
    ));
    if state.gpu.is_some() {
        camera.insert((
            bevy::render::view::Hdr,
            Msaa::Off,
            GpuView {
                anchor: [0; 4],
                coloring: 0,
            },
        ));
    }
    commands.spawn((
        DirectionalLight {
            illuminance: 25000.0,
            ..default()
        },
        Transform::IDENTITY.looking_to(Vec3::new(-1.0, -0.5, -0.6), Vec3::Y),
    ));
    commands.insert_resource(TerrainMaterial(materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.95,
        ..default()
    })));
}
#[derive(Resource)]
struct TerrainMaterial(Handle<StandardMaterial>);
fn receive(mut state: NonSendMut<Inspector>) {
    if state.gpu.is_none() && state.staging.is_none() && !state.retiring {
        match state.jobs.surfaces.try_recv() {
            Ok(Ok(result)) => {
                state.terrain_busy = false;
                state.status = "Uploading terrain…".into();
                state.staging = Some(Staging {
                    result,
                    entities: Vec::new(),
                });
            }
            Ok(Err(e)) => {
                state.terrain_busy = false;
                state.status = e;
            }
            Err(TryRecvError::Disconnected) => state.status = "Terrain worker stopped.".into(),
            Err(TryRecvError::Empty) => {}
        }
    }
    match state.jobs.patches.try_recv() {
        Ok(Ok(result)) => {
            state.collision_busy = false;
            state.collision_seconds = result.seconds;
            if result
                .patch
                .covers(state.eye.relative_to(result.patch.origin_m()), 1.0)
            {
                state.collision = Some(result.patch);
            }
        }
        Ok(Err(e)) => {
            state.collision_busy = false;
            state.status = e;
        }
        Err(TryRecvError::Disconnected) => state.status = "Collision worker stopped.".into(),
        Err(TryRecvError::Empty) => {}
    }
}
fn record_route(
    mut state: NonSendMut<Inspector>,
    time: Res<Time<Real>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    if let Some(mut record) = state.record.take() {
        use crate::physical_record::RouteAction;
        match record.action() {
            Some(RouteAction::Descend) => {
                state.descent = true;
                state.face_ground();
            }
            Some(RouteAction::Ground) => state.near_ground(),
            Some(RouteAction::Walk) => {
                state.landing = true;
            }
            Some(RouteAction::Fly) => {
                state.walker = None;
                state.landing = false;
                state.mode = Navigation::Fly;
            }
            Some(RouteAction::Orbit) => state.orbit(),
            Some(RouteAction::Finish) => {
                record.finish().expect("write route CSV");
                exit.write(AppExit::Success);
            }
            None => {}
        }
        if let Some(path) = record.screenshot() {
            use bevy::render::view::screenshot::{Screenshot, save_to_disk};
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path));
        }
        if let Some(bridge) = &state.gpu
            && let Ok(output) = bridge.output.try_lock()
        {
            record
                .write(
                    time.delta_secs() * 1000.0,
                    &output.stats,
                    state.eye,
                    state.mode == Navigation::Walk,
                    state.collision.is_some(),
                )
                .expect("write route CSV");
        }
        state.record = Some(record);
    }
}
fn movement(
    mut state: NonSendMut<Inspector>,
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mouse: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut contexts: EguiContexts,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let keyboard_blocked = ctx.wants_keyboard_input();
    let pointer_blocked = ctx.wants_pointer_input();
    let dt = time.delta_secs().min(0.05);
    state.frame_ms = state.frame_ms * 0.95 + time.delta_secs() * 1000.0 * 0.05;
    state.peak_frame_ms = state.peak_frame_ms.max(time.delta_secs() * 1000.0);
    let up = state.up();
    if !pointer_blocked && buttons.pressed(MouseButton::Right) {
        state.rotation = Quat::from_axis_angle(up, -mouse.delta.x * 0.003)
            * state.rotation
            * Quat::from_rotation_x(-mouse.delta.y * 0.003);
    }
    if !pointer_blocked && state.mode == Navigation::Orbit && buttons.pressed(MouseButton::Left) {
        let rotation = Quat::from_rotation_y(-mouse.delta.x * 0.003)
            * Quat::from_axis_angle(state.rotation * Vec3::X, -mouse.delta.y * 0.003);
        let radius = state.field.config().radius_m
            + state.eye.altitude_m(state.field.config().radius_m) as f32;
        let p = rotation * up * radius;
        state.eye = MeterPosition::new(
            VoxelPosition {
                x_m: p.x.round() as i32,
                y_m: p.y.round() as i32,
                z_m: p.z.round() as i32,
            },
            procgen_core::Vec3::ZERO,
        );
        state.face_ground();
    }
    let scroll_lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y * 0.02,
    };
    if !pointer_blocked && scroll_lines != 0.0 {
        if state.mode == Navigation::Orbit {
            let clearance = (state.clearance_estimate() * (-scroll_lines * 0.08).exp()).clamp(
                5.0,
                state.field.config().radius_m * MAX_ORBIT_CLEARANCE_RADII,
            );
            state.set_radial(clearance);
            state.face_ground();
        } else {
            state.speed_factor =
                (state.speed_factor * (scroll_lines * 0.08).exp()).clamp(0.25, 8.0);
        }
    }
    if state.descent {
        let clearance = state.clearance_estimate();
        if clearance > 6.0 {
            state.set_radial((clearance * (-dt * 0.8).exp()).max(5.0));
        } else {
            state.descent = false;
            state.mode = Navigation::Fly;
            state.face_horizon();
        }
    }
    let key = |k| {
        if !keyboard_blocked && keys.pressed(k) {
            1.0
        } else {
            0.0
        }
    };
    let mut input = Vec3::new(
        key(KeyCode::KeyD) - key(KeyCode::KeyA),
        key(KeyCode::KeyE) - key(KeyCode::KeyQ),
        key(KeyCode::KeyS) - key(KeyCode::KeyW),
    );
    if state.record.as_ref().is_some_and(|r| r.moving()) {
        input.z = -1.0;
    }
    if state.mode == Navigation::Fly && input != Vec3::ZERO {
        state.descent = false;
        let clearance = state.clearance_estimate();
        let speed = if clearance < 64.0 {
            8.0
        } else if state.eye.altitude_m(state.field.config().radius_m)
            < state.field.config().height_limit_m as f64 + 100.0
        {
            50.0
        } else {
            (clearance * 0.6).clamp(8.0, 2_000_000.0)
        };
        let delta = core(
            (state.rotation * Vec3::new(input.x, 0.0, input.z) + up * input.y).normalize_or_zero()
                * speed
                * state.speed_factor
                * dt,
        );
        if clearance < 32.0 {
            if let Some(collision) = &state.collision {
                match collision.sweep(state.eye.relative_to(collision.origin_m()), delta, 0.2) {
                    Ok(sweep) => {
                        state.eye = MeterPosition::new(collision.origin_m(), sweep.position_m)
                    }
                    Err(e) => state.status = format!("Flight paused: {e}"),
                }
            } else {
                state.status = "Flight paused for nearby collision.".into();
            }
        } else {
            let next = state.eye.translated(delta);
            if next.altitude_m(state.field.config().radius_m)
                <= (state.field.config().radius_m * MAX_ORBIT_CLEARANCE_RADII
                    + state.field.config().height_limit_m) as f64
            {
                state.eye = next;
            } else {
                state.status = "Flight reached the exploration boundary. Move toward the planet or press Orbit.".into();
            }
        }
    }
    let estimated = state.clearance_estimate();
    if estimated < 32.0
        && !state.collision_busy
        && state
            .collision
            .as_ref()
            .is_none_or(|c| !c.covers(state.eye.relative_to(c.origin_m()), 20.0))
    {
        let request = state.eye.anchor();
        if state.jobs.collision.try_send(request).is_ok() {
            state.collision_busy = true;
        }
    }
    if state.landing
        && let Some(collision) = &state.collision
    {
        match PhysicalWalker::land(collision, state.eye) {
            Ok(walker) => {
                state.eye = walker.eye();
                state.walker = Some(walker);
                state.mode = Navigation::Walk;
                state.landing = false;
                state.face_horizon();
                state.status = "Walking on one-meter collision terrain.".into();
            }
            Err(e) => {
                state.status = format!("Landing: {e}");
            }
        }
    }
    if state.mode == Navigation::Walk {
        let started = Instant::now();
        let direction = core(state.rotation * Vec3::new(input.x, 0.0, input.z));
        // Move the walker out briefly to borrow the independent patch.
        if let Some(mut walker) = state.walker.take() {
            if let Some(collision) = &state.collision {
                if let Err(e) = walker.advance(collision, direction, dt) {
                    state.status = format!("Walking paused: {e}");
                }
                state.eye = walker.eye();
            }
            state.walker = Some(walker);
        }
        state.query_ms = started.elapsed().as_secs_f32() * 1000.0;
    }
    if state.last_query.elapsed().as_millis() > 200 {
        state.last_query = Instant::now();
        state.ground_clearance = state.collision.as_ref().and_then(|c| {
            let p = state.eye.relative_to(c.origin_m());
            let up = state.eye.direction();
            c.ray(p, p - up * 24.0)
                .ok()
                .flatten()
                .map(|hit| (p - hit.position_m).length())
        });
    }
}
fn stream(
    mut state: NonSendMut<Inspector>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    material: Res<TerrainMaterial>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    bridge: Res<PhysicalUploads>,
) {
    if state.gpu.is_some() {
        return;
    }
    let start = Instant::now();
    state.upload_bytes = 0;
    let mut receipts = bridge.0.lock().unwrap();
    if state.retiring && receipts.retired {
        state.retiring = false;
        receipts.retired = false;
    }
    if let Some(mut staging) = state.staging.take() {
        while staging
            .result
            .packed
            .pieces
            .front()
            .is_some_and(|p| state.upload_bytes + p.bytes() <= UPLOAD_LIMIT)
            && start.elapsed().as_secs_f32() < 0.002
        {
            let piece = staging.result.packed.pieces.pop_front().unwrap();
            state.upload_bytes += piece.bytes();
            let origin = piece.origin;
            let mesh = meshes.add(piece.mesh());
            receipts.pending.push(mesh.id());
            let entity = commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.0.clone()),
                    WorldOrigin(origin),
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .id();
            staging.entities.push(MeshEntity { entity, mesh });
        }
        if staging.result.packed.pieces.is_empty()
            && staging
                .entities
                .iter()
                .all(|e| receipts.ready.contains(&e.mesh.id()))
        {
            for e in &staging.entities {
                commands.entity(e.entity).insert(Visibility::Visible);
                receipts.ready.remove(&e.mesh.id());
            }
            let mut retired = Vec::new();
            for e in state.active.drain(..) {
                commands.entity(e.entity).despawn();
                meshes.remove(e.mesh.id());
                retired.push(e.mesh.id());
            }
            if !retired.is_empty() {
                receipts.retiring = Some(retired);
                state.retiring = true;
            }
            materials
                .get_mut(&material.0)
                .expect("retained terrain material")
                .unlit = matches!(
                staging.result.kind,
                TerrainKind::Voxels(Coloring::Lod | Coloring::Normals)
            );
            state.active = staging.entities;
            state.generation_seconds = staging.result.generation_seconds;
            state.source_bytes = staging.result.source_bytes;
            state.mesh_bytes = staging.result.mesh_bytes;
            state.display_bytes = staging.result.packed.bytes;
            state.triangles = staging.result.packed.triangles;
            state.status = if matches!(staging.result.kind, TerrainKind::Voxels(_)) {
                "Complete voxel terrain displayed."
            } else {
                "Distant overview displayed."
            }
            .into();
            info!(
                "physical terrain: {} triangles, {:.2} s generation, {} source bytes, {} mesh bytes, {} upload bytes",
                state.triangles,
                state.generation_seconds,
                state.source_bytes,
                state.mesh_bytes,
                state.display_bytes
            );
        } else {
            state.staging = Some(staging);
        }
    }
    state.upload_ms = start.elapsed().as_secs_f32() * 1000.0;
    state.peak_upload_ms = state.peak_upload_ms.max(state.upload_ms);
    if !state.terrain_busy && state.staging.is_none() && !state.retiring {
        let camera = state.eye.anchor();
        let threshold = (state.clearance_estimate() * 0.1).clamp(16.0, 100_000.0);
        let changed = state.last_request.is_none_or(|previous| {
            previous.coloring != state.coloring
                || MeterPosition::new(camera, procgen_core::Vec3::ZERO)
                    .relative_to(previous.camera)
                    .length()
                    > threshold
        });
        if changed {
            let request = TerrainRequest {
                camera,
                coloring: state.coloring,
            };
            if state.jobs.terrain.try_send(request).is_ok() {
                state.last_request = Some(request);
                state.terrain_busy = true;
                state.status = "Generating complete voxel terrain…".into();
            }
        }
    }
}
fn position_scene(
    state: NonSend<Inspector>,
    mut camera: Query<
        (&mut Transform, &mut Projection, Option<&mut GpuView>),
        With<PhysicalCamera>,
    >,
    mut terrain: Query<(&WorldOrigin, &mut Transform), Without<PhysicalCamera>>,
) {
    let anchor = state.eye.anchor();
    if let Some(bridge) = &state.gpu
        && let Ok(mut camera) = bridge.camera.try_lock()
    {
        *camera = GpuCamera { eye: state.eye };
    }
    for (mut transform, mut projection, gpu) in &mut camera {
        if let Some(mut gpu) = gpu {
            gpu.anchor = [anchor.x_m, anchor.y_m, anchor.z_m, 0];
            gpu.coloring = match state.coloring {
                Coloring::Neutral => 0,
                Coloring::Lod => 1,
                Coloring::Normals => 2,
            };
        }
        *transform = Transform::from_translation(vector(state.eye.relative_to(anchor)))
            .with_rotation(state.rotation);
        if let Projection::Perspective(p) = &mut *projection {
            p.near = (state.clearance_estimate() * 0.0001).clamp(0.05, 1000.0);
        }
    }
    for (origin, mut transform) in &mut terrain {
        let p = origin.0.relative_to(anchor);
        transform.translation = Vec3::new(p.x_m as f32, p.y_m as f32, p.z_m as f32);
    }
}
