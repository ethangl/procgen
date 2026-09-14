//! Camera-independent detail shared by the corners and edges of a balanced cover.
use crate::HeightTile;
use procgen_cubesphere::{MAX_TILE_LEVEL, TILE_QUADS, TileAddress, vertex_spacing};
use std::collections::BTreeMap;

/// Retain distant bands despite the fixed tile budget and coarse corner constraints.
pub const HEIGHT_DETAIL_RATIO: f32 = 8.0;
pub const HEIGHT_FILTER_MIN_M: f32 = 0.25;
const _: () =
    assert!((1u64 << MAX_TILE_LEVEL) * (crate::HEIGHT_QUADS as u64).pow(2) <= u32::MAX as u64);
const CORNERS: [[u32; 2]; 4] = [[0, 0], [1, 0], [0, 1], [1, 1]];
const EDGES: [[usize; 2]; 4] = [[0, 2], [1, 3], [0, 1], [2, 3]];

/// Integer cube coordinates identify a corner across reversed cube-face seams.
fn corner(tile: TileAddress, [x, y]: [u32; 2]) -> [i32; 3] {
    let scale = 1i32 << MAX_TILE_LEVEL;
    let u = ((tile.x() + x) << (MAX_TILE_LEVEL - tile.level())) as i32 * 2 - scale;
    let v = ((tile.y() + y) << (MAX_TILE_LEVEL - tile.level())) as i32 * 2 - scale;
    let frame = tile.face().frame();
    let p = frame.normal * scale as f32 + frame.u_axis * u as f32 + frame.v_axis * v as f32;
    [p.x as i32, p.y as i32, p.z as i32]
}

pub(crate) fn share_detail(tiles: &mut [HeightTile]) {
    let mut spans = BTreeMap::<[i32; 3], u32>::new();
    for tile in tiles.iter() {
        let span = 1 << (MAX_TILE_LEVEL - tile.address().level());
        for c in CORNERS {
            spans
                .entry(corner(tile.address(), c))
                .and_modify(|v| *v = (*v).max(span))
                .or_insert(span);
        }
    }
    // A fine corner at a coarse edge midpoint must interpolate that edge's
    // endpoints. Resolve coarser constraints first, including cube corners.
    let mut order: Vec<_> = tiles.iter().map(|t| t.address()).collect();
    order.sort_by_key(|t| t.level());
    for tile in order {
        let points = CORNERS.map(|c| corner(tile, c));
        for [a, b] in EDGES {
            let midpoint = std::array::from_fn(|i| (points[a][i] + points[b][i]) / 2);
            if spans.contains_key(&midpoint) {
                let sum = spans[&points[a]] + spans[&points[b]];
                debug_assert_eq!(sum % 2, 0, "coarse edge spans have an exact midpoint");
                spans.insert(midpoint, sum / 2);
            }
        }
    }
    for tile in tiles {
        tile.corner_spans = CORNERS.map(|c| spans[&corner(tile.address(), c)]);
    }
}

impl HeightTile {
    /// Bilinear integer interpolation makes shared fine/coarse samples use
    /// exactly the same footprint, independent of face orientation and camera.
    /// Coordinates must be in 0..=HEIGHT_QUADS.
    pub fn footprint_m(self, [x, y]: [u32; 2], radius_m: f32) -> f32 {
        let q = crate::HEIGHT_QUADS;
        let [a, b, c, d] = self.corner_spans;
        let numerator = a * (q - x) * (q - y) + b * x * (q - y) + c * (q - x) * y + d * x * y;
        let unit =
            vertex_spacing(MAX_TILE_LEVEL) * radius_m * (TILE_QUADS / crate::HEIGHT_QUADS) as f32
                / HEIGHT_DETAIL_RATIO;
        (numerator as f32 * unit / (q * q) as f32).max(HEIGHT_FILTER_MIN_M)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        HEIGHT_QUADS, PlanetDesignConfig, VoxelPosition, height_tile_vertices,
        select_height_coverage,
    };
    use procgen_core::Vec3;

    fn eye(field: &crate::PlanetDesignField, d: Vec3, clearance: f32) -> VoxelPosition {
        let p = d * (field.config().radius_m + field.height(d, 0.0) + clearance);
        VoxelPosition {
            x_m: p.x.round() as i32,
            y_m: p.y.round() as i32,
            z_m: p.z.round() as i32,
        }
    }
    fn key(tile: HeightTile, [x, y]: [u32; 2]) -> [i32; 3] {
        let a = tile.address();
        let scale = (HEIGHT_QUADS << MAX_TILE_LEVEL) as i32;
        let u = ((a.x() * HEIGHT_QUADS + x) << (MAX_TILE_LEVEL - a.level())) as i32 * 2 - scale;
        let v = ((a.y() * HEIGHT_QUADS + y) << (MAX_TILE_LEVEL - a.level())) as i32 * 2 - scale;
        let f = a.face().frame();
        let p = f.normal * scale as f32 + f.u_axis * u as f32 + f.v_axis * v as f32;
        [p.x as i32, p.y as i32, p.z as i32]
    }
    #[test]
    fn selected_covers_share_exact_detail_at_all_edges_and_corners() {
        let mut config = PlanetDesignConfig::starter(42);
        for radius in [100_000.0, 300_000.0, 8_000_000.0] {
            config.radius_m = radius;
            let field = config.validate().unwrap();
            for d in [
                Vec3::X,
                Vec3::new(1., 1., 0.).normalized(),
                Vec3::new(1., 1., 1.).normalized(),
                Vec3::new(1., 0.7, -0.3).normalized(),
            ] {
                for clearance in [5., radius * 2.] {
                    let tiles = select_height_coverage(&field, eye(&field, d, clearance));
                    let mut samples = BTreeMap::new();
                    let mut shared = 0;
                    for tile in tiles {
                        for i in 0..=HEIGHT_QUADS {
                            for [x, y] in [[0, i], [HEIGHT_QUADS, i], [i, 0], [i, HEIGHT_QUADS]] {
                                let xy = tile.grid_coordinates(y * crate::HEIGHT_SIDE + x);
                                let footprint = tile.footprint_m(xy, radius);
                                if let Some(previous) = samples.insert(key(tile, xy), footprint) {
                                    assert_eq!(
                                        previous, footprint,
                                        "shared sample at {radius} m, {d:?}"
                                    );
                                    shared += 1;
                                }
                            }
                        }
                    }
                    assert!(shared > 10_000, "actual complete coverage exercised");
                }
            }
        }
    }
    #[test]
    fn small_movement_reuses_mesh_identity_and_revisit_recreates_it() {
        let mut config = PlanetDesignConfig::starter(42);
        config.radius_m = 300_000.;
        let field = config.validate().unwrap();
        let a = eye(&field, Vec3::new(1., 0.7, 0.3).normalized(), 5.);
        let b = VoxelPosition {
            x_m: a.x_m,
            y_m: a.y_m + 8,
            z_m: a.z_m,
        };
        let before = select_height_coverage(&field, a);
        let after = select_height_coverage(&field, b);
        let reused: Vec<_> = after.iter().filter(|t| before.contains(t)).collect();
        assert!(
            reused.len() > before.len() / 2,
            "small travel must retain most mesh identities"
        );
        assert_eq!(
            before,
            select_height_coverage(&field, a),
            "revisit is independent of history"
        );
        let tile = **reused.first().unwrap();
        let initial = height_tile_vertices(&field, tile);
        let rebuilt = height_tile_vertices(&field, tile);
        assert_eq!(initial, rebuilt, "eviction changes no mesh data");
        let mut edited = config.clone();
        edited.seed += 1;
        let changed = height_tile_vertices(&edited.validate().unwrap(), tile);
        assert!(
            initial
                .iter()
                .zip(changed)
                .any(|(a, b)| a.normal[3] != b.normal[3]),
            "design revision must invalidate geometry even with the same tile identity"
        );
    }
    #[test]
    fn changed_neighbors_invalidate_the_shared_filter_identity() {
        let field = PlanetDesignConfig::starter(42).validate().unwrap();
        let a = eye(&field, Vec3::X, 5.);
        let b = VoxelPosition { y_m: 256, ..a };
        let before = select_height_coverage(&field, a);
        let after = select_height_coverage(&field, b);
        assert!(
            before.iter().any(|a| after
                .iter()
                .any(|b| a.address() == b.address() && a.corner_spans != b.corner_spans)),
            "same address can require a new mesh after neighboring LOD changes"
        );
    }
}
