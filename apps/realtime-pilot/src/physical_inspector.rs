//! Native physical exploration. Geometry and movement remain library consumers.
#[path = "physical_navigation.rs"]
mod navigation;
#[path = "physical_inspector_ui.rs"]
mod ui;
use crate::{
    physical_capture::CaptureWrites,
    physical_gpu_bridge::{GpuBridge, GpuCamera},
    physical_gpu_render::{GpuTerrainPlugin, GpuView},
    physical_render::{Coloring, vector},
};
use bevy::{
    camera::{CameraOutputMode, visibility::RenderLayers},
    prelude::*,
    render::render_resource::BlendState,
};
use bevy_egui::{EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext};
use procgen_realtime_pilot::{MeterPosition, PlanetDesignField, VoxelPosition};
use std::{path::PathBuf, sync::Arc, time::Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Navigation {
    Orbit,
    Fly,
}
#[derive(Component)]
struct PhysicalCamera;
const MAX_ORBIT_CLEARANCE_RADII: f32 = 6.0;

struct Inspector {
    record: Option<crate::physical_record::PhysicalRecord>,
    gpu: GpuBridge,
    field: Arc<PlanetDesignField>,
    keep_above_terrain: bool,
    show_local_voxels: bool,
    editor: crate::design_panel::DesignPanel,
    revision: u64,
    eye: MeterPosition,
    rotation: Quat,
    mode: Navigation,
    descent: bool,
    speed_factor: f32,
    coloring: Coloring,
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
        record: Option<crate::physical_record::PhysicalRecord>,
    ) -> Self {
        let field = Arc::new(document.design.validate().expect("validated design"));
        let radius = field.config().radius_m;
        let eye = MeterPosition::new(
            VoxelPosition {
                x_m: (radius * 3.0) as i32,
                y_m: 0,
                z_m: 0,
            },
            procgen_core::Vec3::ZERO,
        );
        let mut state = Inspector {
            record,
            gpu: GpuBridge::new(Arc::clone(&field), GpuCamera { eye }),
            keep_above_terrain: true,
            show_local_voxels: true,
            field,
            editor: crate::design_panel::DesignPanel::new(document, path),
            revision: 0,
            eye,
            rotation: Quat::IDENTITY,
            mode: Navigation::Orbit,
            descent: false,
            speed_factor: 1.0,
            coloring: Coloring::default(),
            frame_ms: 0.0,
            peak_frame_ms: 0.0,
            status: "GPU height terrain and local voxels · orbit and flight".into(),
        };
        state.face_ground();
        state
    }
}

pub fn run(
    document: crate::design_file::DesignFile,
    path: Option<PathBuf>,
    record: Option<crate::physical_record::PhysicalRecord>,
) {
    let state = Inspector::new(document, path, record);
    let gpu = state.gpu.clone();
    // Keep the render queue on the event-loop thread. GPU generation remains
    // asynchronous; the pipelined renderer can stall during macOS teardown.
    let plugins = DefaultPlugins
        .set(WindowPlugin {
            primary_window: Some(Window {
                title: "Real-time world pilot — physical exploration".into(),
                resolution: (1440, 1000).into(),
                ..default()
            }),
            ..default()
        })
        .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>();
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
        .add_plugins(EguiPlugin::default())
        .add_plugins(GpuTerrainPlugin(gpu))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                apply_design,
                record_route,
                navigation::movement,
                position_scene,
                record_frame,
            )
                .chain(),
        )
        .add_systems(EguiPrimaryContextPass, ui::panel);
    app.run();
}
fn setup(
    state: NonSend<Inspector>,
    mut commands: Commands,
    mut settings: ResMut<EguiGlobalSettings>,
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
    commands.spawn((
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
        bevy::render::view::Hdr,
        Msaa::Off,
        GpuView {
            anchor: [0; 4],
            coloring: 0,
            show_local: state.show_local_voxels,
            eye: state.eye,
            ocean: state.editor.ocean,
        },
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 25000.0,
            ..default()
        },
        Transform::IDENTITY.looking_to(Vec3::new(-1.0, -0.5, -0.6), Vec3::Y),
    ));
}
fn apply_design(mut state: NonSendMut<Inspector>) {
    let bridge = state.gpu.clone();
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
        // The scripted edit goes through the panel's own draft, so it takes the
        // same debounce, revision, and request path a person's edit takes.
        if record.design_edit(state.revision) {
            let octaves = &mut state.editor.edits.config.octaves;
            let finest = octaves.len() - 1;
            octaves[finest].enabled = !octaves[finest].enabled;
        }
        if let Some(path) = record.screenshot() {
            captures.request(&mut commands, path);
        }
        state.record = Some(record);
    }
}
fn record_frame(mut state: NonSendMut<Inspector>, time: Res<Time<Real>>) {
    if let Some(mut record) = state.record.take() {
        let displayed = state.gpu.display.lock().unwrap().clone();
        let output = displayed.output.try_lock().ok();
        record
            .write(
                time.delta_secs() * 1000.0,
                output.as_ref().map(|o| &o.stats),
                state.eye,
                state.clearance_estimate(),
                state.keep_above_terrain,
                state.revision,
            )
            .expect("write route CSV");
        drop(output);
        state.record = Some(record);
    }
}
fn position_scene(
    state: NonSend<Inspector>,
    mut camera: Query<(&mut Transform, &mut Projection, &mut GpuView), With<PhysicalCamera>>,
) {
    let anchor = state.eye.anchor();
    if let Ok(mut camera) = state.gpu.camera.try_lock() {
        *camera = GpuCamera { eye: state.eye };
    }
    for (mut transform, mut projection, mut gpu) in &mut camera {
        gpu.eye = state.eye;
        gpu.ocean = state.editor.ocean;
        gpu.show_local = state.show_local_voxels;
        gpu.anchor = [anchor.x_m, anchor.y_m, anchor.z_m, 0];
        gpu.coloring = match state.coloring {
            Coloring::Neutral => 0,
            Coloring::Lod => 1,
            Coloring::Normals => 2,
            Coloring::Height => 3,
            Coloring::Material => 4,
        };
        *transform = Transform::from_translation(vector(state.eye.relative_to(anchor)))
            .with_rotation(state.rotation);
        if let Projection::Perspective(p) = &mut *projection {
            p.near = (state.clearance_estimate() * 0.0001).clamp(0.05, 1000.0);
        }
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
    }
    #[test]
    fn publication_switches_design_without_moving_the_camera() {
        let mut config = PlanetDesignConfig::starter(42);
        config.radius_m = 300_000.0;
        config.octaves.iter_mut().for_each(|o| o.enabled = false);
        let mut state = Inspector::new(
            crate::design_file::DesignFile::new(config.clone()),
            None,
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
        let bridge = state.gpu.clone();
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
    }
}
