//! Data contracts for the raster plate partition.
//!
//! Everything here is backend-neutral: the configuration, the packed growth
//! label, the buffer layouts the kernels read, and the assembled WGSL source.
//! Dispatch lives in [`crate::pipeline`].

use procgen_core::{RandomStream, hash_u32, random_streams};
use procgen_cubesphere::{
    AXIS_LINK_LENGTH, DIAGONAL_LINK_LENGTH, FaceTexel, MAPPING_WGSL_SOURCE, RASTER_WGSL_SOURCE,
    RasterError,
};
use procgen_tectonics::MAX_GROWTH_ROUGHNESS;
use std::{f32::consts::FRAC_PI_2, fmt};

/// Bits of a packed growth label reserved for the owning plate id.
///
/// A label is `(arrival cost << PLATE_LABEL_BITS) | plate`, so the numeric
/// order of the word is the lexicographic order of `(cost, plate)` and one
/// `atomicMin` settles growth ties on cost and then on the lower plate id.
pub const PLATE_LABEL_BITS: u32 = 9;
/// Plate id reserved to mean "no plate has reached this cell".
pub const UNCLAIMED_PLATE: u32 = (1 << PLATE_LABEL_BITS) - 1;
/// Largest plate count a partition can address, since one id is reserved.
pub const MAX_PLATE_COUNT: u32 = UNCLAIMED_PLATE;
/// The label of a cell no plate has reached.
pub const UNCLAIMED_LABEL: u32 = u32::MAX;
/// Largest arrival cost the packed label can carry.
pub const MAX_GROWTH_COST: u32 = UNCLAIMED_LABEL >> PLATE_LABEL_BITS;
/// Traversal cost of one unit of link length before roughness is applied.
pub const BASE_GROWTH_COST: u32 = 100;
/// Largest face resolution the pilot runs, and the cap the cost budget assumes.
pub const MAX_TECTONIC_RESOLUTION: u32 = 1_024;

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
pub const fn growth_passes(resolution: u32) -> u32 {
    GROWTH_PATH_ROUGHNESS_MARGIN * MAX_SHORTEST_PATH_FACE_WIDTHS * resolution
}

/// Workgroups the farthest-point reduction always dispatches. Each one strides
/// over the whole raster and emits one candidate.
pub(crate) const SEED_REDUCTION_WORKGROUPS: u32 = 256;

/// Workgroups one dispatch may cover, from `wgpu`'s default device limits.
const MAX_DISPATCH_WORKGROUPS: u32 = 65_535;

/// Squared chord distance no pair of unit directions can reach, used as the
/// farthest-point field's initial value.
const SEED_DISTANCE_CEILING: f32 = 1.0e30;
/// Distance of the candidate a reduction emits when it found no eligible cell.
const NO_SEED_DISTANCE: f32 = -1.0;
/// Frontier pass index no cell has been queued for.
const NO_FRONTIER_PASS: u32 = u32::MAX;

const PARTITION_WGSL_SOURCE: &str = include_str!("../wgsl/partition.wgsl");

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

    pub(crate) fn validate(&self, resolution: u32) -> Result<(), RasterPartitionError> {
        let cell_count = validate_resolution(resolution)?;
        if self.major_plate_count == 0 {
            return Err(RasterPartitionError::NoMajorPlates);
        }
        if self.plate_count() > MAX_PLATE_COUNT || self.plate_count() > cell_count {
            return Err(RasterPartitionError::TooManyPlates);
        }
        if self.growth_roughness > MAX_GROWTH_ROUGHNESS {
            return Err(RasterPartitionError::InvalidGrowthRoughness);
        }
        if !self.major_head_start_arc.is_finite()
            || !(0.0..=std::f32::consts::PI).contains(&self.major_head_start_arc)
        {
            return Err(RasterPartitionError::InvalidHeadStartArc);
        }
        Ok(())
    }
}

/// Returns the cell count of a face resolution the pilot can run.
pub(crate) fn validate_resolution(resolution: u32) -> Result<u32, RasterPartitionError> {
    let cell_count = FaceTexel::cell_count(resolution)?;
    if resolution > MAX_TECTONIC_RESOLUTION {
        return Err(RasterPartitionError::UnsupportedResolution);
    }
    Ok(cell_count)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterPartitionError {
    Resolution(RasterError),
    UnsupportedResolution,
    NoMajorPlates,
    TooManyPlates,
    InvalidGrowthRoughness,
    InvalidHeadStartArc,
    InvalidWorkgroupSize,
    InvalidFrontierChunk,
}

impl From<RasterError> for RasterPartitionError {
    fn from(error: RasterError) -> Self {
        Self::Resolution(error)
    }
}

impl fmt::Display for RasterPartitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolution(error) => error.fmt(formatter),
            Self::UnsupportedResolution => write!(
                formatter,
                "raster tectonics runs at face resolutions up to {MAX_TECTONIC_RESOLUTION}"
            ),
            Self::NoMajorPlates => formatter.write_str("at least one major plate is required"),
            Self::TooManyPlates => write!(
                formatter,
                "plate count cannot exceed the cell count or {MAX_PLATE_COUNT}"
            ),
            Self::InvalidGrowthRoughness => write!(
                formatter,
                "plate growth roughness cannot exceed {MAX_GROWTH_ROUGHNESS}%"
            ),
            Self::InvalidHeadStartArc => {
                formatter.write_str("the major head start must be an arc between 0 and PI radians")
            }
            Self::InvalidWorkgroupSize => write!(
                formatter,
                "the workgroup size must be a power of two of at most {} invocations",
                PipelineTuning::MAX_WORKGROUP_SIZE
            ),
            Self::InvalidFrontierChunk => {
                formatter.write_str("the frontier chunk must relax at least one entry")
            }
        }
    }
}

impl std::error::Error for RasterPartitionError {}

/// Dispatch shape the kernels are compiled for.
///
/// Neither field changes any result: the frontier is order-independent and the
/// farthest-point reduction is an exact minimum. They exist so the pipeline
/// tests can assert that by running the same seed at different shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipelineTuning {
    /// Invocations per workgroup. Must be a power of two of at most 256.
    pub workgroup_size: u32,
    /// Frontier entries one invocation relaxes per pass, which sizes the
    /// indirect dispatch.
    pub frontier_chunk: u32,
}

impl Default for PipelineTuning {
    fn default() -> Self {
        Self {
            workgroup_size: 64,
            frontier_chunk: 4,
        }
    }
}

impl PipelineTuning {
    /// Largest workgroup size `wgpu`'s default device limits allow.
    pub const MAX_WORKGROUP_SIZE: u32 = 256;

    pub(crate) fn validate(&self) -> Result<(), RasterPartitionError> {
        if self.workgroup_size == 0
            || self.workgroup_size > Self::MAX_WORKGROUP_SIZE
            || !self.workgroup_size.is_power_of_two()
        {
            return Err(RasterPartitionError::InvalidWorkgroupSize);
        }
        if self.frontier_chunk == 0 {
            return Err(RasterPartitionError::InvalidFrontierChunk);
        }
        Ok(())
    }

    /// Workgroups a kernel that strides over every cell dispatches.
    pub(crate) fn cell_workgroups(&self, cell_count: u32) -> u32 {
        cell_count
            .div_ceil(self.workgroup_size * self.frontier_chunk)
            .clamp(1, MAX_DISPATCH_WORKGROUPS)
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

/// The uniform block every partition kernel reads. The trailing word rounds the
/// block to the sixteen-byte stride a uniform binding requires.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PackedPartitionConfig {
    resolution: u32,
    cell_count: u32,
    major_plate_count: u32,
    head_start_cost: u32,
    growth_roughness: u32,
    growth_key: u32,
    first_seed_cell: u32,
    padding: u32,
}

impl PackedPartitionConfig {
    pub(crate) fn new(
        config: &RasterPlatePartitionConfig,
        resolution: u32,
        cell_count: u32,
    ) -> Self {
        Self {
            resolution,
            cell_count,
            major_plate_count: config.major_plate_count,
            head_start_cost: config.head_start_cost(resolution),
            growth_roughness: config.growth_roughness,
            growth_key: fold_growth_key(config.seed),
            first_seed_cell: first_seed_cell(config.seed, cell_count),
            padding: 0,
        }
    }
}

/// Bytes one farthest-point candidate occupies: an `f32` distance beside the
/// `u32` cell id that carries it.
pub(crate) const SEED_CANDIDATE_SIZE: u64 = 2 * size_of::<u32>() as u64;

/// Assembles the partition kernels for one dispatch shape.
///
/// The source composes `procgen-core`'s hash and `procgen-cubesphere`'s mapping
/// and raster mirrors, so every constant this crate owns reaches WGSL from its
/// Rust definition rather than a shader literal.
pub fn partition_kernel_source(tuning: PipelineTuning) -> String {
    let PipelineTuning {
        workgroup_size,
        frontier_chunk,
    } = tuning;
    let constants = format!(
        "const RASTER_PLATE_LABEL_BITS: u32 = {PLATE_LABEL_BITS}u;\n\
         const RASTER_UNCLAIMED_PLATE: u32 = {UNCLAIMED_PLATE}u;\n\
         const RASTER_UNCLAIMED_LABEL: u32 = {UNCLAIMED_LABEL}u;\n\
         const RASTER_BASE_GROWTH_COST: u32 = {BASE_GROWTH_COST}u;\n\
         const RASTER_GROWTH_COST_STREAM: u32 = {}u;\n\
         const RASTER_SEED_REDUCTION_WORKGROUPS: u32 = {SEED_REDUCTION_WORKGROUPS}u;\n\
         const RASTER_MAX_DISPATCH_WORKGROUPS: u32 = {MAX_DISPATCH_WORKGROUPS}u;\n\
         const RASTER_SEED_DISTANCE_CEILING: f32 = {SEED_DISTANCE_CEILING:e};\n\
         const RASTER_NO_SEED_DISTANCE: f32 = {NO_SEED_DISTANCE:?};\n\
         const RASTER_NO_FRONTIER_PASS: u32 = {NO_FRONTIER_PASS}u;\n\
         const RASTER_WORKGROUP_SIZE: u32 = {workgroup_size}u;\n\
         const RASTER_FRONTIER_CHUNK: u32 = {frontier_chunk}u;",
        random_streams::PLATE_GROWTH_COST as u32,
    );
    [
        procgen_core::HASH_WGSL_SOURCE,
        MAPPING_WGSL_SOURCE,
        RASTER_WGSL_SOURCE,
        &constants,
        PARTITION_WGSL_SOURCE,
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_labels_order_by_cost_then_plate() {
        let pack = |cost: u32, plate: u32| (cost << PLATE_LABEL_BITS) | plate;
        let mut labels: Vec<u32> = [(8, 4), (7, 500), (8, 3), (7, 0)]
            .into_iter()
            .map(|(cost, plate)| pack(cost, plate))
            .collect();
        labels.sort_unstable();
        assert_eq!(
            labels
                .iter()
                .map(|&label| (
                    crate::growth_label_cost(label),
                    crate::growth_label_plate(label)
                ))
                .collect::<Vec<_>>(),
            [(7, 0), (7, 500), (8, 3), (8, 4)]
        );
        assert_eq!(
            UNCLAIMED_LABEL,
            pack(MAX_GROWTH_COST, UNCLAIMED_PLATE),
            "the unclaimed label must be the largest label, so atomicMin settles it"
        );
    }

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
            Err(RasterPartitionError::Resolution(
                RasterError::InvalidResolution
            ))
        );
        assert_eq!(
            config.validate(2_048),
            Err(RasterPartitionError::UnsupportedResolution)
        );
        assert_eq!(
            RasterPlatePartitionConfig {
                major_plate_count: 0,
                ..config
            }
            .validate(64),
            Err(RasterPartitionError::NoMajorPlates)
        );
        assert_eq!(
            RasterPlatePartitionConfig {
                minor_plate_count: MAX_PLATE_COUNT,
                ..config
            }
            .validate(64),
            Err(RasterPartitionError::TooManyPlates)
        );
        assert_eq!(
            RasterPlatePartitionConfig {
                growth_roughness: MAX_GROWTH_ROUGHNESS + 1,
                ..config
            }
            .validate(64),
            Err(RasterPartitionError::InvalidGrowthRoughness)
        );
        assert_eq!(
            RasterPlatePartitionConfig {
                major_head_start_arc: f32::NAN,
                ..config
            }
            .validate(64),
            Err(RasterPartitionError::InvalidHeadStartArc)
        );
    }

    #[test]
    fn rejects_dispatch_shapes_the_kernels_cannot_compile() {
        assert_eq!(
            PipelineTuning {
                workgroup_size: 48,
                frontier_chunk: 1,
            }
            .validate(),
            Err(RasterPartitionError::InvalidWorkgroupSize)
        );
        assert_eq!(
            PipelineTuning {
                workgroup_size: 512,
                frontier_chunk: 1,
            }
            .validate(),
            Err(RasterPartitionError::InvalidWorkgroupSize)
        );
        assert_eq!(
            PipelineTuning {
                workgroup_size: 64,
                frontier_chunk: 0,
            }
            .validate(),
            Err(RasterPartitionError::InvalidFrontierChunk)
        );
    }

    #[test]
    fn cell_dispatches_stay_inside_the_device_workgroup_limit() {
        let tuning = PipelineTuning {
            workgroup_size: 64,
            frontier_chunk: 1,
        };
        let cells = FaceTexel::cell_count(MAX_TECTONIC_RESOLUTION).unwrap();
        assert_eq!(tuning.cell_workgroups(cells), MAX_DISPATCH_WORKGROUPS);
        assert_eq!(tuning.cell_workgroups(1), 1);
    }
}
