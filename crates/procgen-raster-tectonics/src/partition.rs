//! Data contracts for the raster plate partition.
//!
//! Everything here is backend-neutral: the configuration, the traversal costs,
//! and the budgets the seeding and growth kernels run inside. How their results
//! are packed is [`crate::field`]'s; dispatch is [`crate::pipeline`]'s.

use crate::device::{MAX_TECTONIC_RESOLUTION, RasterTectonicsError, validate_resolution};
use crate::field::{MAX_GROWTH_COST, MAX_PLATE_COUNT};
use procgen_core::{RandomStream, hash_u32, random_streams};
use procgen_cubesphere::{AXIS_LINK_LENGTH, DIAGONAL_LINK_LENGTH};
use procgen_tectonics::MAX_GROWTH_ROUGHNESS;
use std::f32::consts::FRAC_PI_2;

/// Traversal cost of one unit of link length before roughness is applied.
pub const BASE_GROWTH_COST: u32 = 100;

/// Face widths a chamfer shortest path can cross between any two cube cells.
///
/// Two cells are at most half a face, one whole face, and half a face apart, so
/// three widths bound the link count of a path that uses the fewest links. The
/// cost budget and the frontier pass budget both derive from it.
const MAX_SHORTEST_PATH_FACE_WIDTHS: u32 = 3;

const COSTLIEST_SHORTEST_PATH: u64 = (MAX_SHORTEST_PATH_FACE_WIDTHS * MAX_TECTONIC_RESOLUTION)
    as u64
    * DIAGONAL_LINK_LENGTH as u64
    * (BASE_GROWTH_COST + MAX_GROWTH_ROUGHNESS) as u64;

const _: () = assert!(
    COSTLIEST_SHORTEST_PATH < MAX_GROWTH_COST as u64,
    "the packed label must hold the costliest shortest path the pilot can produce"
);

/// Factor by which roughness may lengthen a shortest path past the shortest
/// path that uses the fewest links.
const GROWTH_PATH_ROUGHNESS_MARGIN: u32 = 2;

/// Frontier passes one relaxation runs before it stops.
///
/// Every pass settles at least the cells one more link along their shortest
/// path, so the link count of the longest shortest path bounds the passes a
/// relaxation needs, and [`MAX_SHORTEST_PATH_FACE_WIDTHS`] bounds that count in
/// texels. Passes past convergence find an empty frontier, dispatch no
/// workgroups, and cost only their two commands, so the relaxation stops
/// without a readback.
pub(crate) const fn growth_passes(resolution: u32) -> u32 {
    GROWTH_PATH_ROUGHNESS_MARGIN * MAX_SHORTEST_PATH_FACE_WIDTHS * resolution
}

/// Workgroups the farthest-point reduction always dispatches. Each one strides
/// over the whole raster and emits one candidate.
pub(crate) const SEED_REDUCTION_WORKGROUPS: u32 = 256;

/// Squared chord distance no pair of unit directions can reach, used as the
/// farthest-point field's initial value.
pub(crate) const SEED_DISTANCE_CEILING: f32 = 1.0e30;
/// Distance of the candidate a reduction emits when it found no eligible cell.
pub(crate) const NO_SEED_DISTANCE: f32 = -1.0;
/// Frontier pass index no cell has been queued for.
pub(crate) const NO_FRONTIER_PASS: u32 = u32::MAX;

/// Bytes the frontier occupies, which is the largest binding the pipeline
/// makes and therefore the one a device is most likely to refuse.
pub(crate) const fn frontier_size(cell_count: u32) -> u64 {
    2 * cell_count as u64 * size_of::<u32>() as u64
}

/// Configuration of the raster plate partition.
///
/// Plate counts, roughness, and the seed carry the mesh pipeline's meaning
/// unchanged. The head start does not: the mesh measures it in graph hops,
/// which have no fixed size on a raster, so the pilot measures it as an arc and
/// converts it against the face resolution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterPlatePartitionConfig {
    pub major_plate_count: u32,
    pub minor_plate_count: u32,
    /// Great-circle arc, in radians, that major plates grow before minor seeds
    /// are placed. Configuring a distance rather than a hop count keeps plate
    /// sizes comparable when the face resolution changes.
    pub major_head_start_arc: f32,
    /// Maximum percentage a link's traversal cost varies from the baseline.
    /// Must not exceed [`MAX_GROWTH_ROUGHNESS`].
    pub growth_roughness: u32,
    pub seed: u64,
}

/// Head start matching the mesh pipeline's six growth rounds, whose 65,536-cell
/// mesh has a mean cell spacing of 0.0138 radians.
const DEFAULT_HEAD_START_ARC: f32 = 0.083;

impl Default for RasterPlatePartitionConfig {
    fn default() -> Self {
        Self {
            major_plate_count: 6,
            minor_plate_count: 111,
            major_head_start_arc: DEFAULT_HEAD_START_ARC,
            growth_roughness: MAX_GROWTH_ROUGHNESS,
            seed: 0,
        }
    }
}

impl RasterPlatePartitionConfig {
    pub const fn plate_count(&self) -> u32 {
        self.major_plate_count
            .saturating_add(self.minor_plate_count)
    }

    /// Converts the head-start arc to growth cost at a face resolution.
    ///
    /// The equi-angular mapping spaces texel centers evenly, so one axis step
    /// spans `FRAC_PI_2 / resolution` radians and costs
    /// `AXIS_LINK_LENGTH * BASE_GROWTH_COST` at the baseline.
    pub fn head_start_cost(&self, resolution: u32) -> u32 {
        let steps = self.major_head_start_arc * resolution as f32 / FRAC_PI_2;
        (steps * (AXIS_LINK_LENGTH * BASE_GROWTH_COST) as f32) as u32
    }

    pub(crate) fn validate(&self, resolution: u32) -> Result<(), RasterTectonicsError> {
        let cell_count = validate_resolution(resolution)?;
        if self.major_plate_count == 0 {
            return Err(RasterTectonicsError::NoMajorPlates);
        }
        if self.plate_count() > MAX_PLATE_COUNT || self.plate_count() > cell_count {
            return Err(RasterTectonicsError::TooManyPlates);
        }
        if self.growth_roughness > MAX_GROWTH_ROUGHNESS {
            return Err(RasterTectonicsError::InvalidGrowthRoughness);
        }
        if !self.major_head_start_arc.is_finite()
            || !(0.0..=std::f32::consts::PI).contains(&self.major_head_start_arc)
        {
            return Err(RasterTectonicsError::InvalidHeadStartArc);
        }
        Ok(())
    }
}

/// Narrows the configuration seed to the 32-bit key the growth kernel hashes.
///
/// This is the pipeline's one seed-narrowing point, so no kernel ever sees a
/// 64-bit value.
pub const fn fold_growth_key(seed: u64) -> u32 {
    const SEED_DOMAIN_TAG: u32 = 0x5345_4544; // ASCII "SEED"
    const RASTER_DOMAIN_TAG: u32 = 0x5241_5354; // ASCII "RAST"

    hash_u32(
        seed as u32,
        (seed >> 32) as u32,
        SEED_DOMAIN_TAG,
        RASTER_DOMAIN_TAG,
    )
}

/// Selects the first major plate's seed cell, matching the mesh pipeline's
/// stream and reduction.
pub fn first_seed_cell(seed: u64, cell_count: u32) -> u32 {
    (RandomStream::new(seed, random_streams::FIRST_MAJOR_PLATE_SEED).sample_u64(0, 0)
        % u64::from(cell_count)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_cubesphere::RasterError;

    #[test]
    fn head_start_arc_converts_to_the_same_distance_at_every_resolution() {
        let config = RasterPlatePartitionConfig::default();
        let axis_step = (AXIS_LINK_LENGTH * BASE_GROWTH_COST) as f32;
        for resolution in [128, 256, 512, 1_024] {
            let steps = config.head_start_cost(resolution) as f32 / axis_step;
            let arc = steps * FRAC_PI_2 / resolution as f32;
            // Truncating to whole cost units is the only loss in the round trip.
            let quantization = FRAC_PI_2 / (resolution as f32 * axis_step);
            assert!(
                (arc - config.major_head_start_arc).abs() <= quantization,
                "resolution {resolution} recovered {arc}"
            );
        }
    }

    #[test]
    fn rejects_configurations_the_kernels_cannot_represent() {
        let config = RasterPlatePartitionConfig::default();
        assert_eq!(
            config.validate(3),
            Err(RasterTectonicsError::Resolution(
                RasterError::InvalidResolution
            ))
        );
        assert_eq!(
            RasterPlatePartitionConfig {
                major_plate_count: 0,
                ..config
            }
            .validate(64),
            Err(RasterTectonicsError::NoMajorPlates)
        );
        assert_eq!(
            RasterPlatePartitionConfig {
                minor_plate_count: MAX_PLATE_COUNT,
                ..config
            }
            .validate(64),
            Err(RasterTectonicsError::TooManyPlates)
        );
        assert_eq!(
            RasterPlatePartitionConfig {
                growth_roughness: MAX_GROWTH_ROUGHNESS + 1,
                ..config
            }
            .validate(64),
            Err(RasterTectonicsError::InvalidGrowthRoughness)
        );
        assert_eq!(
            RasterPlatePartitionConfig {
                major_head_start_arc: f32::NAN,
                ..config
            }
            .validate(64),
            Err(RasterTectonicsError::InvalidHeadStartArc)
        );
    }
}
