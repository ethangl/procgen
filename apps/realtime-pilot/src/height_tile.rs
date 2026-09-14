//! Mesh identity includes the edges shared with a coarser neighbor.
use procgen_cubesphere::{FaceEdge, TILE_QUADS, TileAddress};

// Use every canonical grid sample so distant geometry retains smaller features.
pub const HEIGHT_QUADS: u32 = TILE_QUADS;
pub const HEIGHT_SIDE: u32 = HEIGHT_QUADS + 1;
pub const HEIGHT_VERTEX_COUNT: u32 = HEIGHT_SIDE * HEIGHT_SIDE;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HeightTile {
    address: TileAddress,
    coarse_edges: u32,
}
impl HeightTile {
    /// Neighbors must differ by at most one level. Mark only edges whose
    /// neighbor is one level coarser; coverage selection maintains this rule.
    pub fn new(address: TileAddress, coarse_edges: impl IntoIterator<Item = FaceEdge>) -> Self {
        let coarse_edges = coarse_edges
            .into_iter()
            .fold(0, |bits, edge| bits | edge_bit(edge));
        assert!(
            address.level() > 0 || coarse_edges == 0,
            "root has no coarser neighbor"
        );
        Self {
            address,
            coarse_edges,
        }
    }
    pub fn address(self) -> TileAddress {
        self.address
    }

    #[cfg(feature = "gpu")]
    pub(crate) fn gpu_words(self) -> [u32; 8] {
        let [face, level, x, y] = self.address.gpu_words();
        [face, level, x, y, self.coarse_edges, 0, 0, 0]
    }

    pub(crate) fn grid_coordinates(self, vertex: u32) -> [u32; 2] {
        let mut x = vertex % HEIGHT_SIDE;
        let mut y = vertex / HEIGHT_SIDE;
        // Even samples coincide exactly with the neighbor's grid. Collapsing
        // odd samples also shares its straight triangle edge, without a skirt.
        if (x == 0 && self.coarse_edges & edge_bit(FaceEdge::Left) != 0)
            || (x == HEIGHT_QUADS && self.coarse_edges & edge_bit(FaceEdge::Right) != 0)
        {
            y &= !1;
        }
        if (y == 0 && self.coarse_edges & edge_bit(FaceEdge::Bottom) != 0)
            || (y == HEIGHT_QUADS && self.coarse_edges & edge_bit(FaceEdge::Top) != 0)
        {
            x &= !1;
        }
        [x, y]
    }
}
pub(crate) const fn edge_bit(edge: FaceEdge) -> u32 {
    match edge {
        FaceEdge::Left => 1,
        FaceEdge::Right => 2,
        FaceEdge::Bottom => 4,
        FaceEdge::Top => 8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PlanetDesignConfig, VoxelPosition, height_indices, select_height_coverage};
    use procgen_core::Vec3;
    use procgen_cubesphere::{CubeFace, MAX_TILE_LEVEL};
    use std::collections::HashMap;

    #[test]
    fn every_edge_combination_preserves_tile_area_and_triangle_orientation() {
        let address = TileAddress::new(CubeFace::PositiveX, 4, 7, 9).unwrap();
        for mask in 0..16 {
            let tile = HeightTile::new(
                address,
                FaceEdge::ALL
                    .into_iter()
                    .filter(|&e| mask & edge_bit(e) != 0),
            );
            let mut area = 0;
            for triangle in height_indices().chunks_exact(3) {
                let p: Vec<_> = triangle
                    .iter()
                    .map(|&v| tile.grid_coordinates(v).map(i64::from))
                    .collect();
                let signed = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1])
                    - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
                assert!(signed >= 0, "mask {mask} must not fold a triangle");
                area += signed;
            }
            assert_eq!(
                area,
                2 * i64::from(HEIGHT_QUADS).pow(2),
                "mask {mask} must cover the whole tile"
            );
        }
    }

    #[test]
    fn selected_mesh_is_closed_across_lods_cube_edges_and_corners() {
        let config = PlanetDesignConfig::starter(42);
        let field = config.validate().unwrap();
        for direction in [
            Vec3::X,
            Vec3::new(1.0, 1.0, 0.0).normalized(),
            Vec3::new(1.0, 1.0, 1.0).normalized(),
        ] {
            for clearance in [2.0, config.radius_m * 0.5] {
                let p = direction * (config.radius_m + field.height(direction, 0.0) + clearance);
                let tiles = select_height_coverage(
                    &field,
                    VoxelPosition {
                        x_m: p.x as i32,
                        y_m: p.y as i32,
                        z_m: p.z as i32,
                    },
                );
                // Integer cube coordinates identify the canonical source sample
                // before sphere mapping or noise, including reversed face seams.
                let mut edges = HashMap::new();
                for tile in tiles {
                    let frame = tile.address.face().frame();
                    let xyz = |v: Vec3| [v.x as i32, v.y as i32, v.z as i32];
                    let normal = xyz(frame.normal);
                    let u_axis = xyz(frame.u_axis);
                    let v_axis = xyz(frame.v_axis);
                    let scale = (HEIGHT_QUADS << MAX_TILE_LEVEL) as i32;
                    let points: Vec<[i32; 3]> = (0..HEIGHT_VERTEX_COUNT)
                        .map(|v| {
                            let [x, y] = tile.grid_coordinates(v);
                            let u = (2
                                * ((tile.address.x() * HEIGHT_QUADS + x)
                                    << (MAX_TILE_LEVEL - tile.address.level())))
                                as i32
                                - scale;
                            let v = (2
                                * ((tile.address.y() * HEIGHT_QUADS + y)
                                    << (MAX_TILE_LEVEL - tile.address.level())))
                                as i32
                                - scale;
                            std::array::from_fn(|i| {
                                normal[i] * scale + u_axis[i] * u + v_axis[i] * v
                            })
                        })
                        .collect();
                    for triangle in height_indices().chunks_exact(3) {
                        let [a, b, c] = [
                            points[triangle[0] as usize],
                            points[triangle[1] as usize],
                            points[triangle[2] as usize],
                        ];
                        if a == b || b == c || a == c {
                            continue;
                        }
                        for (a, b) in [(a, b), (b, c), (c, a)] {
                            let (key, sign) = if a < b { ((a, b), 1) } else { ((b, a), -1) };
                            let entry = edges.entry(key).or_insert((0, 0));
                            entry.0 += 1;
                            entry.1 += sign;
                        }
                    }
                }
                assert!(
                    edges.values().all(|&v| v == (2, 0)),
                    "each edge must join two oppositely wound triangles: {:?}",
                    edges.iter().find(|(_, v)| **v != (2, 0))
                );
            }
        }
    }
}
