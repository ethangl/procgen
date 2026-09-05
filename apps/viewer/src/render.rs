use crate::{camera::ViewerCamera, model::GeneratedWorld};
use bevy::{camera::visibility::RenderLayers, gizmos::config::GizmoLineConfig, prelude::*};
use procgen_core::Vec3 as SphereVec3;
use procgen_sphere_mesh::{SphereMesh, VoronoiEdge};
use procgen_tectonics::{
    BoundaryClass, BoundaryClassification, CrustClass, CrustClassification, PlateKinematics,
    PlatePartition, SeafloorAge,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum DiagnosticLayer {
    Points,
    Delaunay,
    Voronoi,
    Plates,
    Crust,
    SeafloorAge,
    BaseElevation,
    Deformation,
    Elevation,
    Hotspots,
    Boundaries,
    Motion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DrawSurface {
    Layer(DiagnosticLayer),
    PlateBorders,
}

const DRAW_ORDER: [DrawSurface; 13] = [
    DrawSurface::Layer(DiagnosticLayer::Delaunay),
    DrawSurface::Layer(DiagnosticLayer::Voronoi),
    DrawSurface::Layer(DiagnosticLayer::Plates),
    DrawSurface::Layer(DiagnosticLayer::Crust),
    DrawSurface::Layer(DiagnosticLayer::Points),
    DrawSurface::Layer(DiagnosticLayer::SeafloorAge),
    DrawSurface::Layer(DiagnosticLayer::BaseElevation),
    DrawSurface::Layer(DiagnosticLayer::Deformation),
    DrawSurface::Layer(DiagnosticLayer::Elevation),
    DrawSurface::Layer(DiagnosticLayer::Hotspots),
    DrawSurface::PlateBorders,
    DrawSurface::Layer(DiagnosticLayer::Boundaries),
    DrawSurface::Layer(DiagnosticLayer::Motion),
];
const DRAW_RADIUS_BASE: f32 = 1.0;
const DRAW_RADIUS_STEP: f32 = 0.004;
const DEFORMATION_COLOR_STOPS: [(f32, Vec3); 3] = [
    (-0.5, Vec3::new(0.08, 0.35, 0.95)),
    (0.0, Vec3::new(0.12, 0.12, 0.16)),
    (0.5, Vec3::new(1.0, 0.38, 0.08)),
];
const SEAFLOOR_AGE_COLOR_STOPS: [(f32, Vec3); 3] = [
    (0.0, Vec3::new(0.35, 0.95, 1.0)),
    (0.5, Vec3::new(0.08, 0.4, 0.8)),
    (1.0, Vec3::new(0.015, 0.05, 0.2)),
];
const ELEVATION_COLOR_STOPS: [(f32, Vec3); 5] = [
    (0.0, Vec3::new(0.02, 0.08, 0.3)),
    (0.5, Vec3::new(0.08, 0.65, 0.85)),
    // Duplicate sea-level stop deliberately separates water from land.
    (0.5, Vec3::new(0.16, 0.55, 0.18)),
    (0.75, Vec3::new(0.55, 0.38, 0.16)),
    (1.0, Vec3::new(0.96, 0.96, 0.94)),
];
const HOTSPOT_COLOR_STOPS: [(f32, Vec3); 4] = [
    (0.0, Vec3::new(0.08, 0.06, 0.12)),
    (0.25, Vec3::new(0.55, 0.08, 0.3)),
    (0.65, Vec3::new(1.0, 0.25, 0.05)),
    (1.0, Vec3::new(1.0, 0.95, 0.25)),
];

impl DiagnosticLayer {
    pub const ALL: [Self; 12] = [
        Self::Points,
        Self::Delaunay,
        Self::Voronoi,
        Self::Plates,
        Self::Crust,
        Self::SeafloorAge,
        Self::BaseElevation,
        Self::Deformation,
        Self::Elevation,
        Self::Hotspots,
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
            Self::SeafloorAge => 3.1,
            Self::BaseElevation => 3.2,
            Self::Deformation => 3.3,
            Self::Elevation => 3.5,
            Self::Hotspots => 3.7,
            Self::Boundaries => 4.0,
            Self::Motion => 2.6,
        }
    }

    fn radius(self) -> f32 {
        draw_radius(DrawSurface::Layer(self))
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Points => "Cell centers",
            Self::Delaunay => "Delaunay",
            Self::Voronoi => "Voronoi",
            Self::Plates => "Tectonic plates",
            Self::Crust => "Crust classes",
            Self::SeafloorAge => "Seafloor age",
            Self::BaseElevation => "Base elevation",
            Self::Deformation => "Boundary deformation",
            Self::Elevation => "Coarse elevation",
            Self::Hotspots => "Mantle hotspots",
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
            Self::Plates => plate_asset(
                &world.voronoi,
                &world.plates,
                radius,
                draw_radius(DrawSurface::PlateBorders),
            ),
            Self::Crust => crust_asset(&world.voronoi, &world.plates, &world.crust, radius),
            Self::SeafloorAge => seafloor_age_asset(&world.voronoi, &world.seafloor_age, radius),
            Self::BaseElevation => scalar_field_asset(
                &world.voronoi,
                &world.base_elevation.cell_elevations,
                &ELEVATION_COLOR_STOPS,
                radius,
            ),
            Self::Deformation => scalar_field_asset(
                &world.voronoi,
                &world.deformation.cell_deformation,
                &DEFORMATION_COLOR_STOPS,
                radius,
            ),
            Self::Elevation => scalar_field_asset(
                &world.voronoi,
                &world.elevation.cell_elevations,
                &ELEVATION_COLOR_STOPS,
                radius,
            ),
            Self::Hotspots => scalar_field_asset(
                &world.voronoi,
                &world.hotspots.cell_intensities,
                &HOTSPOT_COLOR_STOPS,
                radius,
            ),
            Self::Boundaries => boundary_asset(&world.voronoi, &world.boundaries, radius),
            Self::Motion => motion_asset(&world.voronoi, &world.plates, &world.kinematics, radius),
        }
    }
}

fn seafloor_age_asset(mesh: &SphereMesh, age: &SeafloorAge, radius: f32) -> GizmoAsset {
    let maximum_age = age.diagnostics.summary.maximum.max(1.0);
    voronoi_edge_asset(mesh, |_, edge| {
        let ages = edge.cells.map(|cell| age.cell_ages[cell]);
        let color = match ages {
            [None, None] => Color::srgba(0.18, 0.16, 0.14, 0.75),
            [Some(_), None] | [None, Some(_)] => Color::srgba(0.96, 0.96, 1.0, 1.0),
            [Some(left), Some(right)] => scalar_edge_color(
                [left as f32 / maximum_age, right as f32 / maximum_age],
                &SEAFLOOR_AGE_COLOR_STOPS,
            ),
        };
        Some((radius, color))
    })
}

fn draw_radius(surface: DrawSurface) -> f32 {
    let position = DRAW_ORDER
        .iter()
        .position(|&candidate| candidate == surface)
        .expect("every draw surface must have a declared order");
    DRAW_RADIUS_BASE + position as f32 * DRAW_RADIUS_STEP
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

fn plate_asset(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    radius: f32,
    border_radius: f32,
) -> GizmoAsset {
    voronoi_edge_asset(mesh, |_, edge| {
        let left_plate = plates.cell_plates[edge.cells[0]];
        let right_plate = plates.cell_plates[edge.cells[1]];
        // White outlines keep this layer useful on its own; the boundary-class
        // layer deliberately overlays them at a slightly larger radius.
        if left_plate == right_plate {
            Some((radius, id_color(left_plate)))
        } else {
            Some((border_radius, Color::srgba(0.95, 0.95, 1.0, 0.98)))
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

fn scalar_field_asset(
    mesh: &SphereMesh,
    values: &[f32],
    stops: &[(f32, Vec3)],
    radius: f32,
) -> GizmoAsset {
    voronoi_edge_asset(mesh, |_, edge| {
        Some((
            radius,
            scalar_edge_color(edge.cells.map(|cell| values[cell]), stops),
        ))
    })
}

fn scalar_edge_color(values: [f32; 2], stops: &[(f32, Vec3)]) -> Color {
    let color = piecewise_lerp((values[0] + values[1]) * 0.5, stops);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_order_covers_each_surface_once_with_unique_radii() {
        for layer in DiagnosticLayer::ALL {
            assert_eq!(
                DRAW_ORDER
                    .iter()
                    .filter(|&&surface| surface == DrawSurface::Layer(layer))
                    .count(),
                1
            );
        }
        assert_eq!(
            DRAW_ORDER
                .iter()
                .filter(|&&surface| surface == DrawSurface::PlateBorders)
                .count(),
            1
        );

        let radii: Vec<_> = DRAW_ORDER.iter().copied().map(draw_radius).collect();
        assert!(radii.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
