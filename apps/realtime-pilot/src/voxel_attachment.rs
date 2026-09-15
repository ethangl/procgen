//! Floating-fragment audit for the volumetric detail term. CPU only, no device.
//!
//! The height fade bounds the envelope the term may occupy, but it does not
//! prove that every solid piece inside that envelope is attached to the ground.
//! This module answers that question directly for a small block of the field.
use crate::{
    ChunkIndex, PlanetDesignField, VOXEL_CHUNK_CELLS, VoxelChunkAddress, VoxelSampleIndex,
    sample_voxel_chunk,
};

/// The audit covers a cube of this many one-meter chunks on a side, centered on
/// the requested chunk. Three is the smallest block with an interior.
pub const ATTACHMENT_BLOCK_CHUNKS: i32 = 3;
/// Shared chunk endpoints are counted once, so the sample grid is one wider
/// than the block's cells.
const SIDE: i32 = ATTACHMENT_BLOCK_CHUNKS * VOXEL_CHUNK_CELLS + 1;
const SAMPLES: usize = (SIDE * SIDE * SIDE) as usize;

/// Solid components the flood fill could not reach from the block's
/// planet-facing outer face. An empty `sizes` means every solid sample in the
/// block is connected to the ground through the block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetachedReport {
    pub solid: usize,
    pub attached: usize,
    /// One entry per unreached component, in sample counts, largest first.
    pub sizes: Vec<usize>,
}
impl DetachedReport {
    pub fn count(&self) -> usize {
        self.sizes.len()
    }
    pub fn detached(&self) -> usize {
        self.sizes.iter().sum()
    }
    pub fn largest(&self) -> usize {
        self.sizes.first().copied().unwrap_or(0)
    }
}

/// Sample the 3x3x3 one-meter chunks around `center`, treat potential above
/// zero as solid, flood-fill six-connected from every solid sample on the
/// block's planet-facing outer face, and report the solid components that were
/// never reached.
///
/// The block is an open window on a much larger field, so a component reported
/// here may still be attached through rock outside the window; this is a lower
/// bound on attachment, and a nonzero count is a reason to look, not a proof of
/// a floating island. `center` must be a level-zero chunk with a full ring of
/// in-range neighbors, which every surface chunk of a supported radius has.
pub fn audit_detached_solids(
    field: &PlanetDesignField,
    center: VoxelChunkAddress,
) -> DetachedReport {
    assert_eq!(center.lod(), 0, "the attachment audit samples 1 m chunks");
    let mut solid = vec![false; SAMPLES];
    let base = center.index();
    for block in 0..ATTACHMENT_BLOCK_CHUNKS.pow(3) {
        let offset = |axis: i32| block / ATTACHMENT_BLOCK_CHUNKS.pow(axis as u32) % 3;
        let (bx, by, bz) = (offset(0), offset(1), offset(2));
        let step = |index: u32, block: i32| {
            index
                .checked_add_signed(block - 1)
                .expect("audited chunk needs a full ring of neighbors")
        };
        let address = VoxelChunkAddress::new(
            0,
            ChunkIndex {
                x: step(base.x, bx),
                y: step(base.y, by),
                z: step(base.z, bz),
            },
        )
        .expect("audited chunk needs a full ring of neighbors");
        let volume = sample_voxel_chunk(field, address).expect("validated field sample");
        for z in 0..=VOXEL_CHUNK_CELLS {
            for y in 0..=VOXEL_CHUNK_CELLS {
                for x in 0..=VOXEL_CHUNK_CELLS {
                    let cell = [
                        bx * VOXEL_CHUNK_CELLS + x,
                        by * VOXEL_CHUNK_CELLS + y,
                        bz * VOXEL_CHUNK_CELLS + z,
                    ];
                    solid[linear(cell)] = volume.potential(VoxelSampleIndex { x, y, z }) > 0.0;
                }
            }
        }
    }

    // The ground lies inward, so the seeds are the face of the block nearest
    // the planet center: the low or high end of whichever axis dominates the
    // block's own position.
    let p = center.origin();
    let middle = [p.x_m, p.y_m, p.z_m].map(|v| v as i64 + (VOXEL_CHUNK_CELLS / 2) as i64);
    let axis = (0..3).max_by_key(|&a| middle[a].abs()).unwrap();
    let face = if middle[axis] > 0 { 0 } else { SIDE - 1 };

    let mut reached = vec![false; SAMPLES];
    let mut stack = Vec::new();
    for first in 0..SIDE {
        for second in 0..SIDE {
            let mut cell = [0; 3];
            cell[axis] = face;
            cell[(axis + 1) % 3] = first;
            cell[(axis + 2) % 3] = second;
            push(&solid, &mut reached, &mut stack, cell);
        }
    }
    let attached = fill(&solid, &mut reached, &mut stack);

    let mut visited = reached.clone();
    let mut sizes = Vec::new();
    for z in 0..SIDE {
        for y in 0..SIDE {
            for x in 0..SIDE {
                if !solid[linear([x, y, z])] || visited[linear([x, y, z])] {
                    continue;
                }
                push(&solid, &mut visited, &mut stack, [x, y, z]);
                sizes.push(fill(&solid, &mut visited, &mut stack));
            }
        }
    }
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    DetachedReport {
        solid: solid.iter().filter(|&&s| s).count(),
        attached,
        sizes,
    }
}

fn linear(cell: [i32; 3]) -> usize {
    ((cell[2] * SIDE + cell[1]) * SIDE + cell[0]) as usize
}
fn push(solid: &[bool], visited: &mut [bool], stack: &mut Vec<[i32; 3]>, cell: [i32; 3]) {
    let i = linear(cell);
    if solid[i] && !visited[i] {
        visited[i] = true;
        stack.push(cell);
    }
}
/// Drains the stack and returns how many samples this fill claimed.
fn fill(solid: &[bool], visited: &mut [bool], stack: &mut Vec<[i32; 3]>) -> usize {
    let mut count = 0;
    while let Some(cell) = stack.pop() {
        count += 1;
        for axis in 0..3 {
            for step in [-1, 1] {
                let mut next = cell;
                next[axis] += step;
                if (0..SIDE).contains(&next[axis]) {
                    push(solid, visited, stack, next);
                }
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PlanetDesignConfig, VoxelPosition};
    use procgen_core::Vec3;

    /// The level-zero chunk holding the surface point along `direction`.
    fn surface_chunk(field: &PlanetDesignField, direction: Vec3) -> VoxelChunkAddress {
        let direction = direction.normalized();
        let p = direction * (field.config().radius_m + field.elevation_m(direction, 0.0).unwrap());
        VoxelChunkAddress::containing(
            VoxelPosition {
                x_m: p.x.round() as i32,
                y_m: p.y.round() as i32,
                z_m: p.z.round() as i32,
            },
            0,
        )
        .unwrap()
    }
    fn directions() -> [Vec3; 3] {
        [
            Vec3::X,
            Vec3::new(0.2, -1.0, 0.35),
            Vec3::new(-0.6, 0.4, 1.0),
        ]
    }

    #[test]
    fn a_pure_height_field_leaves_nothing_detached() {
        // Zero amplitude reduces the potential to height - altitude, which is
        // monotonic inward, so every solid column reaches the inner face. The
        // enabled flag must make no difference at zero amplitude.
        for enabled in [false, true] {
            let mut config = PlanetDesignConfig::starter(42);
            config.volume.enabled = enabled;
            config.volume.amplitude_m = 0.0;
            let field = config.validate().unwrap();
            for direction in directions() {
                let report = audit_detached_solids(&field, surface_chunk(&field, direction));
                assert!(report.solid > 0 && report.attached > 0, "{report:?}");
                assert_eq!(report.sizes, Vec::<usize>::new(), "{report:?}");
            }
        }
    }

    #[test]
    fn the_starter_volume_values_stay_within_a_measured_detachment_bound() {
        let mut config = PlanetDesignConfig::starter(42);
        config.volume = crate::VolumeConfig {
            enabled: true,
            ..Default::default()
        };
        let field = config.validate().unwrap();
        for direction in directions() {
            let report = audit_detached_solids(&field, surface_chunk(&field, direction));
            println!(
                "starter volume, {direction:?}: {} solid, {} attached, {} detached components, largest {}",
                report.solid,
                report.attached,
                report.count(),
                report.largest()
            );
            // Measured: all three points report zero detached components, so
            // the bound is zero. Raise it only together with the numbers that
            // forced the change, and only after looking at the shapes.
            assert_eq!(report.sizes, Vec::<usize>::new(), "{report:?}");
        }
    }

    #[test]
    fn an_extreme_configuration_detaches_solid_the_audit_can_see() {
        let mut config = PlanetDesignConfig::starter(42);
        config.volume = crate::VolumeConfig {
            enabled: true,
            wavelength_m: 8.0,
            amplitude_m: 64.0,
            sharpness: 0.5,
            fade_m: 256.0,
        };
        let field = config.validate().unwrap();
        let report = audit_detached_solids(&field, surface_chunk(&field, Vec3::X));
        println!(
            "extreme volume: {} solid, {} attached, {} detached components, largest {}",
            report.solid,
            report.attached,
            report.count(),
            report.largest()
        );
        assert!(report.count() >= 1, "the audit must be able to fail");
    }
}
