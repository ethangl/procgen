//! GPU-resident tectonics on a cube-sphere raster.
//!
//! Every stage is a WGSL compute kernel dispatched through `wgpu`; there is no
//! CPU implementation and none is planned. [`field`] owns how the buffers pack
//! their fields and mirrors `wgsl/field.wgsl` one for one; [`device`] owns the
//! failure convention and what the kernels need of a device; [`partition`] and
//! [`evolution`] own their stages' backend-neutral contracts; [`kernels`]
//! assembles the WGSL from all of them; [`layout`] mirrors the kernels' own
//! structs; and [`pipeline`] owns dispatch.
//!
//! The pilot this crate serves is described in
//! `docs/compute-shader-tectonics-pilot.md`.

mod device;
mod evolution;
mod field;
mod kernels;
mod layout;
mod partition;
mod pipeline;

pub use device::{MAX_TECTONIC_RESOLUTION, PipelineTuning, RasterTectonicsError};
pub use evolution::{ANGULAR_VELOCITY_STEPS_PER_UNIT, RasterEvolutionConfig};
pub use field::{
    AREA_UNITS_PER_STERADIAN, BOUNDARY_CLASS_BITS, BOUNDARY_CLASS_MASK, CELL_PLATE_BITS,
    CELL_PLATE_MASK, MAX_GROWTH_COST, MAX_PLATE_COUNT, PLATE_ID_COUNT, PLATE_LABEL_BITS,
    RASTER_SPHERE_RADIUS, RasterPlate, UNCLAIMED_LABEL, UNCLAIMED_PLATE, boundary_class,
    boundary_class_code, crust_class_code, current_plate, growth_label, growth_label_cost,
    growth_label_plate, pending_plate, plate_ownership,
};
pub use kernels::{field_wgsl_source, tectonics_kernel_source};
pub use partition::{
    BASE_GROWTH_COST, RasterPlatePartitionConfig, first_seed_cell, fold_growth_key,
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
