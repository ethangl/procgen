use super::{
    SURFACE_RADIUS,
    palette::{
        INSOLATION_COLOR_STOPS, SEAFLOOR_AGE_COLOR_STOPS, id_color, opaque_color, piecewise_lerp,
    },
    to_bevy,
};
use crate::model::GeneratedWorld;
use bevy::{
    asset::RenderAssetUsages,
    color::LinearRgba,
    mesh::{Indices, PrimitiveTopology},
    prelude::{Color, ColorToComponents, Mesh, Vec3},
};
use procgen_sphere_mesh::SphereMesh;
use procgen_tectonics::{CrustClass, SEA_LEVEL};

pub(super) fn empty_surface_mesh() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
}

pub(super) fn scalar_surface_mesh(
    world: &GeneratedWorld,
    values: &[f32],
    stops: &[(f32, Vec3)],
    relief_exaggeration: f32,
) -> Mesh {
    cell_surface_mesh(
        &world.voronoi,
        values
            .iter()
            .map(|&value| opaque_color(piecewise_lerp(value, stops))),
        &world.isostasy.cell_elevations,
        relief_exaggeration,
    )
}

pub(super) fn plate_surface_mesh(world: &GeneratedWorld, relief_exaggeration: f32) -> Mesh {
    cell_surface_mesh(
        &world.voronoi,
        world
            .plates
            .cell_plates
            .iter()
            .map(|&plate| id_color(plate)),
        &world.isostasy.cell_elevations,
        relief_exaggeration,
    )
}

pub(super) fn crust_surface_mesh(world: &GeneratedWorld, relief_exaggeration: f32) -> Mesh {
    cell_surface_mesh(
        &world.voronoi,
        (0..world.voronoi.cell_count()).map(|cell| {
            match world.crust.cell_class(&world.plates, cell) {
                CrustClass::Oceanic => Color::srgb(0.12, 0.48, 0.95),
                CrustClass::Continental => Color::srgb(0.92, 0.62, 0.2),
            }
        }),
        &world.isostasy.cell_elevations,
        relief_exaggeration,
    )
}

pub(super) fn seafloor_age_surface_mesh(world: &GeneratedWorld, relief_exaggeration: f32) -> Mesh {
    let maximum_age = world.seafloor_age.diagnostics.summary.maximum.max(1.0);
    cell_surface_mesh(
        &world.voronoi,
        world.seafloor_age.cell_ages.iter().map(|age| match age {
            Some(age) => opaque_color(piecewise_lerp(
                *age as f32 / maximum_age,
                SEAFLOOR_AGE_COLOR_STOPS,
            )),
            None => Color::srgb(0.18, 0.16, 0.14),
        }),
        &world.isostasy.cell_elevations,
        relief_exaggeration,
    )
}

pub(super) fn insolation_surface_mesh(world: &GeneratedWorld, relief_exaggeration: f32) -> Mesh {
    let maximum = world.solar_forcing.diagnostics.daily_mean.maximum;
    let reciprocal = if maximum > 0.0 { maximum.recip() } else { 0.0 };
    let values = world
        .solar_forcing
        .daily_mean_insolation
        .iter()
        .map(|value| value * reciprocal);
    cell_surface_mesh(
        &world.voronoi,
        values.map(|value| opaque_color(piecewise_lerp(value, INSOLATION_COLOR_STOPS))),
        &world.isostasy.cell_elevations,
        relief_exaggeration,
    )
}

pub(super) fn basin_surface_mesh(world: &GeneratedWorld, relief_exaggeration: f32) -> Mesh {
    cell_surface_mesh(
        &world.voronoi,
        world
            .basins
            .cell_basins
            .iter()
            .map(|basin| basin.map_or(Color::srgb(0.045, 0.065, 0.075), id_color)),
        &world.isostasy.cell_elevations,
        relief_exaggeration,
    )
}

fn cell_surface_mesh(
    sphere: &SphereMesh,
    colors: impl IntoIterator<Item = Color>,
    cell_elevations: &[f32],
    relief_exaggeration: f32,
) -> Mesh {
    let colors = colors.into_iter().collect::<Vec<_>>();
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

    let vertex_count = sphere.corners.len() + sphere.cell_count();
    let mut positions = Vec::with_capacity(vertex_count);
    let mut normal_sources = Vec::with_capacity(vertex_count);
    let mut vertex_colors = Vec::with_capacity(vertex_count);
    let mut indices = Vec::with_capacity(sphere.corners.len() * 3);

    for (cell, color) in colors.into_iter().enumerate() {
        let center = displaced_position(
            to_bevy(sphere.cell_centers[cell]),
            cell_elevations[cell],
            relief_exaggeration,
        );
        let corners = sphere.cell_corners(cell);
        let base = u32::try_from(positions.len()).expect("surface mesh exceeds u32 indices");
        let linear_color = LinearRgba::from(color).to_f32_array();

        positions.push(center.to_array());
        normal_sources.push(cell);
        vertex_colors.push(linear_color);
        for corner in corners {
            let position = displaced_position(
                to_bevy(sphere.vertices[corner.vertex]),
                corner_elevations[corner.vertex],
                relief_exaggeration,
            );
            positions.push(position.to_array());
            normal_sources.push(sphere.cell_count() + corner.vertex);
            vertex_colors.push(linear_color);
        }

        for corner in 0..corners.len() {
            let current = base + 1 + corner as u32;
            let next = base + 1 + ((corner + 1) % corners.len()) as u32;
            let a = Vec3::from_array(positions[current as usize]);
            let b = Vec3::from_array(positions[next as usize]);
            if (a - center).cross(b - center).dot(center) >= 0.0 {
                indices.extend([base, current, next]);
            } else {
                indices.extend([base, next, current]);
            }
        }
    }

    let normals = shared_geometry_normals(
        &positions,
        &indices,
        &normal_sources,
        sphere.cell_count() + sphere.vertex_count(),
    );
    empty_surface_mesh()
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vertex_colors)
        .with_inserted_indices(Indices::U32(indices))
}

fn shared_geometry_normals(
    positions: &[[f32; 3]],
    indices: &[u32],
    normal_sources: &[usize],
    source_count: usize,
) -> Vec<[f32; 3]> {
    let mut source_normals = vec![Vec3::ZERO; source_count];
    for triangle in indices.chunks_exact(3) {
        let [a, b, c] = triangle else { unreachable!() };
        let a = Vec3::from_array(positions[*a as usize]);
        let b = Vec3::from_array(positions[*b as usize]);
        let c = Vec3::from_array(positions[*c as usize]);
        let face_normal = (b - a).cross(c - a);
        for &vertex in triangle {
            source_normals[normal_sources[vertex as usize]] += face_normal;
        }
    }

    positions
        .iter()
        .zip(normal_sources)
        .map(|(position, &source)| {
            let normal = source_normals[source];
            if normal.is_finite() && normal.length_squared() > 1.0e-20 {
                normal.normalize().to_array()
            } else {
                Vec3::from_array(*position).normalize().to_array()
            }
        })
        .collect()
}

fn displaced_position(direction: Vec3, elevation: f32, relief_exaggeration: f32) -> Vec3 {
    direction.normalize() * (SURFACE_RADIUS + (elevation - SEA_LEVEL) * relief_exaggeration)
}

pub(super) fn maximum_surface_radius(cell_elevations: &[f32], relief_exaggeration: f32) -> f32 {
    let maximum_elevation = cell_elevations.iter().copied().fold(SEA_LEVEL, f32::max);
    SURFACE_RADIUS + (maximum_elevation - SEA_LEVEL) * relief_exaggeration
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;
    use procgen_core::Vec3 as SphereVec3;
    use procgen_sphere_mesh::build_sphere_mesh;

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

    fn positions(mesh: &Mesh) -> &Vec<[f32; 3]> {
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("surface mesh must have float3 positions");
        };
        positions
    }

    fn normals(mesh: &Mesh) -> &Vec<[f32; 3]> {
        let Some(VertexAttributeValues::Float32x3(normals)) =
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            panic!("surface mesh must have float3 normals");
        };
        normals
    }

    #[test]
    fn zero_exaggeration_leaves_every_vertex_at_surface_radius() {
        let sphere = tetrahedron_mesh();
        let mesh = cell_surface_mesh(&sphere, [Color::WHITE; 4], &[0.0, 0.3, 0.7, 1.0], 0.0);

        assert!(positions(&mesh).iter().all(|position| {
            (Vec3::from_array(*position).length() - SURFACE_RADIUS).abs() <= f32::EPSILON
        }));
    }

    #[test]
    fn incident_fans_share_identical_displaced_corner_positions() {
        let sphere = tetrahedron_mesh();
        let mesh = cell_surface_mesh(&sphere, [Color::WHITE; 4], &[0.0, 0.3, 0.7, 1.0], 0.3);
        let positions = positions(&mesh);
        let mut copies = vec![Vec::new(); sphere.vertex_count()];
        let mut base = 0;
        for cell in 0..sphere.cell_count() {
            for (corner_offset, corner) in sphere.cell_corners(cell).iter().enumerate() {
                copies[corner.vertex].push(positions[base + 1 + corner_offset]);
            }
            base += 1 + sphere.cell_corners(cell).len();
        }

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
    fn displaced_triangle_fans_remain_finite_and_outward() {
        let sphere = tetrahedron_mesh();
        let mesh = cell_surface_mesh(&sphere, [Color::WHITE; 4], &[0.0, 0.3, 0.7, 1.0], 0.4);

        assert_eq!(
            mesh.count_vertices(),
            sphere.cell_count() + sphere.corners.len()
        );
        let Indices::U32(indices) = mesh.indices().unwrap() else {
            panic!("surface mesh must use u32 indices");
        };
        assert_eq!(indices.len(), sphere.corners.len() * 3);
        let positions = positions(&mesh);
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
        let elevations = [0.0, 0.3, 0.7, 1.0];
        let flat = cell_surface_mesh(&sphere, [Color::WHITE; 4], &elevations, 0.0);
        let relief = cell_surface_mesh(&sphere, [Color::WHITE; 4], &elevations, 0.4);
        let relief_positions = positions(&relief);
        let relief_normals = normals(&relief);

        assert_eq!(relief_normals.len(), relief_positions.len());
        for (&position, &normal) in relief_positions.iter().zip(relief_normals) {
            let position = Vec3::from_array(position);
            let normal = Vec3::from_array(normal);
            assert!(normal.is_finite());
            assert!((normal.length() - 1.0).abs() < 1.0e-5);
            assert!(normal.dot(position) > 0.0);
        }
        assert!(
            normals(&flat)
                .iter()
                .zip(relief_normals)
                .any(
                    |(flat, relief)| Vec3::from_array(*flat).distance(Vec3::from_array(*relief))
                        > 1.0e-4
                )
        );

        let mut shared_corner_normals = vec![Vec::new(); sphere.vertex_count()];
        let mut base = 0;
        for cell in 0..sphere.cell_count() {
            for (corner_offset, corner) in sphere.cell_corners(cell).iter().enumerate() {
                shared_corner_normals[corner.vertex].push(relief_normals[base + 1 + corner_offset]);
            }
            base += 1 + sphere.cell_corners(cell).len();
        }
        assert!(shared_corner_normals.into_iter().all(|copies| {
            copies.len() == 3 && copies[1..].iter().all(|copy| *copy == copies[0])
        }));
    }
}
