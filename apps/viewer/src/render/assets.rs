use crate::model::GeneratedWorld;
use bevy::prelude::*;
use procgen_climate::CALM_WIND_SPEED_METERS_PER_SECOND;
use procgen_core::Vec3 as SphereVec3;
use procgen_geology::{OceanicPeakKind, VolcanicArcField};
use procgen_sphere_mesh::{SphereMesh, VoronoiEdge};
use procgen_tectonics::{BoundaryClass, BoundaryClassification, PlateKinematics, PlatePartition};

const MAXIMUM_VECTOR_COUNT: usize = 256;
const SEAMOUNT_PEAK_COLOR: Vec3 = Vec3::new(1.0, 0.42, 0.08);
const ABYSSAL_HILL_PEAK_COLOR: Vec3 = Vec3::new(0.55, 0.92, 1.0);
const CELL_MARKER_SCALE: f32 = 0.32;
const MINIMUM_CELL_MARKER_SIZE: f32 = 0.003;
const MAXIMUM_CELL_MARKER_SIZE: f32 = 0.012;
pub(super) fn oceanic_peak_markers(world: &GeneratedWorld, radius: f32) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let mesh = &world.voronoi;
    let field = &world.oceanic_peaks;
    let base_size = cell_marker_size(mesh);
    for peak in &field.peaks {
        let color = match peak.kind {
            OceanicPeakKind::Seamount => SEAMOUNT_PEAK_COLOR,
            OceanicPeakKind::AbyssalHill => ABYSSAL_HILL_PEAK_COLOR,
        };
        let position = to_bevy(peak.position.normalized()) * radius;
        add_cross_marker(
            &mut asset,
            position,
            base_size * (0.5 + peak.height.clamp(0.0, 1.5)),
            opaque_color(color),
        );
    }
    asset
}

pub(super) fn volcanic_arc_markers(
    mesh: &SphereMesh,
    field: &VolcanicArcField,
    radius: f32,
) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let marker_size = cell_marker_size(mesh);
    let marker_color = Vec3::new(1.0, 0.95, 0.28);
    for &peak_cell in field.segments.iter().flat_map(|segment| &segment.peaks) {
        let position = to_bevy(mesh.cell_centers[peak_cell].normalized()) * radius;
        add_cross_marker(
            &mut asset,
            position,
            marker_size,
            opaque_color(marker_color),
        );
    }
    asset
}

pub(super) fn point_asset(mesh: &SphereMesh, radius: f32) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let points = &mesh.cell_centers;
    let size = (0.018 / (points.len() as f32).sqrt().max(8.0)).max(0.001);
    for (index, &point) in points.iter().enumerate() {
        let point = to_bevy(point) * radius;
        add_cross_marker(&mut asset, point, size, id_color(index));
    }
    asset
}

fn add_cross_marker(asset: &mut GizmoAsset, position: Vec3, size: f32, color: Color) {
    let reference = if position.y.abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let tangent = position.cross(reference).normalize() * size;
    let bitangent = position.cross(tangent).normalize() * size;
    asset.line(position - tangent, position + tangent, color);
    asset.line(position - bitangent, position + bitangent, color);
}

fn cell_marker_size(mesh: &SphereMesh) -> f32 {
    (CELL_MARKER_SCALE / (mesh.cell_count() as f32).sqrt())
        .clamp(MINIMUM_CELL_MARKER_SIZE, MAXIMUM_CELL_MARKER_SIZE)
}

fn opaque_color(color: Vec3) -> Color {
    Color::srgba(color.x, color.y, color.z, 1.0)
}

pub(super) fn delaunay_asset(mesh: &SphereMesh, radius: f32) -> GizmoAsset {
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

pub(super) fn voronoi_asset(mesh: &SphereMesh, radius: f32) -> GizmoAsset {
    voronoi_edge_asset(mesh, |_, edge| Some((radius, id_color(edge.cells[0]))))
}

pub(super) fn plate_border_asset(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    radius: f32,
) -> GizmoAsset {
    voronoi_edge_asset(mesh, |_, edge| {
        let left_plate = plates.cell_plates[edge.cells[0]];
        let right_plate = plates.cell_plates[edge.cells[1]];
        (left_plate != right_plate).then_some((radius, Color::srgba(0.95, 0.95, 1.0, 0.98)))
    })
}

pub(super) fn boundary_asset(
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

pub(super) fn piecewise_lerp(value: f32, stops: &[(f32, Vec3)]) -> Vec3 {
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

pub(super) fn motion_asset(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    kinematics: &PlateKinematics,
    radius: f32,
) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let stride = (mesh.cell_count() / MAXIMUM_VECTOR_COUNT).max(1);
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

pub(super) fn wind_asset(
    world: &GeneratedWorld,
    radius: f32,
    color_stops: &[(f32, Vec3)],
) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let mesh = &world.voronoi;
    let circulation = &world.atmospheric_circulation;
    let stride = (mesh.cell_count() / MAXIMUM_VECTOR_COUNT).max(1);
    let maximum_speed = circulation
        .diagnostics
        .wind_speed_meters_per_second
        .maximum
        .max(CALM_WIND_SPEED_METERS_PER_SECOND);
    for cell in (0..mesh.cell_count()).step_by(stride) {
        let wind = circulation.cell_wind_meters_per_second[cell];
        let speed = circulation.cell_wind_speed_meters_per_second[cell];
        if speed <= CALM_WIND_SPEED_METERS_PER_SECOND {
            continue;
        }
        let start = to_bevy(mesh.cell_centers[cell].normalized()) * radius;
        let direction = to_bevy(wind) / speed;
        let length = 0.025 + 0.075 * (speed / maximum_speed);
        let color = opaque_color(piecewise_lerp(speed, color_stops));
        asset.arrow(start, start + direction * length, color);
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

pub(super) fn id_color(id: usize) -> Color {
    let hue = (id as f32 * 137.508) % 360.0;
    Color::hsla(hue, 0.62, 0.62, 0.95)
}
