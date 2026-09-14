//! Native physical exploration. Geometry and movement remain library consumers.
#[path = "physical_navigation.rs"]
mod navigation;
#[path = "physical_inspector_ui.rs"]
mod ui;
use crate::{
    physical_capture::CaptureWrites,
    physical_gpu_bridge::{ExplorationBackend, GpuBridge, GpuCamera},
    physical_gpu_render::{GpuTerrainPlugin, GpuView},
    physical_jobs::{Jobs, TerrainKind, TerrainRequest, TerrainResult},
    physical_record::{ContactFrame, MotionOutcome},
    physical_render::{Coloring, PhysicalUploads, UPLOAD_LIMIT, vector},
};
use bevy::{
    camera::{CameraOutputMode, visibility::RenderLayers},
    prelude::*,
    render::render_resource::BlendState,
};
use bevy_egui::{EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext};
use procgen_realtime_pilot::{
    MeterPosition, PhysicalCollision, PhysicalWalker, PlanetDesignConfig, PlanetDesignField,
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
    editor: crate::design_panel::DesignPanel,
    revision: u64,
    eye: MeterPosition,
    rotation: Quat,
    mode: Navigation,
    walker: Option<PhysicalWalker>,
    landing: bool,
    descent: bool,
    speed_factor: f32,
    coloring: Coloring,
    collision: PhysicalCollision,
    motion: MotionOutcome,
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
    fn collision_position(&self) -> MeterPosition {
        self.walker
            .as_ref()
            .map_or(self.eye, PhysicalWalker::center)
    }
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
        self.status = "Orbiting.".into();
        self.set_radial(self.field.config().radius_m * 2.0);
        self.face_ground();
    }
    fn near_ground(&mut self) {
        self.walker = None;
        self.mode = Navigation::Fly;
        self.status = "Flying near ground.".into();
        self.descent = false;
        self.set_radial(5.0);
        self.face_horizon();
    }
}
impl Inspector {
    fn new(
        config: PlanetDesignConfig,
        path: Option<PathBuf>,
        backend: ExplorationBackend,
        record: Option<crate::physical_record::PhysicalRecord>,
    ) -> Self {
        let field = Arc::new(config.validate().expect("validated design"));
        let radius = field.config().radius_m;
        let mut state = Inspector {
            record,
            gpu: None,
            jobs: Jobs::start(Arc::clone(&field), backend),
            field,
            editor: crate::design_panel::DesignPanel::new(config, path),
            revision: 0,
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
            coloring: Coloring::default(),
            collision: PhysicalCollision::default(),
            motion: MotionOutcome::Idle,
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
        state
    }
    fn receive_collision(
        &mut self,
        field: Arc<PlanetDesignField>,
        result: Result<crate::physical_jobs::CollisionResult, String>,
    ) {
        self.collision_busy = false;
        if !Arc::ptr_eq(&field, &self.field) {
            return;
        }
        match result {
            Ok(result) => {
                self.collision_seconds = result.seconds;
                self.collision
                    .install(result.patch, self.collision_position());
            }
            Err(e) => self.status = e,
        }
    }
}
pub fn run(
    config: PlanetDesignConfig,
    path: Option<PathBuf>,
    backend: ExplorationBackend,
    record: Option<crate::physical_record::PhysicalRecord>,
) {
    let state = Inspector::new(config, path, backend, record);
    let gpu = state.gpu.clone();
    let plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Real-time world pilot — physical exploration".into(),
            resolution: (1440, 1000).into(),
            ..default()
        }),
        ..default()
    });
    // Keep the render queue on the event-loop thread. GPU generation remains
    // asynchronous; the pipelined renderer can stall during macOS teardown.
    let plugins = if backend == ExplorationBackend::Gpu {
        plugins.disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>()
    } else {
        plugins
    };
    let mut app = App::new();
    app.insert_non_send_resource(state)
        .init_resource::<CaptureWrites>()
        .insert_resource(ClearColor(Color::srgb(0.025, 0.03, 0.04)))
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 500.0,
            ..default()
        })
        .add_plugins(plugins)
        .add_plugins((EguiPlugin::default(), PhysicalUploads::default()))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                apply_design,
                receive,
                record_route,
                navigation::movement,
                stream,
                position_scene,
                record_frame,
            )
                .chain(),
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
fn apply_design(mut state: NonSendMut<Inspector>) {
    let Some(bridge) = state.gpu.clone() else {
        return;
    };
    if let Some((revision, field)) = state.editor.edits.request(Instant::now()) {
        bridge.designs.lock().unwrap().request =
            Some(crate::physical_gpu_bridge::DesignRequest { revision, field });
    }
    let ready = bridge.designs.lock().unwrap().take_ready();
    if let Some(publication) = ready {
        state.field = Arc::clone(&publication.design.field);
        state.revision = publication.design.revision;
        // Field, render buffers and collision change together. Never reuse old support.
        state.collision = PhysicalCollision::default();
        state.ground_clearance = None;
        state.collision_seconds = 0.0;
        if let Some(result) = publication.collision {
            state.collision_seconds = result.seconds;
            let position = state.collision_position();
            state.collision.install(result.patch, position);
        }
        *bridge.display.lock().unwrap() = publication.design;
        info!(
            "Applied design revision {} at {:?}",
            state.revision, state.eye
        );
    }
}
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
    if let Ok((field, result)) = state.jobs.patches.try_recv() {
        state.receive_collision(field, result);
    }
}
fn record_route(
    mut state: NonSendMut<Inspector>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
    mut captures: ResMut<CaptureWrites>,
) {
    match captures.poll() {
        Err(error) => {
            error!("Cannot save route capture: {error}");
            exit.write(AppExit::error());
            return;
        }
        Ok(true) if captures.finishing => {
            exit.write(AppExit::Success);
            return;
        }
        _ => {}
    }
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
                captures.finishing = true;
                return;
            }
            None => {}
        }
        if let Some(path) = record.screenshot() {
            captures.request(&mut commands, path);
        }
        state.record = Some(record);
    }
}
fn record_frame(mut state: NonSendMut<Inspector>, time: Res<Time<Real>>) {
    if let Some(mut record) = state.record.take() {
        let contact = ContactFrame {
            walking: state.mode == Navigation::Walk,
            grounded: state.walker.as_ref().is_some_and(PhysicalWalker::grounded),
            covered: state.collision.covers(state.collision_position()),
            building: state.collision_busy,
            build_seconds: state.collision_seconds,
            motion: state.motion,
        };
        let displayed = state
            .gpu
            .as_ref()
            .map(|bridge| bridge.display.lock().unwrap().clone());
        let output = displayed.as_ref().and_then(|d| d.output.try_lock().ok());
        record
            .write(
                time.delta_secs() * 1000.0,
                output.as_ref().map(|o| &o.stats),
                state.eye,
                contact,
            )
            .expect("write route CSV");
        drop(output);
        state.record = Some(record);
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
                Coloring::Height => 3,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical_gpu_bridge::{DesignPublication, DisplayedDesign, GpuOutput};
    use std::sync::{Mutex, atomic::AtomicBool};

    #[test]
    fn publication_switches_design_and_collision_without_moving_camera() {
        let mut config = PlanetDesignConfig::starter(42);
        config.radius_m = 300_000.0;
        config.octaves.iter_mut().for_each(|o| o.enabled = false);
        let mut state = Inspector::new(config.clone(), None, ExplorationBackend::Gpu, None);
        state.near_ground();
        let old_field = Arc::clone(&state.field);
        let eye = state.eye;
        let rotation = state.rotation;
        let old_patch = procgen_realtime_pilot::VoxelCollision::build(
            &old_field,
            eye.anchor(),
            &AtomicBool::new(false),
        )
        .unwrap();
        state.collision.install(old_patch, eye);
        state.editor.edits.seed_text = "43".into();
        state.editor.edits.observe(Instant::now());
        let revision = state.editor.edits.revision();
        let field = Arc::new(state.editor.edits.validated().unwrap());
        let patch = procgen_realtime_pilot::VoxelCollision::build(
            &field,
            eye.anchor(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let output = Arc::new(Mutex::new(GpuOutput::default()));
        let bridge = state.gpu.as_ref().unwrap().clone();
        bridge.designs.lock().unwrap().invalidate(revision);
        assert!(bridge.designs.lock().unwrap().publish(DesignPublication {
            design: DisplayedDesign {
                revision,
                field: Arc::clone(&field),
                output: Arc::clone(&output)
            },
            collision: Some(crate::physical_jobs::CollisionResult {
                patch,
                seconds: 1.0
            }),
        }));
        let mut app = App::new();
        app.insert_non_send_resource(state)
            .add_systems(Update, apply_design);
        app.update();
        let mut state = app.world_mut().non_send_resource_mut::<Inspector>();
        assert_eq!(state.eye, eye);
        assert_eq!(state.rotation, rotation);
        assert!(Arc::ptr_eq(&state.field, &field));
        assert!(Arc::ptr_eq(&bridge.display.lock().unwrap().output, &output));
        assert!(state.collision.covers(eye));
        let stale_patch = procgen_realtime_pilot::VoxelCollision::build(
            &old_field,
            eye.anchor(),
            &AtomicBool::new(false),
        )
        .unwrap();
        state.receive_collision(
            Arc::clone(&old_field),
            Ok(crate::physical_jobs::CollisionResult {
                patch: stale_patch,
                seconds: 99.0,
            }),
        );
        assert_eq!(
            state.collision_seconds, 1.0,
            "old successful jobs cannot replace current support"
        );
        state.status = "current".into();
        state.receive_collision(old_field, Err("obsolete collision failure".into()));
        assert_eq!(state.status, "current");
        assert!(state.collision.covers(eye));
    }
}
