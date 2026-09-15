//! Bounded distant height coverage and the camera-centered mixed-LOD voxel band.
use crate::{
    HeightTile, PlanetDesignField, VoxelCoverage, VoxelCoverageError, VoxelPosition,
    select_voxel_coverage,
};
use procgen_core::Vec3;
use procgen_cubesphere::{
    CubeFace, FaceEdge, MAX_TILE_LEVEL, TILE_QUADS, TileAddress, vertex_spacing,
};
use std::collections::BTreeSet;

pub const MAX_HEIGHT_TILES: usize = 384;
/// Coarsest voxel cell the band keeps. Beyond it the height tiles stand alone.
/// A starting value; route measurements are expected to retune it.
pub const VOXEL_BAND_MAX_SPACING_M: i32 = 16;
/// Leaves asked of the octree selection before the spacing cap is applied.
pub const VOXEL_BAND_REQUESTED_LEAVES: usize = 384;

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

/// A camera-centered band of mixed-LOD chunks: the balanced near-field
/// selection with every leaf coarser than `max_spacing_m` dropped.
///
/// There is no altitude gate: once the camera is far enough that no leaf within
/// two chunk spans is finer than the cap, the band empties.
pub fn select_voxel_band(
    field: &PlanetDesignField,
    camera: VoxelPosition,
    max_spacing_m: i32,
    requested_leaves: usize,
) -> Result<VoxelCoverage, VoxelCoverageError> {
    cap_voxel_coverage(
        &select_voxel_coverage(
            field,
            camera,
            requested_leaves,
            crate::MAX_VOXEL_COVERAGE_LEAVES,
        )?,
        max_spacing_m,
    )
}

/// The band a balanced partition draws: its leaves at or under the cap. A
/// subset of a balanced partition is still 2:1 balanced, so nothing is
/// rebalanced, and dropping only coarser leaves leaves every kept leaf's finer
/// neighbors, and so its mesh key, exactly as the whole partition had them.
///
/// Chunks at the band edge then have no coarser neighbor and mesh their outer
/// faces as ordinary cells; that edge dissolves into the height tiles rather
/// than stitching to them.
///
/// The worker steps whole partitions and caps each one, because `step_toward`
/// only splits and coarsens the volume a coverage already holds: stepping band
/// to band stalls as soon as the camera moves the band over ground the previous
/// band did not cover at all.
pub fn cap_voxel_coverage(
    coverage: &VoxelCoverage,
    max_spacing_m: i32,
) -> Result<VoxelCoverage, VoxelCoverageError> {
    VoxelCoverage::new(
        coverage
            .leaves()
            .iter()
            .copied()
            .filter(|leaf| leaf.spacing_m() <= max_spacing_m)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_bounded_height_coverage_on_both_presets() {
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
            }
        }
    }

    /// The 300 km starter design, straight above one point, at four clearances.
    /// "Under the camera" is the band leaf covering the ground point below it:
    /// the octree also refines around the camera itself while the camera is
    /// inside the density shell, so the band's own minimum stays at one meter
    /// well past the altitude at which it stops covering the ground.
    #[test]
    fn voxel_band_respects_its_cap_and_leaves_the_ground_as_clearance_grows() {
        let json: serde_json::Value =
            serde_json::from_str(include_str!("../../../planet-design-300km.json")).unwrap();
        let config: crate::PlanetDesignConfig =
            serde_json::from_value(json["design"].clone()).unwrap();
        let field = config.validate().unwrap();
        let direction = Vec3::new(1.0, 1.0, 1.0).normalized();
        let height = field.height(direction, 0.0);
        let surface = direction * (config.radius_m + height);
        let ground = VoxelPosition {
            x_m: surface.x as i32,
            y_m: surface.y as i32,
            z_m: surface.z as i32,
        };
        let mut under = Vec::new();
        for clearance_m in [5.0, 100.0, 1_000.0, 10_000.0] {
            let p = direction * (config.radius_m + height + clearance_m);
            let camera = VoxelPosition {
                x_m: p.x as i32,
                y_m: p.y as i32,
                z_m: p.z as i32,
            };
            let selected = band(&field, camera);
            selected.validate().unwrap();
            assert!(
                selected
                    .leaves()
                    .iter()
                    .all(|leaf| leaf.spacing_m() <= VOXEL_BAND_MAX_SPACING_M),
                "{clearance_m} m: a leaf is coarser than the cap"
            );
            assert!(
                selected.leaves().len() <= BAND_LEAF_BOUND,
                "{clearance_m} m: {} leaves",
                selected.leaves().len()
            );
            assert_eq!(
                selected.leaves(),
                band(&field, camera).leaves(),
                "{clearance_m} m: the same inputs must give the same leaves"
            );
            let covering: BTreeSet<_> = selected.leaves().iter().copied().collect();
            let finest = crate::VoxelChunkAddress::containing(ground, 0).unwrap();
            under.push(
                std::iter::successors(Some(finest), |a| a.parent())
                    .find(|a| covering.contains(a))
                    .map(|a| a.spacing_m()),
            );
            println!(
                "{clearance_m} m clearance: {} leaves, spacings {:?}, under the camera {:?}",
                selected.leaves().len(),
                selected
                    .leaves()
                    .iter()
                    .map(|leaf| leaf.spacing_m())
                    .collect::<BTreeSet<_>>(),
                under.last().unwrap()
            );
        }
        assert_eq!(under[0], Some(1), "meter cells directly under the camera");
        assert!(
            under[1] >= under[0] && under[2] > under[1],
            "spacing under the camera must not refine as clearance grows: {under:?}"
        );
        assert_eq!(under[3], None, "the band leaves the ground to the tiles");
    }

    /// Sampled over both presets, four directions, and clearances from five
    /// meters to fifty kilometers. `LOCAL_GPU_WORLD_CONFIG` reserves slots for
    /// this many chunks; a band beyond it is refused by voxel residency.
    const BAND_LEAF_BOUND: usize = 1_280;

    fn band(field: &PlanetDesignField, camera: VoxelPosition) -> VoxelCoverage {
        select_voxel_band(
            field,
            camera,
            VOXEL_BAND_MAX_SPACING_M,
            VOXEL_BAND_REQUESTED_LEAVES,
        )
        .unwrap()
    }

    /// The band is a few hundred chunks on an octree axis and about four times
    /// that on a diagonal, where fewer chunk faces line up with the surface.
    #[test]
    fn voxel_band_leaf_count_stays_inside_the_reserved_slots() {
        for source in [
            include_str!("../../../planet-design.json"),
            include_str!("../../../planet-design-300km.json"),
        ] {
            let json: serde_json::Value = serde_json::from_str(source).unwrap();
            let config: crate::PlanetDesignConfig =
                serde_json::from_value(json["design"].clone()).unwrap();
            let field = config.validate().unwrap();
            let mut peak = 0;
            for direction in [
                Vec3::X,
                Vec3::new(1.0, 1.0, 1.0).normalized(),
                -Vec3::Z,
                Vec3::new(0.3, -0.9, 0.2).normalized(),
            ] {
                let height = field.height(direction, 0.0);
                for clearance_m in [5.0, 100.0, 500.0, 1_000.0, 10_000.0, 20_000.0, 50_000.0] {
                    let p = direction * (config.radius_m + height + clearance_m);
                    let camera = VoxelPosition {
                        x_m: p.x as i32,
                        y_m: p.y as i32,
                        z_m: p.z as i32,
                    };
                    let leaves = band(&field, camera).leaves().len();
                    assert!(leaves <= BAND_LEAF_BOUND, "{direction:?} {clearance_m} m");
                    peak = peak.max(leaves);
                }
            }
            println!("radius {}: peak band {peak} leaves", config.radius_m);
        }
    }
}
