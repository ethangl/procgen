//! GPU-resident tectonics on a cube-sphere raster.
//!
//! Every stage is a WGSL compute kernel dispatched through `wgpu`; there is no
//! CPU implementation and none is planned. [`field`] owns the failure
//! convention, the dispatch shape, and the per-plate record; [`partition`] and
//! [`evolution`] own their stages' backend-neutral contracts; [`kernels`]
//! assembles the WGSL from them; and [`pipeline`] owns dispatch against a
//! device.
//!
//! The pilot this crate serves is described in
//! `docs/compute-shader-tectonics-pilot.md`.

mod evolution;
mod field;
mod kernels;
mod partition;
mod pipeline;

pub use evolution::{
    ANGULAR_VELOCITY_STEPS_PER_UNIT, AREA_UNITS_PER_STERADIAN, BOUNDARY_CLASS_BITS,
    BOUNDARY_CLASS_MASK, CELL_PLATE_BITS, CELL_PLATE_MASK, RasterEvolutionConfig, boundary_class,
    boundary_class_code, crust_class_code, current_plate, pending_plate, plate_ownership,
};
pub use field::{
    MAX_TECTONIC_RESOLUTION, PipelineTuning, RASTER_SPHERE_RADIUS, RasterPlate,
    RasterTectonicsError,
};
pub use kernels::{field_wgsl_source, tectonics_kernel_source};
pub use partition::{
    BASE_GROWTH_COST, MAX_PLATE_COUNT, PLATE_ID_COUNT, PLATE_LABEL_BITS,
    RasterPlatePartitionConfig, UNCLAIMED_LABEL, UNCLAIMED_PLATE, first_seed_cell, fold_growth_key,
    growth_label, growth_label_cost, growth_label_plate,
};
pub use pipeline::{
    PipelineStage, RasterTectonicsConfig, StageTimings, TectonicsDiagnostics, TectonicsPipeline,
    TectonicsRun,
};

/// Stable vectors that reveal whether a backend contracts `a * b + c` into a
/// fused multiply-add.
///
/// Each entry is `(a, b, c, rounded, fused)`, where `rounded` is the `f32`
/// encoding of the product rounded before the sum and `fused` is the encoding
/// of the single rounding a fused operation produces. Every product here is
/// inexact in `f32`, so the two always differ.
///
/// Boundary classes and migration winners are integer outcomes of float
/// comparisons, so contraction moves a boundary rather than rounding it. Which
/// of the two a backend produces is a property of its shader compiler that
/// `wgpu` does not expose a control for, so the pilot records it per backend
/// rather than requiring one; a result matching neither means the backend is
/// reassociating or reducing precision, which no kernel here can be reasoned
/// about on.
pub const FMA_CONTRACTION_TEST_VECTORS: [(f32, f32, f32, u32, u32); 4] = [
    (1.000_244_1, 1.000_244_1, -1.0, 0x3a00_0000, 0x3a00_0400),
    (1.000_244_1, 1.000_122_1, -1.0, 0x39c0_0000, 0x39c0_0400),
    (1.5, 1.000_000_1, -1.5, 0x3480_0000, 0x3440_0000),
    (0.333_333_34, 3.0, -1.0, 0x0000_0000, 0x3300_0000),
];
