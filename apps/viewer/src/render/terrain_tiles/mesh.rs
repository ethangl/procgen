use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::{Mesh, Vec3},
};
use procgen_cubesphere::{TILE_QUADS, TILE_VERTICES, TileAddress};

use super::{SURFACE_RADIUS, coverage::tile_direction};

pub(super) fn tile_grid_mesh() -> Mesh {
    // Integer grid coordinates are decoded by the custom vertex shader; normals and colors are
    // present only to select Bevy's lit, vertex-color mesh pipeline layout.
    let mut positions = (0..TILE_VERTICES)
        .flat_map(|y| (0..TILE_VERTICES).map(move |x| [x as f32, y as f32, 0.0]))
        .collect::<Vec<_>>();
    let mut indices = Vec::with_capacity(((TILE_QUADS * TILE_QUADS + 4 * TILE_QUADS) * 6) as usize);
    for y in 0..TILE_QUADS {
        for x in 0..TILE_QUADS {
            let lower_left = y * TILE_VERTICES + x;
            let lower_right = lower_left + 1;
            let upper_left = lower_left + TILE_VERTICES;
            let upper_right = upper_left + 1;
            indices.extend([
                lower_left,
                lower_right,
                upper_right,
                lower_left,
                upper_right,
                upper_left,
            ]);
        }
    }
    append_skirt_edge(
        &mut positions,
        &mut indices,
        (0..TILE_VERTICES).map(|x| (x, 0)),
        false,
    );
    append_skirt_edge(
        &mut positions,
        &mut indices,
        (0..TILE_VERTICES).map(|y| (TILE_QUADS, y)),
        false,
    );
    append_skirt_edge(
        &mut positions,
        &mut indices,
        (0..TILE_VERTICES).map(|x| (x, TILE_QUADS)),
        true,
    );
    append_skirt_edge(
        &mut positions,
        &mut indices,
        (0..TILE_VERTICES).map(|y| (0, y)),
        true,
    );
    let normals = vec![[0.0, 0.0, 1.0]; positions.len()];
    let colors = vec![[1.0, 1.0, 1.0, 1.0]; positions.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}

fn append_skirt_edge(
    positions: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u32>,
    edge: impl Iterator<Item = (u32, u32)>,
    reverse_winding: bool,
) {
    let edge = edge.collect::<Vec<_>>();
    let skirt_start = positions.len() as u32;
    positions.extend(edge.iter().map(|&(x, y)| [x as f32, y as f32, 1.0]));
    for (offset, pair) in edge.windows(2).enumerate() {
        let core = [
            pair[0].1 * TILE_VERTICES + pair[0].0,
            pair[1].1 * TILE_VERTICES + pair[1].0,
        ];
        let skirt = [skirt_start + offset as u32, skirt_start + offset as u32 + 1];
        if reverse_winding {
            indices.extend([core[0], core[1], skirt[1], core[0], skirt[1], skirt[0]]);
        } else {
            indices.extend([core[0], skirt[1], core[1], core[0], skirt[0], skirt[1]]);
        }
    }
}

pub(super) fn tile_origin(address: TileAddress) -> Vec3 {
    tile_direction(address, TILE_QUADS / 2, TILE_QUADS / 2) * SURFACE_RADIUS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::terrain_tiles::TERRAIN_SKIRT_DEPTH_SPACINGS;
    use bevy::mesh::VertexAttributeValues;
    use procgen_cubesphere::{CubeFace, vertex_spacing};

    #[test]
    fn tile_grid_triangles_face_outward_on_every_cube_face() {
        let mesh = tile_grid_mesh();
        let VertexAttributeValues::Float32x3(positions) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
        else {
            panic!("terrain tile positions must be float3");
        };
        let Indices::U32(indices) = mesh.indices().unwrap() else {
            panic!("terrain tile indices must be u32");
        };

        for face in CubeFace::ALL {
            let address = TileAddress::root(face);
            for triangle in indices[..(TILE_QUADS * TILE_QUADS * 6) as usize].chunks_exact(3) {
                let directions = [triangle[0], triangle[1], triangle[2]].map(|index| {
                    let [x, y, _] = positions[index as usize];
                    tile_direction(address, x as u32, y as u32)
                });
                let geometric_normal =
                    (directions[1] - directions[0]).cross(directions[2] - directions[0]);
                let center = (directions[0] + directions[1] + directions[2]).normalize();
                assert!(geometric_normal.dot(center) > 0.0);
            }
        }
    }

    #[test]
    fn skirt_is_one_quad_deep_uses_edge_samples_and_winds_outward() {
        let mesh = tile_grid_mesh();
        let VertexAttributeValues::Float32x3(positions) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
        else {
            panic!("terrain tile positions must be float3");
        };
        let Indices::U32(indices) = mesh.indices().unwrap() else {
            panic!("terrain tile indices must be u32");
        };
        assert_eq!(
            positions.len(),
            (TILE_VERTICES * TILE_VERTICES + 4 * TILE_VERTICES) as usize
        );
        assert_eq!(
            indices.len(),
            ((TILE_QUADS * TILE_QUADS + 4 * TILE_QUADS) * 6) as usize
        );
        let core_indices = (TILE_QUADS * TILE_QUADS * 6) as usize;
        let address = TileAddress::root(CubeFace::PositiveZ);
        let center = tile_origin(address);
        let skirt_depth = TERRAIN_SKIRT_DEPTH_SPACINGS * vertex_spacing(address.level());
        for triangle in indices[core_indices..].chunks_exact(3) {
            let world = [triangle[0], triangle[1], triangle[2]].map(|index| {
                let [x, y, skirt] = positions[index as usize];
                tile_direction(address, x as u32, y as u32) * (1.0 - skirt * skirt_depth)
            });
            let normal = (world[1] - world[0]).cross(world[2] - world[0]);
            let edge_direction = (world[0] + world[1] + world[2]).normalize();
            let away_from_center = edge_direction - center;
            assert!(normal.dot(away_from_center) > 0.0);
            let skirt_vertices = triangle
                .iter()
                .filter(|&&index| positions[index as usize][2] == 1.0)
                .count();
            assert!((1..=2).contains(&skirt_vertices));
        }
    }

    #[test]
    fn level_twelve_origins_keep_vertex_positions_local_and_reconstructable() {
        let address = TileAddress::new(CubeFace::NegativeY, 12, 1_913, 2_077).unwrap();
        let origin = tile_origin(address);
        for (x, y) in [(0, 0), (32, 32), (64, 64)] {
            let world = tile_direction(address, x, y) * SURFACE_RADIUS;
            let relative = world - origin;
            assert!(relative.length() <= 48.0 * vertex_spacing(address.level()));
            assert!(relative.length() < world.length() * 0.001);
            assert!((origin + relative).distance(world) <= f32::EPSILON);
        }
    }
}
