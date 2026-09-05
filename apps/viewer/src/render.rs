use crate::{camera::ViewerCamera, model::GeneratedWorld};
use bevy::{camera::visibility::RenderLayers, gizmos::config::GizmoLineConfig, prelude::*};
use procgen_core::Vec3 as SphereVec3;
use procgen_sphere_mesh::{SphereMesh, VoronoiEdge};
use procgen_tectonics::{
    BoundaryClass, BoundaryClassification, CoarseElevation, CrustClass, CrustClassification,
    PlateKinematics, PlatePartition,
};

#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum DiagnosticLayer {
    Points,
    Delaunay,
    Voronoi,
    Plates,
    Crust,
    Elevation,
    Boundaries,
    Motion,
}

const PLATE_BORDER_OFFSET: f32 = 0.004;
const ELEVATION_COLOR_STOPS: [(f32, Vec3); 5] = [
    (0.0, Vec3::new(0.02, 0.08, 0.3)),
    (0.5, Vec3::new(0.08, 0.65, 0.85)),
    // Duplicate sea-level stop deliberately separates water from land.
    (0.5, Vec3::new(0.16, 0.55, 0.18)),
    (0.75, Vec3::new(0.55, 0.38, 0.16)),
    (1.0, Vec3::new(0.96, 0.96, 0.94)),
];

impl DiagnosticLayer {
    pub const ALL: [Self; 8] = [
        Self::Points,
        Self::Delaunay,
        Self::Voronoi,
        Self::Plates,
        Self::Crust,
        Self::Elevation,
        Self::Boundaries,
        Self::Motion,
    ];
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
            Self::Crust => 3.0,
            Self::Elevation => 3.4,
            Self::Boundaries => 4.0,
            Self::Motion => 2.6,
        }
    }

    // Radius order defines the intended composition: Delaunay, Voronoi, plate
    // interiors, crust/elevation, points, plate borders, boundaries, then motion.
    const fn radius(self) -> f32 {
        match self {
            Self::Points => 1.012,
            Self::Delaunay => 1.000,
            Self::Voronoi => 1.006,
            Self::Plates => 1.009,
            Self::Crust => 1.011,
            Self::Elevation => 1.013,
            Self::Boundaries => 1.017,
            Self::Motion => 1.035,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Points => "Cell centers",
            Self::Delaunay => "Delaunay",
            Self::Voronoi => "Voronoi",
            Self::Plates => "Tectonic plates",
            Self::Crust => "Crust classes",
            Self::Elevation => "Coarse elevation",
            Self::Boundaries => "Boundary classes",
            Self::Motion => "Plate motion",
        }
    }

    fn build(self, world: &GeneratedWorld) -> GizmoAsset {
        let radius = self.radius();
        match self {
            Self::Points => point_asset(&world.voronoi, radius),
            Self::Delaunay => delaunay_asset(&world.voronoi, radius),
            Self::Voronoi => voronoi_asset(&world.voronoi, radius),
            Self::Plates => plate_asset(&world.voronoi, &world.plates, radius),
            Self::Crust => crust_asset(&world.voronoi, &world.plates, &world.crust, radius),
            Self::Elevation => elevation_asset(&world.voronoi, &world.elevation, radius),
            Self::Boundaries => boundary_asset(&world.voronoi, &world.boundaries, radius),
            Self::Motion => motion_asset(&world.voronoi, &world.plates, &world.kinematics, radius),
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
        visible[DiagnosticLayer::Elevation.index()] = true;
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

fn point_asset(mesh: &SphereMesh, radius: f32) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let points = &mesh.cell_centers;
    let size = (0.018 / (points.len() as f32).sqrt().max(8.0)).max(0.001);
    for (index, &point) in points.iter().enumerate() {
        let point = to_bevy(point) * radius;
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

fn delaunay_asset(mesh: &SphereMesh, radius: f32) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let color = Color::srgba(0.35, 0.5, 0.72, 0.9);
    for edge in &mesh.edges {
        add_surface_edge(
            &mut asset,
            to_bevy(mesh.cell_centers[edge.cells[0]]),
            to_bevy(mesh.cell_centers[edge.cells[1]]),
            radius,
            color,
        );
    }
    asset
}

fn voronoi_asset(mesh: &SphereMesh, radius: f32) -> GizmoAsset {
    voronoi_edge_asset(mesh, |_, edge| Some((radius, id_color(edge.cells[0]))))
}

fn plate_asset(mesh: &SphereMesh, plates: &PlatePartition, radius: f32) -> GizmoAsset {
    voronoi_edge_asset(mesh, |_, edge| {
        let left_plate = plates.cell_plates[edge.cells[0]];
        let right_plate = plates.cell_plates[edge.cells[1]];
        // White outlines keep this layer useful on its own; the boundary-class
        // layer deliberately overlays them at a slightly larger radius.
        if left_plate == right_plate {
            Some((radius, id_color(left_plate)))
        } else {
            Some((
                radius + PLATE_BORDER_OFFSET,
                Color::srgba(0.95, 0.95, 1.0, 0.98),
            ))
        }
    })
}

fn crust_asset(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: &CrustClassification,
    radius: f32,
) -> GizmoAsset {
    voronoi_edge_asset(mesh, |_, edge| {
        let left = crust.cell_class(plates, edge.cells[0]);
        let right = crust.cell_class(plates, edge.cells[1]);
        let color = if left != right {
            Color::srgba(0.96, 0.96, 1.0, 1.0)
        } else {
            match left {
                CrustClass::Oceanic => Color::srgba(0.12, 0.48, 0.95, 0.98),
                CrustClass::Continental => Color::srgba(0.92, 0.62, 0.2, 0.98),
            }
        };
        Some((radius, color))
    })
}

fn boundary_asset(
    mesh: &SphereMesh,
    boundaries: &BoundaryClassification,
    radius: f32,
) -> GizmoAsset {
    voronoi_edge_asset(mesh, |edge_index, _| {
        let color = match boundaries.edge_classes[edge_index] {
            BoundaryClass::Interior => return None,
            BoundaryClass::Convergent => Color::srgba(1.0, 0.25, 0.18, 1.0),
            BoundaryClass::Divergent => Color::srgba(0.15, 0.6, 1.0, 1.0),
            BoundaryClass::Transform => Color::srgba(1.0, 0.78, 0.12, 1.0),
        };
        Some((radius, color))
    })
}

fn elevation_asset(mesh: &SphereMesh, elevation: &CoarseElevation, radius: f32) -> GizmoAsset {
    voronoi_edge_asset(mesh, |_, edge| {
        let value = (elevation.cell_elevations[edge.cells[0]]
            + elevation.cell_elevations[edge.cells[1]])
            * 0.5;
        Some((radius, elevation_color(value)))
    })
}

fn elevation_color(value: f32) -> Color {
    let color = piecewise_lerp(value, &ELEVATION_COLOR_STOPS);
    Color::srgba(color.x, color.y, color.z, 0.98)
}

fn piecewise_lerp(value: f32, stops: &[(f32, Vec3)]) -> Vec3 {
    let value = value.clamp(stops[0].0, stops[stops.len() - 1].0);
    for pair in stops.windows(2) {
        let (low_value, low) = pair[0];
        let (high_value, high) = pair[1];
        if value < high_value {
            let t = (value - low_value) / (high_value - low_value);
            return low.lerp(high, t);
        }
    }
    stops[stops.len() - 1].1
}

fn motion_asset(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    kinematics: &PlateKinematics,
    radius: f32,
) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let stride = (mesh.cell_count() / 256).max(1);
    for cell in (0..mesh.cell_count()).step_by(stride) {
        let plate = plates.cell_plates[cell];
        let position = mesh.cell_centers[cell];
        let start = to_bevy(position.normalized()) * radius;
        let velocity = to_bevy(kinematics.velocity_at(plate, position));
        if velocity.length_squared() > 1.0e-12 {
            asset.arrow(start, start + velocity * 0.09, id_color(plate));
        }
    }
    asset
}

fn voronoi_edge_asset(
    mesh: &SphereMesh,
    mut style: impl FnMut(usize, &VoronoiEdge) -> Option<(f32, Color)>,
) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        let Some((radius, color)) = style(edge_index, edge) else {
            continue;
        };
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
