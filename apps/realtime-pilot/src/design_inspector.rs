//! Physical planet editor. Generation stays in the library; display is normalized
//! uniformly to keep orbit controls independent of the chosen physical radius.
use crate::{design_controls, design_file};
use bevy::{
    asset::RenderAssetUsages,
    camera::{CameraOutputMode, Viewport, visibility::RenderLayers},
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::BlendState,
    window::PrimaryWindow,
};
use bevy_egui::{
    EguiContexts, EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext, egui,
};
use procgen_realtime_pilot::{
    DesignPreview, DesignPreviewConfig, PlanetDesignConfig, PreviewError, generate_design_preview,
};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

#[derive(Clone, PartialEq)]
struct Request {
    config: PlanetDesignConfig,
    preview: DesignPreviewConfig,
}
struct Generated {
    request: Request,
    preview: DesignPreview,
    elapsed_ms: u128,
}
struct Inspector {
    draft: Request,
    seed_text: String,
    path: String,
    generated: Option<Generated>,
    pending: Option<Receiver<Result<Generated, PreviewError>>>,
    attempted: Option<Request>,
    last_edit: Instant,
    auto_apply: bool,
    show_octaves: bool,
    elevation_colors: bool,
    status: String,
}
impl Inspector {
    fn generate(&mut self) {
        let Ok(seed) = self.seed_text.parse::<u64>() else {
            self.status = "Seed must be an unsigned 64-bit integer.".into();
            return;
        };
        self.draft.config.seed = seed;
        let request = self.draft.clone();
        self.attempted = Some(request.clone());
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        self.status = "Generating preview…".into();
        std::thread::spawn(move || {
            let start = Instant::now();
            let result = generate_design_preview(&request.config, request.preview).map(|preview| {
                Generated {
                    request,
                    preview,
                    elapsed_ms: start.elapsed().as_millis(),
                }
            });
            let _ = sender.send(result);
        });
    }
    fn poll(&mut self) -> bool {
        let Some(receiver) = &self.pending else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.status = "Preview worker stopped.".into();
                return false;
            }
        };
        self.pending = None;
        match result {
            Ok(generated) => {
                self.status = format!(
                    "{} triangles · {} ms",
                    generated.preview.triangles().len(),
                    generated.elapsed_ms
                );
                self.generated = Some(generated);
                true
            }
            Err(error) => {
                self.status = error.to_string();
                false
            }
        }
    }
}

pub fn run(config: PlanetDesignConfig, preview: DesignPreviewConfig, path: Option<PathBuf>) {
    let mut state = Inspector {
        seed_text: config.seed.to_string(),
        draft: Request { config, preview },
        path: path
            .unwrap_or_else(|| PathBuf::from("planet-design.json"))
            .to_string_lossy()
            .into_owned(),
        generated: None,
        pending: None,
        attempted: None,
        last_edit: Instant::now(),
        auto_apply: true,
        show_octaves: false,
        elevation_colors: true,
        status: String::new(),
    };
    state.generate();
    App::new()
        .insert_non_send_resource(state)
        .insert_resource(Orbit {
            yaw: 0.65,
            pitch: 0.6,
            distance: 3.0,
        })
        .insert_resource(ClearColor(Color::srgb(0.025, 0.03, 0.04)))
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 750.0,
            ..default()
        })
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Real-time world pilot — planet design".into(),
                resolution: (1440, 1000).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            orbit.run_if(not(bevy_egui::input::egui_wants_any_pointer_input)),
        )
        .add_systems(EguiPrimaryContextPass, ui)
        .run();
}
#[derive(Resource)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
}
impl Orbit {
    fn transform(&self) -> Transform {
        let p = Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.cos() * self.pitch.cos(),
        ) * self.distance;
        Transform::from_translation(p).looking_at(Vec3::ZERO, Vec3::Y)
    }
}
#[derive(Component)]
struct PreviewCamera;
#[derive(Component)]
struct PreviewEntity;
fn setup(mut commands: Commands, orbit: Res<Orbit>, mut egui_settings: ResMut<EguiGlobalSettings>) {
    egui_settings.auto_create_primary_context = false;
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
        PreviewCamera,
        bevy::core_pipeline::tonemapping::Tonemapping::Reinhard,
        Projection::Perspective(PerspectiveProjection {
            near: 0.0001,
            far: 100.0,
            ..default()
        }),
        orbit.transform(),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 40_000.0,
            ..default()
        },
        Transform::from_xyz(8.0, 12.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
fn orbit(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut orbit: ResMut<Orbit>,
    mut cameras: Query<&mut Transform, With<PreviewCamera>>,
) {
    if buttons.pressed(MouseButton::Left) {
        orbit.yaw -= motion.delta.x * 0.006;
        orbit.pitch = (orbit.pitch + motion.delta.y * 0.006).clamp(-1.5, 1.5);
    }
    let sensitivity = match scroll.unit {
        MouseScrollUnit::Line => 0.08,
        MouseScrollUnit::Pixel => 0.0025,
    };
    orbit.distance = (orbit.distance * (-scroll.delta.y * sensitivity).exp()).clamp(0.05, 20.0);
    for mut camera in &mut cameras {
        *camera = orbit.transform();
    }
}
fn render_mesh(generated: &Generated, elevation_colors: bool) -> Mesh {
    let preview = &generated.preview;
    let positions: Vec<_> = preview
        .positions_m()
        .iter()
        .map(|p| {
            let p = *p * preview.extent_m.recip();
            [p.x, p.y, p.z]
        })
        .collect();
    let colors: Vec<_> = preview
        .heights_m()
        .iter()
        .map(|&height| {
            if !elevation_colors {
                return [0.5, 0.5, 0.5, 1.0];
            }
            let t = (height / generated.request.config.height_limit_m).clamp(-1.0, 1.0);
            if t < 0.0 {
                [0.32 + t * 0.2, 0.40 + t * 0.22, 0.48 + t * 0.2, 1.0]
            } else {
                [0.46 + t * 0.45, 0.43 + t * 0.48, 0.37 + t * 0.54, 1.0]
            }
        })
        .collect();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(
        preview.triangles().iter().flatten().copied().collect(),
    ));
    mesh.compute_smooth_normals();
    mesh
}
#[derive(bevy::ecs::system::SystemParam)]
struct Scene<'w, 's> {
    commands: Commands<'w, 's>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    cameras: Query<'w, 's, &'static mut Camera, With<PreviewCamera>>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    entities: Query<
        'w,
        's,
        (
            Entity,
            &'static Mesh3d,
            &'static MeshMaterial3d<StandardMaterial>,
        ),
        With<PreviewEntity>,
    >,
}
fn ui(mut contexts: EguiContexts, mut state: NonSendMut<Inspector>, mut scene: Scene) -> Result {
    let mut refresh = state.poll();
    let before = state.draft.clone();
    let ctx = contexts.ctx_mut()?;
    let panel = egui::SidePanel::left("design-controls").exact_width(380.0).show(ctx, |ui| {
        ui.heading("Planet design");
        ui.label("Physical scale · CPU height preview");
        ui.label("Drag to orbit · Scroll to zoom");
        ui.horizontal(|ui| {
            ui.checkbox(&mut state.auto_apply, "Auto apply");
            if ui.add_enabled(state.pending.is_none(), egui::Button::new("Generate")).clicked() { state.generate(); }
        });
        ui.label(if !state.auto_apply && state.generated.as_ref().is_some_and(|g| g.request != state.draft) {
            "Edits pending. Press Generate."
        } else {
            &state.status
        });
        ui.separator();
        ui.label("Preset file · meters, seed, and all octave values");
        ui.add(egui::TextEdit::singleline(&mut state.path).id(egui::Id::new("planet-design-path")));
        ui.horizontal(|ui| {
            if ui.button("Load").clicked() {
                match design_file::load(std::path::Path::new(&state.path)) {
                    Ok(config) => { state.seed_text = config.seed.to_string(); state.draft.config = config;
                        state.draft.preview.bands = procgen_realtime_pilot::PreviewBands::Combined;
                        state.status = "Loaded controls.".into(); },
                    Err(e) => state.status = e.to_string(),
                }
            }
            if ui.button("Save controls").clicked() {
                state.status = match state.seed_text.parse::<u64>() {
                    Ok(seed) => { state.draft.config.seed = seed;
                        match design_file::save(std::path::Path::new(&state.path), &state.draft.config) {
                            Ok(()) => "Saved complete design.".into(), Err(e) => e.to_string()
                        }
                    },
                    Err(_) => "Seed must be an unsigned 64-bit integer.".into(),
                };
            }
            if ui.button("Copy JSON").clicked() {
                match state.seed_text.parse::<u64>() {
                    Ok(seed) => { state.draft.config.seed = seed;
                        match design_file::encode(&state.draft.config) {
                            Ok(json) => { ctx.copy_text(json); state.status = "Copied complete design.".into(); },
                            Err(e) => state.status = e.to_string(),
                        }
                    },
                    Err(_) => state.status = "Seed must be an unsigned 64-bit integer.".into(),
                }
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut state.show_octaves, false, "World / preview");
            ui.selectable_value(&mut state.show_octaves, true, "Octaves");
        });
        egui::ScrollArea::vertical().show(ui, |ui| {
            if state.show_octaves {
                let weights = state.generated.as_ref().map(|g| g.preview.octave_weights.clone()).unwrap_or_default();
                let draft = &mut state.draft;
                design_controls::octaves(ui, &mut draft.config, &mut draft.preview.bands, &weights);
            } else {
                ui.label("Seed");
                ui.add(egui::TextEdit::singleline(&mut state.seed_text).id(egui::Id::new("planet-design-seed")));
                if let Ok(seed) = state.seed_text.parse::<u64>() { state.draft.config.seed = seed; }
                else { ui.colored_label(egui::Color32::YELLOW,"Enter an unsigned 64-bit integer."); }
                design_controls::planet(ui, &mut state.draft.config);
                ui.separator();
                design_controls::preview(ui, &mut state.draft.preview);
                refresh |= ui.checkbox(&mut state.elevation_colors, "Elevation colors").changed();
                ui.label("Blue-gray: below datum. Tan: above datum. No water surface or height exaggeration.");
                if let Some(g) = &state.generated {
                    ui.separator();
                    ui.label(format!("Preview spacing: {}",design_controls::distance(g.preview.spacing_m)));
                    ui.label("Full-resolution planetary elevations for the selected bands:");
                    let d = g.preview.distribution;
                    for (name, value) in [("Minimum",d.minimum),("5%",d.p05),("Median",d.median),("95%",d.p95),("Maximum",d.maximum)] {
                        ui.label(format!("{name}: {}",design_controls::distance(value)));
                    }
                }
                ui.separator();
                ui.label("Coarse height preview only. Save this design and use --design --explore --design-file PATH for voxel terrain and walking.");
            }
        });
    });
    let window = scene.windows.single()?;
    let left = (panel.response.rect.width() * ctx.pixels_per_point()).round() as u32;
    let size = UVec2::new(
        window.physical_width().saturating_sub(left),
        window.physical_height(),
    );
    for mut camera in &mut scene.cameras {
        camera.is_active = size.min_element() > 0;
        if camera.is_active {
            camera.viewport = Some(Viewport {
                physical_position: UVec2::new(left, 0),
                physical_size: size,
                ..default()
            });
        }
    }
    if state.draft != before {
        state.last_edit = Instant::now();
    }
    if state.auto_apply
        && state.pending.is_none()
        && state.attempted.as_ref() != Some(&state.draft)
        && state.last_edit.elapsed() >= Duration::from_millis(350)
        && state.seed_text.parse::<u64>().is_ok()
    {
        state.generate();
    }
    if refresh && let Some(generated) = &state.generated {
        for (entity, mesh, material) in &scene.entities {
            scene.meshes.remove(mesh.0.id());
            scene.materials.remove(material.0.id());
            scene.commands.entity(entity).despawn();
        }
        let mesh = scene
            .meshes
            .add(render_mesh(generated, state.elevation_colors));
        let material = scene.materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.9,
            ..default()
        });
        scene
            .commands
            .spawn((Mesh3d(mesh), MeshMaterial3d(material), PreviewEntity));
    }
    Ok(())
}
