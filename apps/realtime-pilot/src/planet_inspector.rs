use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    time::Instant,
};

use bevy::{
    asset::RenderAssetUsages,
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use procgen_core::Vec3 as Point;
use procgen_realtime_pilot::{
    PILOT_PLANET, PILOT_SHELL, PlanetConfig, PlanetError, ShellConfig, SurfaceMesh, contour_shell,
    planet_overview, sample_shell,
};

pub fn run(seed: u64) {
    let mut state = Inspector {
        config: PILOT_PLANET,
        shell: PILOT_SHELL,
        seed: seed.to_string(),
        pending: None,
        generated: None,
        status: String::new(),
        overview: false,
        regions: false,
        rebase: false,
    };
    state.generate();
    App::new()
        .insert_non_send_resource(state)
        .insert_resource(ClearColor(Color::srgb(0.018, 0.025, 0.04)))
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 100.0,
            ..default()
        })
        .insert_resource(Orbit {
            yaw: 0.65,
            pitch: 0.3,
            distance: 13.0,
        })
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Real-time world pilot — spherical regions".into(),
                resolution: (1280, 900).into(),
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
struct Generated {
    config: PlanetConfig,
    shell: ShellConfig,
    seed: u64,
    detail: SurfaceMesh,
    overview: SurfaceMesh,
    elapsed_ms: u128,
}
struct Inspector {
    config: PlanetConfig,
    shell: ShellConfig,
    seed: String,
    pending: Option<Receiver<Result<Generated, PlanetError>>>,
    generated: Option<Generated>,
    status: String,
    overview: bool,
    regions: bool,
    rebase: bool,
}
impl Inspector {
    fn generate(&mut self) {
        let Ok(seed) = self.seed.parse::<u64>() else {
            self.status = "Seed must be an unsigned 64-bit integer.".into();
            return;
        };
        let config = self.config;
        let shell = self.shell;
        let field = match config.validate(seed) {
            Ok(field) => field,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        self.status = "Generating spherical regions…".into();
        std::thread::spawn(move || {
            let start = Instant::now();
            let result = (|| {
                let overview =
                    planet_overview(&field, procgen_realtime_pilot::OVERVIEW_FACE_QUADS)?;
                let volume = sample_shell(&field, shell)?;
                let detail = contour_shell(&volume)?;
                Ok(Generated {
                    config,
                    shell,
                    seed,
                    detail,
                    overview,
                    elapsed_ms: start.elapsed().as_millis(),
                })
            })();
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
                self.status = "Generation worker stopped.".into();
                return false;
            }
        };
        self.pending = None;
        match result {
            Ok(generated) => {
                self.status = format!(
                    "{} vertices · {} triangles\n{} ms\n{} nonmanifold edges",
                    generated.detail.positions().len(),
                    generated.detail.triangles().len(),
                    generated.elapsed_ms,
                    generated.detail.topology().nonmanifold_edges
                );
                eprintln!(
                    "Generated spherical seed={} config={:#?} shell={:?} {}",
                    generated.seed, generated.config, generated.shell, self.status
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
    fn origin(&self) -> Vec3 {
        if self.rebase {
            Vec3::new(16.0, -8.0, 4.0)
        } else {
            Vec3::ZERO
        }
    }
}
#[derive(Component)]
struct Planet;
#[derive(Component)]
struct OrbitCamera;
#[derive(Resource)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
}
impl Orbit {
    fn transform(&self, origin: Vec3) -> Transform {
        let p = Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.cos() * self.pitch.cos(),
        ) * self.distance;
        Transform::from_translation(p - origin).looking_at(-origin, Vec3::Y)
    }
}
fn setup(mut commands: Commands, orbit: Res<Orbit>) {
    commands.spawn((
        Camera3d::default(),
        bevy::core_pipeline::tonemapping::Tonemapping::Reinhard,
        Projection::Perspective(PerspectiveProjection {
            near: 0.001,
            far: 200.0,
            ..default()
        }),
        orbit.transform(Vec3::ZERO),
        OrbitCamera,
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 8000.0,
            ..default()
        },
        Transform::from_xyz(8.0, 12.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
fn orbit(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    state: NonSend<Inspector>,
    mut orbit: ResMut<Orbit>,
    mut cameras: Query<&mut Transform, With<OrbitCamera>>,
) {
    if buttons.pressed(MouseButton::Left) {
        orbit.yaw -= motion.delta.x * 0.006;
        orbit.pitch = (orbit.pitch + motion.delta.y * 0.006).clamp(-1.5, 1.5);
    }
    let sensitivity = match scroll.unit {
        MouseScrollUnit::Line => 0.08,
        MouseScrollUnit::Pixel => 0.0025,
    };
    orbit.distance = (orbit.distance * (-scroll.delta.y * sensitivity).exp()).clamp(0.2, 80.0);
    for mut camera in &mut cameras {
        *camera = orbit.transform(state.origin());
    }
}

// Rendering owns color and shading; generation exposes ordinary indexed triangles.
fn render_mesh(surface: &SurfaceMesh, origin: Vec3, regions: bool, radius: f32) -> Mesh {
    let positions = surface
        .relative_positions(Point::new(origin.x, origin.y, origin.z))
        .expect("finite UI origin");
    let mut normals = vec![Point::ZERO; positions.len()];
    for triangle in surface.triangles() {
        let [a, b, c] = triangle.vertices.map(|i| positions[i as usize]);
        let normal = (b - a).cross(c - a);
        for i in triangle.vertices {
            normals[i as usize] = normals[i as usize] + normal;
        }
    }
    let mut vertices = Vec::new();
    let mut vertex_normals = Vec::new();
    let mut colors = Vec::new();
    let palette = [
        [0.7, 0.3, 0.2, 1.0],
        [0.3, 0.6, 0.85, 1.0],
        [0.8, 0.65, 0.25, 1.0],
        [0.45, 0.35, 0.7, 1.0],
        [0.25, 0.7, 0.5, 1.0],
        [0.75, 0.4, 0.6, 1.0],
    ];
    for triangle in surface.triangles() {
        for i in triangle.vertices.map(|i| i as usize) {
            let p = positions[i];
            let n = normals[i].normalized();
            vertices.push([p.x, p.y, p.z]);
            vertex_normals.push([n.x, n.y, n.z]);
            let h = surface.positions()[i].length() - radius;
            let t = ((h + 0.15) / 0.5).clamp(0.0, 1.0);
            colors.push(if regions {
                palette[triangle.region.face.index()]
            } else {
                [0.14 + 0.44 * t, 0.23 + 0.30 * t, 0.18 + 0.23 * t, 1.0]
            });
        }
    }
    let indices = (0..vertices.len() as u32).collect();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vertex_normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}

#[derive(bevy::ecs::system::SystemParam)]
struct Scene<'w, 's> {
    commands: Commands<'w, 's>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    planets: Query<'w, 's, Entity, With<Planet>>,
    cameras: Query<'w, 's, &'static mut Transform, With<OrbitCamera>>,
}
fn ui(
    mut contexts: EguiContexts,
    mut state: NonSendMut<Inspector>,
    orbit: Res<Orbit>,
    mut scene: Scene,
) -> Result {
    let mut refresh = state.poll();
    let ctx = contexts.ctx_mut()?;
    egui::SidePanel::left("planet-controls").exact_width(280.0).show(ctx,|ui| {
        egui::ScrollArea::vertical().show(ui,|ui| {
            ui.heading("Spherical regions");ui.label("Slice 2 · CPU dual contouring");ui.separator();
            refresh|=ui.checkbox(&mut state.overview,"Broad elevation overview").changed();
            refresh|=ui.checkbox(&mut state.regions,"Color polygon owners").changed();
            refresh|=ui.checkbox(&mut state.rebase,"Shift render origin").changed();
            ui.label("Drag to orbit · Scroll to zoom");ui.separator();
            ui.label("Seed");ui.text_edit_singleline(&mut state.seed);
            crate::controls::terrain(ui,&mut state.config.terrain);
            ui.collapsing("Sphere and sampling",|ui| {
                ui.horizontal(|ui|{ui.label("Radius");ui.add(egui::DragValue::new(&mut state.config.radius).speed(0.1));});
                ui.horizontal(|ui|{ui.label("Band below");ui.add(egui::DragValue::new(&mut state.config.band.below).speed(0.01));});
                ui.horizontal(|ui|{ui.label("Band above");ui.add(egui::DragValue::new(&mut state.config.band.above).speed(0.01));});
                egui::ComboBox::from_label("Face quads").selected_text(state.shell.face_quads.to_string()).show_ui(ui,|ui|{for side in [32,64,128] {ui.selectable_value(&mut state.shell.face_quads,side,side.to_string());}});
                egui::ComboBox::from_label("Radial cells").selected_text(state.shell.radial_cells.to_string()).show_ui(ui,|ui|{for side in [8,16,32,64] {ui.selectable_value(&mut state.shell.radial_cells,side,side.to_string());}});
            });
            if ui.add_enabled(state.pending.is_none(),egui::Button::new("Generate")).clicked() {state.generate();}
            ui.label(&state.status);
            if let Some(g)=&state.generated && (g.config!=state.config||g.shell!=state.shell||Some(g.seed)!=state.seed.parse().ok()) {ui.colored_label(egui::Color32::YELLOW,"Controls changed. Generate to apply.");}
            ui.separator();ui.label("Six equal-resolution face regions. The overview uses the same broad elevation. Small caves can be unresolved at this spacing.");
        });
    });
    if refresh && let Some(generated) = &state.generated {
        for entity in &scene.planets {
            scene.commands.entity(entity).despawn();
        }
        let surface = if state.overview {
            &generated.overview
        } else {
            &generated.detail
        };
        let mesh = render_mesh(
            surface,
            state.origin(),
            state.regions,
            generated.config.radius,
        );
        let mesh = scene.meshes.add(mesh);
        let material = scene.materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.9,
            ..default()
        });
        scene
            .commands
            .spawn((Mesh3d(mesh), MeshMaterial3d(material), Planet));
        for mut camera in &mut scene.cameras {
            *camera = orbit.transform(state.origin());
        }
    }
    Ok(())
}
