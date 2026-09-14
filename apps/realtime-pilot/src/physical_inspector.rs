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
    physical_render::{Coloring, PhysicalUploads, UPLOAD_LIMIT, vector},
};
use bevy::{
    camera::{CameraOutputMode, visibility::RenderLayers},
    prelude::*,
    render::render_resource::BlendState,
};
use bevy_egui::{EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext};
use procgen_realtime_pilot::{MeterPosition, PlanetDesignField, VoxelPosition};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc::TryRecvError},
    time::Instant,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Navigation {
    Orbit,
    Fly,
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
    jobs: Option<Jobs>,
    keep_above_terrain: bool,
    editor: crate::design_panel::DesignPanel,
    revision: u64,
    eye: MeterPosition,
    rotation: Quat,
    mode: Navigation,
    descent: bool,
    speed_factor: f32,
    coloring: Coloring,
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
    status: String,
}
impl Inspector {
    fn protect_altitude(&mut self) {
        if self.keep_above_terrain {
            self.eye = procgen_realtime_pilot::keep_camera_above_terrain(&self.field, self.eye);
        }
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
        self.descent = false;
        self.mode = Navigation::Orbit;
        self.status = "Orbiting.".into();
        self.set_radial(self.field.config().radius_m * 2.0);
        self.face_ground();
    }
    fn near_ground(&mut self) {
        self.mode = Navigation::Fly;
        self.status = "Flying near ground.".into();
        self.descent = false;
        self.set_radial(procgen_realtime_pilot::CAMERA_CLEARANCE_M);
        self.face_horizon();
    }
}
impl Inspector {
    fn new(
        document: crate::design_file::DesignFile,
        path: Option<PathBuf>,
        backend: ExplorationBackend,
        record: Option<crate::physical_record::PhysicalRecord>,
    ) -> Self {
        let field = Arc::new(document.design.validate().expect("validated design"));
        let radius = field.config().radius_m;
        let mut state = Inspector {
            record,
            gpu: None,
            jobs: (backend == ExplorationBackend::Cpu).then(|| Jobs::start(Arc::clone(&field))),
            keep_above_terrain: true,
            field,
            editor: crate::design_panel::DesignPanel::new(document, path),
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
            descent: false,
            speed_factor: 1.0,
            coloring: Coloring::default(),
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
            status: "Building distant overview…".into(),
        };
        state.face_ground();
        if backend == ExplorationBackend::Gpu {
            state.gpu = Some(GpuBridge::new(
                Arc::clone(&state.field),
                GpuCamera { eye: state.eye },
            ));
            state.status = "GPU height terrain · orbit and flight".into();
        }
        state
    }
}

pub fn run(
    document: crate::design_file::DesignFile,
    path: Option<PathBuf>,
    backend: ExplorationBackend,
    record: Option<crate::physical_record::PhysicalRecord>,
) {
    let state = Inspector::new(document, path, backend, record);
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
                eye: state.eye,
                ocean: state.editor.ocean,
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
        state.field = Arc::clone(&publication.field);
        state.revision = publication.revision;
        *bridge.display.lock().unwrap() = publication;
        info!(
            "Applied design revision {} at {:?}",
            state.revision, state.eye
        );
    }
}
fn receive(mut state: NonSendMut<Inspector>) {
    if state.gpu.is_none() && state.staging.is_none() && !state.retiring {
        match state
            .jobs
            .as_ref()
            .expect("CPU audit worker")
            .surfaces
            .try_recv()
        {
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
            Some(RouteAction::Fly) => {
                state.mode = Navigation::Fly;
            }
            Some(RouteAction::Rise) => {
                state.mode = Navigation::Fly;
            }
            Some(RouteAction::Orbit) => state.orbit(),
            Some(RouteAction::View { clearance_m, view }) => {
                state.descent = false;
                state.mode = Navigation::Fly;
                state.set_radial(clearance_m);
                let up = state.up();
                // Fixed tangent makes repeat views independent of the previous rotation.
                let tangent = (Vec3::Y - up * up.y).normalize();
                state.rotation = match view {
                    crate::physical_record::RecordView::Down => {
                        Transform::IDENTITY.looking_to(-up, tangent).rotation
                    }
                    crate::physical_record::RecordView::Horizon => {
                        Transform::IDENTITY.looking_to(tangent, up).rotation
                    }
                };
            }
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
                state.clearance_estimate(),
                state.keep_above_terrain,
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
            if state
                .jobs
                .as_ref()
                .expect("CPU audit worker")
                .terrain
                .try_send(request)
                .is_ok()
            {
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
            gpu.eye = state.eye;
            gpu.ocean = state.editor.ocean;
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
    use crate::physical_gpu_bridge::{DisplayedDesign, GpuOutput};
    use procgen_realtime_pilot::PlanetDesignConfig;
    use std::sync::Mutex;

    #[test]
    fn altitude_protection_can_be_disabled_and_reenabled_below_terrain() {
        let mut state = Inspector::new(
            crate::design_file::DesignFile::new(PlanetDesignConfig::starter(42)),
            None,
            ExplorationBackend::Gpu,
            None,
        );
        state.set_radial(-50.0);
        let below = state.eye;
        state.keep_above_terrain = false;
        state.protect_altitude();
        assert_eq!(state.eye, below);
        state.keep_above_terrain = true;
        state.protect_altitude();
        assert!(
            (state.clearance_estimate() - procgen_realtime_pilot::CAMERA_CLEARANCE_M).abs() < 0.001
        );
        assert!(state.jobs.is_none());
    }
    #[test]
    fn publication_switches_design_without_moving_camera_or_starting_cpu_work() {
        let mut config = PlanetDesignConfig::starter(42);
        config.radius_m = 300_000.0;
        config.octaves.iter_mut().for_each(|o| o.enabled = false);
        let mut state = Inspector::new(
            crate::design_file::DesignFile::new(config.clone()),
            None,
            ExplorationBackend::Gpu,
            None,
        );
        state.near_ground();
        let eye = state.eye;
        let rotation = state.rotation;
        state.editor.edits.seed_text = "43".into();
        state.editor.edits.observe(Instant::now());
        let revision = state.editor.edits.revision();
        let field = Arc::new(state.editor.edits.validated().unwrap());
        let output = Arc::new(Mutex::new(GpuOutput::default()));
        let bridge = state.gpu.as_ref().unwrap().clone();
        bridge.designs.lock().unwrap().invalidate(revision);
        assert!(bridge.designs.lock().unwrap().publish(DisplayedDesign {
            revision,
            field: Arc::clone(&field),
            output: Arc::clone(&output),
        }));
        let mut app = App::new();
        app.insert_non_send_resource(state)
            .add_systems(Update, apply_design);
        app.update();
        let state = app.world_mut().non_send_resource_mut::<Inspector>();
        assert_eq!(state.eye, eye);
        assert_eq!(state.rotation, rotation);
        assert!(Arc::ptr_eq(&state.field, &field));
        assert!(Arc::ptr_eq(&bridge.display.lock().unwrap().output, &output));
        assert!(state.jobs.is_none());
    }
}
