//! Bounded distant height coverage and the independent meter-scale voxel region.
use crate::{
    HeightTile, PlanetDesignField, VoxelChunkAddress, VoxelCoverage, VoxelCoverageError,
    VoxelPosition,
};
use procgen_core::Vec3;
use procgen_cubesphere::{
    CubeFace, FaceEdge, MAX_TILE_LEVEL, TILE_QUADS, TileAddress, vertex_spacing,
};
use std::collections::BTreeSet;

pub const MAX_HEIGHT_TILES: usize = 384;
pub const LOCAL_VOXEL_RADIUS_CHUNKS: i32 = 2;
pub const LOCAL_VOXEL_ACTIVATION_M: f32 = 256.0;

/// Complete six-face coverage. Only address metadata is generated on the CPU.
/// Distance uses the surface projection, so local relief cannot force every tile
/// in the planet's full height envelope to the finest level. Every admitted
/// refinement retains 2:1 edge balance, including across cube faces.
pub fn select_height_coverage(field: &PlanetDesignField, eye: VoxelPosition) -> Vec<HeightTile> {
    let radius = field.config().radius_m;
    let p = Vec3::new(eye.x_m as f32, eye.y_m as f32, eye.z_m as f32);
    let direction = if p.length_squared() == 0.0 {
        Vec3::X
    } else {
        p.normalized()
    };
    let clearance = p.length() - radius - field.height(direction, 0.0);
    let surface = direction * radius;
    let mut leaves: BTreeSet<_> = CubeFace::ALL.into_iter().map(TileAddress::root).collect();
    loop {
        let mut candidates: Vec<_> = leaves
            .iter()
            .filter_map(|&tile| {
                if tile.level() == MAX_TILE_LEVEL
                    || vertex_spacing(tile.level())
                        * radius
                        * (TILE_QUADS / crate::HEIGHT_QUADS) as f32
                        <= 1.0
                {
                    return None;
                }
                let center = tile
                    .grid_vertex(TILE_QUADS / 2, TILE_QUADS / 2)
                    .unwrap()
                    .direction()
                    * radius;
                let width = vertex_spacing(tile.level()) * radius * TILE_QUADS as f32;
                let lateral = ((center - surface).length() - width).max(0.0);
                let distance = lateral.hypot(clearance.max(0.0));
                // A tile should span a small view angle in orbit too.
                // Camera priority and the fixed tile cap still bound refinement.
                (distance < 8.0 * width).then_some((distance / width, tile))
            })
            .collect();
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let refinement = candidates.into_iter().find_map(|(_, tile)| {
            let mut splits = BTreeSet::new();
            refinement_closure(tile, &leaves, &mut splits);
            (leaves.len() + 3 * splits.len() <= MAX_HEIGHT_TILES).then_some(splits)
        });
        let Some(splits) = refinement else { break };
        for tile in splits {
            leaves.remove(&tile);
            leaves.extend(tile.children().unwrap());
        }
    }
    let mut tiles: Vec<_> = leaves
        .iter()
        .map(|&tile| {
            HeightTile::new(
                tile,
                FaceEdge::ALL.into_iter().filter(|&edge| {
                    covering_leaf(tile.edge_neighbor(edge), &leaves)
                        .is_some_and(|neighbor| neighbor.level() < tile.level())
                }),
            )
        })
        .collect();
    crate::height_detail::share_detail(&mut tiles);
    tiles
}

// Start with six roots and admit each split together with its required neighbor
// splits. The tile cap therefore cannot leave an unbalanced intermediate cover.
fn refinement_closure(
    tile: TileAddress,
    leaves: &BTreeSet<TileAddress>,
    splits: &mut BTreeSet<TileAddress>,
) {
    if !splits.insert(tile) {
        return;
    }
    for edge in FaceEdge::ALL {
        if let Some(neighbor) = covering_leaf(tile.edge_neighbor(edge), leaves)
            && neighbor.level() < tile.level()
        {
            refinement_closure(neighbor, leaves, splits);
        }
    }
}
fn covering_leaf(tile: TileAddress, leaves: &BTreeSet<TileAddress>) -> Option<TileAddress> {
    std::iter::successors(Some(tile), |t| t.parent()).find(|t| leaves.contains(t))
}

/// A five-by-five-by-five cube of unchanged one-meter chunks around the ground
/// below the camera. Height coverage remains available throughout its admission.
pub fn local_voxel_coverage(
    field: &PlanetDesignField,
    eye: VoxelPosition,
) -> Result<VoxelCoverage, VoxelCoverageError> {
    let p = Vec3::new(eye.x_m as f32, eye.y_m as f32, eye.z_m as f32);
    let direction = if p.length_squared() == 0.0 {
        Vec3::X
    } else {
        p.normalized()
    };
    let height = field.height(direction, 0.0);
    if p.length() - field.config().radius_m - height > LOCAL_VOXEL_ACTIVATION_M {
        return VoxelCoverage::new(Vec::new());
    }
    let ground = direction * (field.config().radius_m + height);
    let center = VoxelPosition {
        x_m: ground.x as i32,
        y_m: ground.y as i32,
        z_m: ground.z as i32,
    };
    let mut leaves = Vec::new();
    for z in -LOCAL_VOXEL_RADIUS_CHUNKS..=LOCAL_VOXEL_RADIUS_CHUNKS {
        for y in -LOCAL_VOXEL_RADIUS_CHUNKS..=LOCAL_VOXEL_RADIUS_CHUNKS {
            for x in -LOCAL_VOXEL_RADIUS_CHUNKS..=LOCAL_VOXEL_RADIUS_CHUNKS {
                leaves.push(
                    VoxelChunkAddress::containing(
                        VoxelPosition {
                            x_m: center.x_m + x * crate::VOXEL_CHUNK_CELLS,
                            y_m: center.y_m + y * crate::VOXEL_CHUNK_CELLS,
                            z_m: center.z_m + z * crate::VOXEL_CHUNK_CELLS,
                        },
                        0,
                    )
                    .expect("validated planet fits voxel root"),
                );
            }
        }
    }
    VoxelCoverage::new(leaves)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_bounded_height_and_local_meter_coverage_on_both_presets() {
        for source in [
            include_str!("../../../planet-design.json"),
            include_str!("../../../planet-design-300km.json"),
        ] {
            let json: serde_json::Value = serde_json::from_str(source).unwrap();
            let config: crate::PlanetDesignConfig =
                serde_json::from_value(json["design"].clone()).unwrap();
            let field = config.validate().unwrap();
            for direction in [Vec3::X, Vec3::new(1.0, 1.0, 1.0).normalized(), -Vec3::Z] {
                let ground = direction * (config.radius_m + field.height(direction, 0.0) + 2.0);
                let eye = VoxelPosition {
                    x_m: ground.x as i32,
                    y_m: ground.y as i32,
                    z_m: ground.z as i32,
                };
                let tiles = select_height_coverage(&field, eye);
                let addresses: BTreeSet<_> = tiles.iter().map(|t| t.address()).collect();
                for &tile in &addresses {
                    for edge in FaceEdge::ALL {
                        if let Some(neighbor) = covering_leaf(tile.edge_neighbor(edge), &addresses)
                        {
                            assert!(
                                tile.level() - neighbor.level() <= 1,
                                "balanced cube seams too"
                            );
                        }
                    }
                }
                assert!(tiles.len() <= MAX_HEIGHT_TILES);
                for face in CubeFace::ALL {
                    let area: f64 = tiles
                        .iter()
                        .filter(|t| t.address().face() == face)
                        .map(|t| 4.0_f64.powi(-(t.address().level() as i32)))
                        .sum();
                    assert_eq!(area, 1.0);
                }
                assert_eq!(tiles, select_height_coverage(&field, eye));
                assert!(tiles.iter().any(|t| vertex_spacing(t.address().level())
                    * config.radius_m
                    * (TILE_QUADS / crate::HEIGHT_QUADS) as f32
                    <= 1.0));
                let local = local_voxel_coverage(&field, eye).unwrap();
                assert_eq!(local.leaves().len(), 125);
                assert!(local.leaves().iter().all(|t| t.spacing_m() == 1));
            }
        }
    }
}
