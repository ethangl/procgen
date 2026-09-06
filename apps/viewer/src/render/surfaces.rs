use super::palette::{
    INSOLATION_COLOR_STOPS, SEAFLOOR_AGE_COLOR_STOPS, id_color, opaque_color, piecewise_lerp,
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
use procgen_tectonics::CrustClass;

const SURFACE_RADIUS: f32 = 1.0;

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
) -> Mesh {
    cell_surface_mesh(
        &world.voronoi,
        values
            .iter()
            .map(|&value| opaque_color(piecewise_lerp(value, stops))),
    )
}

pub(super) fn plate_surface_mesh(world: &GeneratedWorld) -> Mesh {
    cell_surface_mesh(
        &world.voronoi,
        world
            .plates
            .cell_plates
            .iter()
            .map(|&plate| id_color(plate)),
    )
}

pub(super) fn crust_surface_mesh(world: &GeneratedWorld) -> Mesh {
    cell_surface_mesh(
        &world.voronoi,
        (0..world.voronoi.cell_count()).map(|cell| {
            match world.crust.cell_class(&world.plates, cell) {
                CrustClass::Oceanic => Color::srgb(0.12, 0.48, 0.95),
                CrustClass::Continental => Color::srgb(0.92, 0.62, 0.2),
            }
        }),
    )
}

pub(super) fn seafloor_age_surface_mesh(world: &GeneratedWorld) -> Mesh {
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
    )
}

pub(super) fn insolation_surface_mesh(world: &GeneratedWorld) -> Mesh {
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
    )
}

pub(super) fn basin_surface_mesh(world: &GeneratedWorld) -> Mesh {
    cell_surface_mesh(
        &world.voronoi,
        world
            .basins
            .cell_basins
            .iter()
            .map(|basin| basin.map_or(Color::srgb(0.045, 0.065, 0.075), id_color)),
    )
}

fn cell_surface_mesh(sphere: &SphereMesh, colors: impl IntoIterator<Item = Color>) -> Mesh {
    let colors = colors.into_iter().collect::<Vec<_>>();
    assert_eq!(colors.len(), sphere.cell_count());

    let vertex_count = sphere.corners.len() + sphere.cell_count();
    let mut positions = Vec::with_capacity(vertex_count);
    let mut normals = Vec::with_capacity(vertex_count);
    let mut vertex_colors = Vec::with_capacity(vertex_count);
    let mut indices = Vec::with_capacity(sphere.corners.len() * 3);

    for (cell, color) in colors.into_iter().enumerate() {
        let center = to_bevy(sphere.cell_centers[cell]).normalize() * SURFACE_RADIUS;
        let corners = sphere.cell_corners(cell);
        let base = u32::try_from(positions.len()).expect("surface mesh exceeds u32 indices");
        let linear_color = LinearRgba::from(color).to_f32_array();

        positions.push(center.to_array());
        normals.push(center.normalize().to_array());
        vertex_colors.push(linear_color);
        for corner in corners {
            let position = to_bevy(sphere.vertices[corner.vertex]).normalize() * SURFACE_RADIUS;
            positions.push(position.to_array());
            normals.push(position.normalize().to_array());
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

    empty_surface_mesh()
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vertex_colors)
        .with_inserted_indices(Indices::U32(indices))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;
    use procgen_core::Vec3 as SphereVec3;
    use procgen_sphere_mesh::build_sphere_mesh;

    #[test]
    fn cell_surface_mesh_builds_an_outward_triangle_fan_per_cell() {
        let sphere = build_sphere_mesh(
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
        .unwrap();
        let mesh = cell_surface_mesh(&sphere, [Color::WHITE; 4]);

        assert_eq!(
            mesh.count_vertices(),
            sphere.cell_count() + sphere.corners.len()
        );
        let Indices::U32(indices) = mesh.indices().unwrap() else {
            panic!("surface mesh must use u32 indices");
        };
        assert_eq!(indices.len(), sphere.corners.len() * 3);
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("surface mesh must have float3 positions");
        };
        for triangle in indices.chunks_exact(3) {
            let [a, b, c] = triangle else { unreachable!() };
            let a = Vec3::from_array(positions[*a as usize]);
            let b = Vec3::from_array(positions[*b as usize]);
            let c = Vec3::from_array(positions[*c as usize]);
            assert!((b - a).cross(c - a).dot(a + b + c) > 0.0);
        }
    }
}
