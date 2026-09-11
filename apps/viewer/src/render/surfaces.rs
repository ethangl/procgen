use super::palette::id_color;
use super::{
    SURFACE_RADIUS,
    palette::{
        HOTSPOT_COLOR_STOPS, INSOLATION_COLOR_STOPS, SEAFLOOR_AGE_COLOR_STOPS, opaque_color,
        piecewise_lerp,
    },
    to_bevy,
};
use crate::model::{ClimateWorld, GeologyWorld, TectonicsWorld};
use bevy::{
    asset::RenderAssetUsages,
    color::LinearRgba,
    mesh::{Indices, PrimitiveTopology},
    prelude::{Color, ColorToComponents, Mesh, Vec3},
};
use procgen_sphere_mesh::SphereMesh;
use procgen_tectonics::{CrustClass, ElevationField};

pub(super) fn empty_surface_mesh() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
}

pub(super) fn plate_colors(tectonics: &TectonicsWorld) -> Vec<Color> {
    tectonics
        .plates
        .cell_plates
        .iter()
        .map(|&plate| id_color(plate))
        .collect()
}

pub(super) fn crust_colors(tectonics: &TectonicsWorld) -> Vec<Color> {
    let crust = tectonics.cell_crust();
    (0..tectonics.voronoi.cell_count())
        .map(|cell| match crust.class(cell) {
            CrustClass::Oceanic => Color::srgb(0.12, 0.48, 0.95),
            CrustClass::Continental => Color::srgb(0.92, 0.62, 0.2),
        })
        .collect()
}

pub(super) fn seafloor_age_colors(tectonics: &TectonicsWorld) -> Vec<Color> {
    let maximum_age = tectonics.seafloor_age.diagnostics.summary.maximum.max(1.0);
    tectonics
        .seafloor_age
        .cell_ages
        .iter()
        .map(|age| match age {
            Some(age) => opaque_color(piecewise_lerp(
                *age as f32 / maximum_age,
                SEAFLOOR_AGE_COLOR_STOPS,
            )),
            None => Color::srgb(0.18, 0.16, 0.14),
        })
        .collect()
}

pub(super) fn insolation_colors(_: &TectonicsWorld, climate: &ClimateWorld) -> Vec<Color> {
    let maximum = climate.solar_forcing.diagnostics.daily_mean.maximum;
    let reciprocal = if maximum > 0.0 { maximum.recip() } else { 0.0 };
    climate
        .solar_forcing
        .daily_mean_insolation
        .iter()
        .map(|value| opaque_color(piecewise_lerp(value * reciprocal, INSOLATION_COLOR_STOPS)))
        .collect()
}

/// Trails and flood basalt provinces in one layer, each cell taking whichever
/// of the two reaches further. They never mean the same thing on one cell: a
/// trail is a narrow decaying streak and a province a broad plateau.
pub(super) fn hotspot_colors(_: &TectonicsWorld, geology: &GeologyWorld) -> Vec<Color> {
    geology
        .hotspots
        .cell_intensities
        .iter()
        .zip(&geology.hotspots.cell_plateau)
        .map(|(&intensity, &plateau)| {
            opaque_color(piecewise_lerp(intensity.max(plateau), HOTSPOT_COLOR_STOPS))
        })
        .collect()
}

pub(super) fn basin_colors(_: &TectonicsWorld, geology: &GeologyWorld) -> Vec<Color> {
    geology
        .basins
        .cell_basins
        .iter()
        .map(|basin| basin.map_or(Color::srgb(0.045, 0.065, 0.075), id_color))
        .collect()
}

/// Builds the displaced fan mesh for one coloured elevation field.
///
/// Relief is measured from the field's own datum, so the ocean surface stays
/// at the nominal radius as the datum moves.
pub(super) fn cell_surface_mesh(
    sphere: &SphereMesh,
    colors: &[Color],
    elevation: ElevationField<'_>,
    relief_exaggeration: f32,
) -> Mesh {
    let ElevationField {
        cell_elevations,
        sea_level,
    } = elevation;
    assert_eq!(colors.len(), sphere.cell_count());
    assert_eq!(cell_elevations.len(), sphere.cell_count());
    assert!(relief_exaggeration.is_finite() && relief_exaggeration >= 0.0);

    let corner_elevations = sphere
        .vertex_cells
        .iter()
        .map(|cells| {
            cells.iter().map(|&cell| cell_elevations[cell]).sum::<f32>() / cells.len() as f32
        })
        .collect::<Vec<_>>();

    let center_positions = sphere
        .cell_centers
        .iter()
        .zip(cell_elevations)
        .map(|(&center, &elevation)| {
            displaced_position(to_bevy(center), elevation, sea_level, relief_exaggeration)
        })
        .collect::<Vec<_>>();
    let corner_positions = sphere
        .vertices
        .iter()
        .zip(&corner_elevations)
        .map(|(&corner, &elevation)| {
            displaced_position(to_bevy(corner), elevation, sea_level, relief_exaggeration)
        })
        .collect::<Vec<_>>();
    let mut center_normals = vec![Vec3::ZERO; sphere.cell_count()];
    let mut corner_normals = vec![Vec3::ZERO; sphere.vertex_count()];
    for (cell, &center) in center_positions.iter().enumerate() {
        let corners = sphere.cell_corners(cell);
        for corner in 0..corners.len() {
            let current = corners[corner].vertex;
            let next = corners[(corner + 1) % corners.len()].vertex;
            let face_normal =
                (corner_positions[next] - center).cross(corner_positions[current] - center);
            center_normals[cell] += face_normal;
            corner_normals[current] += face_normal;
            corner_normals[next] += face_normal;
        }
    }

    let vertex_count = sphere.corners.len() + sphere.cell_count();
    let mut positions = Vec::with_capacity(vertex_count);
    let mut normals = Vec::with_capacity(vertex_count);
    let mut vertex_colors = Vec::with_capacity(vertex_count);
    let mut indices = Vec::with_capacity(sphere.corners.len() * 3);

    for (cell, &color) in colors.iter().enumerate() {
        let center = center_positions[cell];
        let corners = sphere.cell_corners(cell);
        let base = u32::try_from(positions.len()).expect("surface mesh exceeds u32 indices");
        let linear_color = LinearRgba::from(color).to_f32_array();

        positions.push(center.to_array());
        normals.push(center_normals[cell].normalize().to_array());
        vertex_colors.push(linear_color);
        for corner in corners {
            let position = corner_positions[corner.vertex];
            positions.push(position.to_array());
            normals.push(corner_normals[corner.vertex].normalize().to_array());
            vertex_colors.push(linear_color);
        }

        for corner in 0..corners.len() {
            let current = base + 1 + corner as u32;
            let next = base + 1 + ((corner + 1) % corners.len()) as u32;
            indices.extend([base, next, current]);
        }
    }

    empty_surface_mesh()
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vertex_colors)
        .with_inserted_indices(Indices::U32(indices))
}

fn displaced_position(
    direction: Vec3,
    elevation: f32,
    sea_level: f32,
    relief_exaggeration: f32,
) -> Vec3 {
    direction.normalize() * surface_radius(elevation, sea_level, relief_exaggeration)
}

fn surface_radius(elevation: f32, sea_level: f32, relief_exaggeration: f32) -> f32 {
    SURFACE_RADIUS + (elevation - sea_level) * relief_exaggeration
}

pub(super) fn maximum_surface_radius(
    elevation: ElevationField<'_>,
    relief_exaggeration: f32,
) -> f32 {
    let maximum_elevation = elevation
        .cell_elevations
        .iter()
        .copied()
        .fold(elevation.sea_level, f32::max);
    surface_radius(maximum_elevation, elevation.sea_level, relief_exaggeration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::{MeshVertexAttribute, VertexAttributeValues};
    use procgen_core::Vec3 as SphereVec3;
    use procgen_sphere_mesh::build_sphere_mesh;

    /// These meshes assert geometric invariants only, so the datum is
    /// deliberately not the pipeline default: nothing here may depend on it.
    const TEST_SEA_LEVEL: f32 = 0.4;

    const TEST_ELEVATIONS: [f32; 4] = [0.0, 0.3, 0.7, 1.0];

    fn test_field() -> ElevationField<'static> {
        ElevationField {
            cell_elevations: &TEST_ELEVATIONS,
            sea_level: TEST_SEA_LEVEL,
        }
    }

    fn tetrahedron_mesh() -> SphereMesh {
        build_sphere_mesh(
            [
                SphereVec3::new(1.0, 1.0, 1.0),
                SphereVec3::new(1.0, -1.0, -1.0),
                SphereVec3::new(-1.0, 1.0, -1.0),
                SphereVec3::new(-1.0, -1.0, 1.0),
            ]
            .into_iter()
            .map(SphereVec3::normalized)
            .collect(),
            1.0,
        )
        .unwrap()
    }

    fn float3_attribute(mesh: &Mesh, attribute: MeshVertexAttribute) -> &[[f32; 3]] {
        let Some(VertexAttributeValues::Float32x3(values)) = mesh.attribute(attribute) else {
            panic!("surface mesh attribute must contain float3 values");
        };
        values
    }

    fn corner_copies(sphere: &SphereMesh, attribute: &[[f32; 3]]) -> Vec<Vec<[f32; 3]>> {
        let mut copies = vec![Vec::new(); sphere.vertex_count()];
        let mut base = 0;
        for cell in 0..sphere.cell_count() {
            for (corner_offset, corner) in sphere.cell_corners(cell).iter().enumerate() {
                copies[corner.vertex].push(attribute[base + 1 + corner_offset]);
            }
            base += 1 + sphere.cell_corners(cell).len();
        }
        copies
    }

    fn assert_shared_corner_values(copies: Vec<Vec<[f32; 3]>>) {
        for corner_copies in copies {
            assert_eq!(corner_copies.len(), 3);
            assert!(
                corner_copies[1..]
                    .iter()
                    .all(|copy| *copy == corner_copies[0])
            );
        }
    }

    #[test]
    fn zero_exaggeration_leaves_every_vertex_at_surface_radius() {
        let sphere = tetrahedron_mesh();
        let mesh = cell_surface_mesh(&sphere, &[Color::WHITE; 4], test_field(), 0.0);

        let positions = float3_attribute(&mesh, Mesh::ATTRIBUTE_POSITION);
        assert!(positions.iter().all(|position| {
            (Vec3::from_array(*position).length() - SURFACE_RADIUS).abs() <= f32::EPSILON
        }));
    }

    #[test]
    fn incident_fans_share_identical_displaced_corner_positions() {
        let sphere = tetrahedron_mesh();
        let mesh = cell_surface_mesh(&sphere, &[Color::WHITE; 4], test_field(), 0.3);
        let positions = float3_attribute(&mesh, Mesh::ATTRIBUTE_POSITION);
        assert_shared_corner_values(corner_copies(&sphere, positions));
    }

    #[test]
    fn displaced_triangle_fans_remain_finite_and_outward() {
        let sphere = tetrahedron_mesh();
        let mesh = cell_surface_mesh(&sphere, &[Color::WHITE; 4], test_field(), 0.4);

        assert_eq!(
            mesh.count_vertices(),
            sphere.cell_count() + sphere.corners.len()
        );
        let Indices::U32(indices) = mesh.indices().unwrap() else {
            panic!("surface mesh must use u32 indices");
        };
        assert_eq!(indices.len(), sphere.corners.len() * 3);
        let positions = float3_attribute(&mesh, Mesh::ATTRIBUTE_POSITION);
        assert!(
            positions
                .iter()
                .flatten()
                .all(|component| component.is_finite())
        );
        for triangle in indices.chunks_exact(3) {
            let [a, b, c] = triangle else { unreachable!() };
            let a = Vec3::from_array(positions[*a as usize]);
            let b = Vec3::from_array(positions[*b as usize]);
            let c = Vec3::from_array(positions[*c as usize]);
            assert!((b - a).cross(c - a).dot(a + b + c) > 0.0);
        }
    }

    #[test]
    fn normals_are_recomputed_from_displaced_geometry() {
        let sphere = tetrahedron_mesh();
        let flat = cell_surface_mesh(&sphere, &[Color::WHITE; 4], test_field(), 0.0);
        let relief = cell_surface_mesh(&sphere, &[Color::WHITE; 4], test_field(), 0.4);
        let flat_normals = float3_attribute(&flat, Mesh::ATTRIBUTE_NORMAL);
        let relief_positions = float3_attribute(&relief, Mesh::ATTRIBUTE_POSITION);
        let relief_normals = float3_attribute(&relief, Mesh::ATTRIBUTE_NORMAL);

        assert_eq!(relief_normals.len(), relief_positions.len());
        for (&position, &normal) in relief_positions.iter().zip(relief_normals) {
            let position = Vec3::from_array(position);
            let normal = Vec3::from_array(normal);
            assert!(normal.is_finite());
            assert!((normal.length() - 1.0).abs() < 1.0e-5);
            assert!(normal.dot(position) > 0.0);
        }
        assert!(
            flat_normals
                .iter()
                .zip(relief_normals)
                .any(
                    |(flat, relief)| Vec3::from_array(*flat).distance(Vec3::from_array(*relief))
                        > 1.0e-4
                )
        );

        assert_shared_corner_values(corner_copies(&sphere, relief_normals));
    }
}
