// Bindings every raster tectonics kernel shares, so a stage never depends on
// the order the host records dispatches.
//
// The kernels sit at wgpu's default limit of eight storage buffers per stage:
// per-cell state that would otherwise want its own binding is packed into one
// of these instead.

struct RasterTectonicsConfig {
    resolution: u32,
    cell_count: u32,
    plate_count: u32,
    major_plate_count: u32,
    head_start_cost: u32,
    growth_roughness: u32,
    growth_key: u32,
    first_seed_cell: u32,
    target_ocean_fraction: f32,
    minimum_convergence: f32,
    padding: vec2<u32>,
}

struct RasterSeedCandidate {
    distance: f32,
    cell: u32,
}

/// Counters the host copies back once a run finishes. Every one is an integer
/// reduction, so none of them depends on the order work was scheduled in.
struct RasterTectonicsDiagnostics {
    /// Productive passes the longer relaxation of the run consumed.
    longest_relaxation: u32,
    total_area: u32,
    ocean_area: u32,
    empty_plate_count: u32,
    migrated_cell_count: atomic<u32>,
    boundary_class_counts: array<atomic<u32>, RASTER_BOUNDARY_CLASS_COUNT>,
}

// `relax_dispatch_*` are the indirect arguments of the next relaxation pass and
// occupy the first twelve bytes of the buffer.
struct RasterTectonicsState {
    relax_dispatch_x: u32,
    relax_dispatch_y: u32,
    relax_dispatch_z: u32,
    /// Frontier generation, which also marks which cells a pass has queued.
    pass_index: u32,
    /// Productive passes the relaxation running now has consumed.
    phase_passes: u32,
    next_plate: u32,
    chosen_cell: u32,
    /// Entries in each frontier half, indexed by the pass that fills it.
    frontier_count: array<atomic<u32>, 2>,
    diagnostics: RasterTectonicsDiagnostics,
    plate_areas: array<atomic<u32>, RASTER_PLATE_ID_COUNT>,
    seed_partials: array<RasterSeedCandidate, RASTER_SEED_REDUCTION_WORKGROUPS>,
}

@group(0) @binding(0) var<uniform> config: RasterTectonicsConfig;
@group(0) @binding(1) var<storage, read_write> state: RasterTectonicsState;
@group(0) @binding(2) var<storage, read_write> labels: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read_write> queued_pass: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read_write> frontier: array<u32>;
@group(0) @binding(5) var<storage, read_write> seed_distance: array<f32>;
@group(0) @binding(6) var<storage, read_write> ownership: array<atomic<u32>>;
@group(0) @binding(7) var<storage, read_write> boundary_classes: array<u32>;
@group(0) @binding(8) var<storage, read_write> plates: array<RasterPlate>;

/// Cells one strided per-cell kernel advances by between iterations.
fn raster_cell_stride(groups: vec3<u32>) -> u32 {
    return groups.x * RASTER_WORKGROUP_SIZE;
}

fn raster_cell_plate(cell: u32) -> u32 {
    return raster_current_plate(atomicLoad(&ownership[cell]));
}

fn raster_cell_direction(cell: u32) -> vec3<f32> {
    return cubesphere_texel_direction(cell, config.resolution);
}
