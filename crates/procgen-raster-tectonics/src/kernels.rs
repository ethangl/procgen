//! Assembly of the crate's WGSL from its Rust definitions.
//!
//! Every constant a kernel reads reaches WGSL from the Rust constant that owns
//! it, so the two can never drift. The sources compose `procgen-core`'s hash
//! and `procgen-cubesphere`'s mapping and raster mirrors rather than restating
//! them.

use crate::device::{MAX_DISPATCH_WORKGROUPS, PipelineTuning};
use crate::field::{
    AREA_UNITS_PER_STERADIAN, BOUNDARY_CLASS_BITS, BOUNDARY_CLASS_MASK, CELL_PLATE_BITS,
    CELL_PLATE_MASK, PLATE_ID_COUNT, PLATE_LABEL_BITS, UNCLAIMED_LABEL, UNCLAIMED_PLATE,
    boundary_class_code, crust_class_code,
};
use crate::partition::{
    BASE_GROWTH_COST, NO_FRONTIER_PASS, NO_SEED_DISTANCE, SEED_DISTANCE_CEILING,
    SEED_REDUCTION_WORKGROUPS,
};
use procgen_cubesphere::{MAPPING_WGSL_SOURCE, RASTER_WGSL_SOURCE};
use procgen_tectonics::{BoundaryClass, CONVERGENCE_TO_SHEAR_THRESHOLD, CrustClass};

const FIELD_WGSL_SOURCE: &str = include_str!("../wgsl/field.wgsl");
const BINDINGS_WGSL_SOURCE: &str = include_str!("../wgsl/bindings.wgsl");
const PARTITION_WGSL_SOURCE: &str = include_str!("../wgsl/partition.wgsl");
const EVOLUTION_WGSL_SOURCE: &str = include_str!("../wgsl/evolution.wgsl");

/// Assembles the packed-field accessors and the constants behind them, which
/// mirror [`crate::field`] one for one.
///
/// Applications that display the pipeline's buffers compose this source, so
/// the packing has one definition rather than one per consumer. It declares no
/// binding and depends only on `procgen-cubesphere`'s mapping and raster
/// mirrors, which the caller composes first.
pub fn field_wgsl_source() -> String {
    let constants = format!(
        "const RASTER_PLATE_LABEL_BITS: u32 = {PLATE_LABEL_BITS}u;\n\
         const RASTER_PLATE_ID_COUNT: u32 = {PLATE_ID_COUNT}u;\n\
         const RASTER_UNCLAIMED_PLATE: u32 = {UNCLAIMED_PLATE}u;\n\
         const RASTER_UNCLAIMED_LABEL: u32 = {UNCLAIMED_LABEL}u;\n\
         const RASTER_CELL_PLATE_BITS: u32 = {CELL_PLATE_BITS}u;\n\
         const RASTER_CELL_PLATE_MASK: u32 = {CELL_PLATE_MASK}u;\n\
         const RASTER_BOUNDARY_CLASS_BITS: u32 = {BOUNDARY_CLASS_BITS}u;\n\
         const RASTER_BOUNDARY_CLASS_MASK: u32 = {BOUNDARY_CLASS_MASK}u;\n\
         const RASTER_BOUNDARY_CLASS_COUNT: u32 = {}u;\n\
         const RASTER_BOUNDARY_INTERIOR: u32 = {}u;\n\
         const RASTER_BOUNDARY_CONVERGENT: u32 = {}u;\n\
         const RASTER_BOUNDARY_DIVERGENT: u32 = {}u;\n\
         const RASTER_BOUNDARY_TRANSFORM: u32 = {}u;\n\
         const RASTER_CRUST_OCEANIC: u32 = {}u;\n\
         const RASTER_CRUST_CONTINENTAL: u32 = {}u;\n\
         const RASTER_CONVERGENCE_TO_SHEAR_THRESHOLD: f32 = {CONVERGENCE_TO_SHEAR_THRESHOLD:?};\n\
         const RASTER_AREA_UNITS_PER_STERADIAN: f32 = {}.0;",
        BoundaryClass::ALL.len(),
        boundary_class_code(BoundaryClass::Interior),
        boundary_class_code(BoundaryClass::Convergent),
        boundary_class_code(BoundaryClass::Divergent),
        boundary_class_code(BoundaryClass::Transform),
        crust_class_code(CrustClass::Oceanic),
        crust_class_code(CrustClass::Continental),
        AREA_UNITS_PER_STERADIAN,
    );
    [constants.as_str(), FIELD_WGSL_SOURCE].join("\n")
}

/// Assembles every tectonics kernel for one dispatch shape.
pub fn tectonics_kernel_source(tuning: PipelineTuning) -> String {
    let PipelineTuning {
        workgroup_size,
        frontier_chunk,
    } = tuning;
    let constants = format!(
        "const RASTER_BASE_GROWTH_COST: u32 = {BASE_GROWTH_COST}u;\n\
         const RASTER_GROWTH_COST_STREAM: u32 = {}u;\n\
         const RASTER_SEED_REDUCTION_WORKGROUPS: u32 = {SEED_REDUCTION_WORKGROUPS}u;\n\
         const RASTER_MAX_DISPATCH_WORKGROUPS: u32 = {MAX_DISPATCH_WORKGROUPS}u;\n\
         const RASTER_SEED_DISTANCE_CEILING: f32 = {SEED_DISTANCE_CEILING:e};\n\
         const RASTER_NO_SEED_DISTANCE: f32 = {NO_SEED_DISTANCE:?};\n\
         const RASTER_NO_FRONTIER_PASS: u32 = {NO_FRONTIER_PASS}u;\n\
         const RASTER_WORKGROUP_SIZE: u32 = {workgroup_size}u;\n\
         const RASTER_FRONTIER_CHUNK: u32 = {frontier_chunk}u;",
        procgen_core::random_streams::PLATE_GROWTH_COST as u32,
    );
    [
        procgen_core::HASH_WGSL_SOURCE,
        MAPPING_WGSL_SOURCE,
        RASTER_WGSL_SOURCE,
        &field_wgsl_source(),
        &constants,
        BINDINGS_WGSL_SOURCE,
        PARTITION_WGSL_SOURCE,
        EVOLUTION_WGSL_SOURCE,
    ]
    .join("\n")
}
