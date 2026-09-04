use crate::{camera::ViewerCamera, model::GeneratedWorld};
use bevy::{camera::visibility::RenderLayers, gizmos::config::GizmoLineConfig, prelude::*};
use procgen_core::Vec3 as SphereVec3;
use procgen_sphere_mesh::{SphereMesh, VoronoiEdge};

#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum DiagnosticLayer {
    Points,
    Delaunay,
    Voronoi,
    Plates,
}

impl DiagnosticLayer {
    pub const ALL: [Self; 4] = [Self::Points, Self::Delaunay, Self::Voronoi, Self::Plates];
    const COUNT: usize = Self::ALL.len();

    const fn index(self) -> usize {
        self as usize
    }

    const fn render_layer(self) -> usize {
        self.index() + 1
    }

    const fn line_width(self) -> f32 {
        match self {
            Self::Points => 1.8,
            Self::Delaunay => 1.1,
            Self::Voronoi => 1.5,
            Self::Plates => 2.4,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Points => "Cell centers",
            Self::Delaunay => "Delaunay",
            Self::Voronoi => "Voronoi",
            Self::Plates => "Tectonic plates",
        }
    }

    fn build(self, world: &GeneratedWorld) -> GizmoAsset {
        match self {
            Self::Points => point_asset(&world.voronoi),
            Self::Delaunay => delaunay_asset(&world.voronoi),
            Self::Voronoi => voronoi_asset(&world.voronoi),
            Self::Plates => plate_asset(world),
        }
    }
}

#[derive(Resource)]
pub struct LayerSettings {
    visible: [bool; DiagnosticLayer::COUNT],
}

impl LayerSettings {
    pub fn is_visible(&self, layer: DiagnosticLayer) -> bool {
        self.visible[layer.index()]
    }

    pub fn set_visible(&mut self, layer: DiagnosticLayer, visible: bool) {
        self.visible[layer.index()] = visible;
    }
}

impl Default for LayerSettings {
    fn default() -> Self {
        let mut visible = [false; DiagnosticLayer::COUNT];
        visible[DiagnosticLayer::Voronoi.index()] = true;
        visible[DiagnosticLayer::Plates.index()] = true;
        Self { visible }
    }
}

#[derive(Resource)]
struct DiagnosticAssets([Handle<GizmoAsset>; DiagnosticLayer::COUNT]);

pub struct DiagnosticRenderPlugin;

impl Plugin for DiagnosticRenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LayerSettings>()
            .add_systems(Startup, setup_scene)
            .add_systems(
                Update,
                (
                    rebuild_diagnostic_assets.run_if(resource_changed::<GeneratedWorld>),
                    sync_visible_layers.run_if(resource_changed::<LayerSettings>),
                ),
            );
    }
}

fn setup_scene(
    mut commands: Commands,
    mut gizmo_assets: ResMut<Assets<GizmoAsset>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(0.985).mesh().ico(5).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.025, 0.035, 0.055),
            perceptual_roughness: 1.0,
            unlit: true,
            ..default()
        })),
    ));

    let diagnostic_assets = DiagnosticLayer::ALL.map(|layer| {
        let handle = gizmo_assets.add(GizmoAsset::new());
        spawn_layer(&mut commands, handle.clone(), layer);
        handle
    });
    spawn_axes(&mut commands, &mut gizmo_assets);

    commands.insert_resource(DiagnosticAssets(diagnostic_assets));
}

fn spawn_layer(commands: &mut Commands, handle: Handle<GizmoAsset>, layer: DiagnosticLayer) {
    commands.spawn((
        Gizmo {
            handle,
            line_config: GizmoLineConfig {
                width: layer.line_width(),
                perspective: false,
                ..default()
            },
            depth_bias: -0.0005,
        },
        RenderLayers::layer(layer.render_layer()),
    ));
}

fn spawn_axes(commands: &mut Commands, assets: &mut Assets<GizmoAsset>) {
    let mut axes = GizmoAsset::new();
    axes.line(
        Vec3::X * 1.08,
        Vec3::X * 1.35,
        Color::srgb(0.95, 0.25, 0.25),
    );
    axes.line(Vec3::Y * 1.08, Vec3::Y * 1.35, Color::srgb(0.3, 0.9, 0.4));
    axes.line(Vec3::Z * 1.08, Vec3::Z * 1.35, Color::srgb(0.3, 0.55, 1.0));
    commands.spawn(Gizmo {
        handle: assets.add(axes),
        line_config: GizmoLineConfig {
            width: 3.0,
            ..default()
        },
        ..default()
    });
}

fn rebuild_diagnostic_assets(
    world: Res<GeneratedWorld>,
    assets: Res<DiagnosticAssets>,
    mut gizmo_assets: ResMut<Assets<GizmoAsset>>,
) {
    for layer in DiagnosticLayer::ALL {
        *gizmo_assets.get_mut(&assets.0[layer.index()]).unwrap() = layer.build(&world);
    }
}

fn sync_visible_layers(
    settings: Res<LayerSettings>,
    mut camera_layers: Single<&mut RenderLayers, With<ViewerCamera>>,
) {
    // Retained gizmos ignore `Visibility`, so filtering must happen on the camera's render layers.
    let mut layers = vec![0];
    layers.extend(
        DiagnosticLayer::ALL
            .into_iter()
            .filter(|&layer| settings.is_visible(layer))
            .map(DiagnosticLayer::render_layer),
    );
    **camera_layers = RenderLayers::from_layers(&layers);
}

fn point_asset(mesh: &SphereMesh) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let points = &mesh.cell_centers;
    let size = (0.018 / (points.len() as f32).sqrt().max(8.0)).max(0.001);
    for (index, &point) in points.iter().enumerate() {
        let point = to_bevy(point) * 1.012;
        let reference = if point.y.abs() < 0.9 {
            Vec3::Y
        } else {
            Vec3::X
        };
        let tangent = point.cross(reference).normalize() * size;
        let bitangent = point.cross(tangent).normalize() * size;
        let color = id_color(index);
        asset.line(point - tangent, point + tangent, color);
        asset.line(point - bitangent, point + bitangent, color);
    }
    asset
}

fn delaunay_asset(mesh: &SphereMesh) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let color = Color::srgba(0.35, 0.5, 0.72, 0.9);
    for edge in &mesh.edges {
        add_surface_edge(
            &mut asset,
            to_bevy(mesh.cell_centers[edge.cells[0]]),
            to_bevy(mesh.cell_centers[edge.cells[1]]),
            1.0,
            color,
        );
    }
    asset
}

fn voronoi_asset(mesh: &SphereMesh) -> GizmoAsset {
    voronoi_edge_asset(mesh, |edge| (1.006, id_color(edge.cells[0])))
}

fn plate_asset(world: &GeneratedWorld) -> GizmoAsset {
    voronoi_edge_asset(&world.voronoi, |edge| {
        let left_plate = world.plates.cell_plates[edge.cells[0]];
        let right_plate = world.plates.cell_plates[edge.cells[1]];
        if left_plate == right_plate {
            (1.009, id_color(left_plate))
        } else {
            (1.013, Color::srgba(0.95, 0.95, 1.0, 0.98))
        }
    })
}

fn voronoi_edge_asset(
    mesh: &SphereMesh,
    mut style: impl FnMut(&VoronoiEdge) -> (f32, Color),
) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    for edge in &mesh.edges {
        let (radius, color) = style(edge);
        add_surface_edge(
            &mut asset,
            to_bevy(mesh.vertices[edge.vertices[0]]),
            to_bevy(mesh.vertices[edge.vertices[1]]),
            radius,
            color,
        );
    }
    asset
}

fn add_surface_edge(asset: &mut GizmoAsset, start: Vec3, end: Vec3, radius: f32, color: Color) {
    let start = start.normalize();
    let end = end.normalize();
    let mut previous = start * radius;
    for segment in 1..=3 {
        let t = segment as f32 / 3.0;
        let current = start.lerp(end, t).normalize() * radius;
        asset.line(previous, current, color);
        previous = current;
    }
}

fn to_bevy(point: SphereVec3) -> Vec3 {
    Vec3::new(point.x, point.y, point.z)
}

fn id_color(id: usize) -> Color {
    let hue = (id as f32 * 137.508) % 360.0;
    Color::hsla(hue, 0.62, 0.62, 0.95)
}
