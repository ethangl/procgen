//! GPU-resident tectonics on a cube-sphere raster.
//!
//! Every stage is a WGSL compute kernel dispatched through `wgpu`; there is no
//! CPU implementation and none is planned. [`partition`] owns the backend-
//! neutral contracts—configuration, packed labels, buffer layouts, and the
//! assembled kernel source—and [`pipeline`] owns dispatch against a device.
//!
//! The pilot this crate serves is described in
//! `docs/compute-shader-tectonics-pilot.md`.

mod partition;
mod pipeline;

pub use partition::{
    BASE_GROWTH_COST, MAX_PLATE_COUNT, MAX_TECTONIC_RESOLUTION, PLATE_LABEL_BITS, PipelineTuning,
    RasterPartitionError, RasterPlatePartitionConfig, UNCLAIMED_LABEL, UNCLAIMED_PLATE,
    first_seed_cell, fold_growth_key, growth_label, growth_label_cost, growth_label_plate,
    partition_kernel_source,
};
pub use pipeline::{PartitionRun, PipelineStage, PlatePartitionPipeline, StageTimings};
