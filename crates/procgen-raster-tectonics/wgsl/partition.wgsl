// WGSL kernels for the raster tectonics plate partition.
//
// Compose procgen-core's hash, procgen-cubesphere's mapping and raster
// sources, and the constant preamble that `partition.rs` emits before this
// source. Every kernel shares one bind group so a stage never depends on the
// order the host records dispatches.

struct RasterPartitionConfig {
    resolution: u32,
    cell_count: u32,
    major_plate_count: u32,
    head_start_cost: u32,
    growth_roughness: u32,
    growth_key: u32,
    first_seed_cell: u32,
    padding: u32,
}

// `relax_dispatch_*` are the indirect arguments of the next relaxation pass and
// occupy the first twelve bytes of the buffer.
struct RasterPartitionState {
    relax_dispatch_x: u32,
    relax_dispatch_y: u32,
    relax_dispatch_z: u32,
    /// Frontier generation, which also marks which cells a pass has queued.
    pass_index: u32,
    /// Productive passes the relaxation running now has consumed.
    phase_passes: u32,
    /// Productive passes the longer relaxation of the run has consumed.
    longest_relaxation: u32,
    next_plate: u32,
    chosen_cell: u32,
    frontier_count_even: atomic<u32>,
    frontier_count_odd: atomic<u32>,
}

struct RasterSeedCandidate {
    distance: f32,
    cell: u32,
}

@group(0) @binding(0) var<uniform> config: RasterPartitionConfig;
@group(0) @binding(1) var<storage, read_write> state: RasterPartitionState;
@group(0) @binding(2) var<storage, read_write> labels: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read_write> queued_pass: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read_write> frontier: array<u32>;
@group(0) @binding(5) var<storage, read_write> seed_distance: array<f32>;
@group(0) @binding(6) var<storage, read_write> seed_partials: array<RasterSeedCandidate>;
@group(0) @binding(7) var<storage, read_write> plate_seed_cells: array<u32>;

var<workgroup> seed_reduction: array<RasterSeedCandidate, RASTER_WORKGROUP_SIZE>;

fn raster_pack_label(cost: u32, plate: u32) -> u32 {
    return (cost << RASTER_PLATE_LABEL_BITS) | plate;
}

fn raster_label_cost(label: u32) -> u32 {
    return label >> RASTER_PLATE_LABEL_BITS;
}

fn raster_label_plate(label: u32) -> u32 {
    return label & RASTER_UNCLAIMED_PLATE;
}

/// Deterministic traversal cost of one link, keyed by its canonical cell pair
/// so both directions agree.
fn raster_link_growth_cost(cell: u32, neighbor: u32, link: u32) -> u32 {
    let roughness = config.growth_roughness;
    let offset = hash_u32(
        min(cell, neighbor),
        max(cell, neighbor),
        RASTER_GROWTH_COST_STREAM,
        config.growth_key,
    ) % (2u * roughness + 1u);
    return cubesphere_texel_link_length(link) * (RASTER_BASE_GROWTH_COST - roughness + offset);
}

/// Appends a cell to the frontier half the current pass fills, at most once.
fn raster_frontier_push(cell: u32) {
    let pass_index = state.pass_index;
    if atomicExchange(&queued_pass[cell], pass_index) == pass_index {
        return;
    }
    var slot: u32;
    if (pass_index & 1u) == 0u {
        slot = atomicAdd(&state.frontier_count_even, 1u);
    } else {
        slot = atomicAdd(&state.frontier_count_odd, 1u);
    }
    frontier[(pass_index & 1u) * config.cell_count + slot] = cell;
}

/// The frontier half the current pass reads, filled by the previous pass.
fn raster_frontier_input_half() -> u32 {
    return (state.pass_index + 1u) & 1u;
}

fn raster_frontier_input_count() -> u32 {
    if raster_frontier_input_half() == 0u {
        return atomicLoad(&state.frontier_count_even);
    }
    return atomicLoad(&state.frontier_count_odd);
}

/// Arrival cost above which a cell still counts as unclaimed for seeding.
/// Major seeds are placed before any growth, so their ceiling is zero.
fn raster_seed_cost_ceiling() -> u32 {
    if state.next_plate < config.major_plate_count {
        return 0u;
    }
    return config.head_start_cost;
}

fn raster_better_seed(left: RasterSeedCandidate, right: RasterSeedCandidate) -> RasterSeedCandidate {
    if left.distance > right.distance
        || (left.distance == right.distance && left.cell < right.cell) {
        return left;
    }
    return right;
}

fn raster_no_seed() -> RasterSeedCandidate {
    return RasterSeedCandidate(RASTER_NO_SEED_DISTANCE, CUBESPHERE_NO_RASTER_CELL);
}

@compute @workgroup_size(RASTER_WORKGROUP_SIZE)
fn initialize(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    if id.x == 0u {
        state.relax_dispatch_x = 0u;
        state.relax_dispatch_y = 1u;
        state.relax_dispatch_z = 1u;
        state.pass_index = 0u;
        state.phase_passes = 0u;
        state.longest_relaxation = 0u;
        state.next_plate = 0u;
        state.chosen_cell = CUBESPHERE_NO_RASTER_CELL;
        atomicStore(&state.frontier_count_even, 0u);
        atomicStore(&state.frontier_count_odd, 0u);
    }
    let stride = groups.x * RASTER_WORKGROUP_SIZE;
    for (var cell = id.x; cell < config.cell_count; cell += stride) {
        atomicStore(&labels[cell], RASTER_UNCLAIMED_LABEL);
        atomicStore(&queued_pass[cell], RASTER_NO_FRONTIER_PASS);
        seed_distance[cell] = RASTER_SEED_DISTANCE_CEILING;
    }
}

/// Opens a fresh frontier before a round of seed placement.
@compute @workgroup_size(1)
fn begin_frontier() {
    let pass_index = state.pass_index + 1u;
    state.pass_index = pass_index;
    state.phase_passes = 0u;
    if (pass_index & 1u) == 0u {
        atomicStore(&state.frontier_count_even, 0u);
    } else {
        atomicStore(&state.frontier_count_odd, 0u);
    }
}

/// Reduces the eligible cells to one farthest-point candidate per workgroup.
@compute @workgroup_size(RASTER_WORKGROUP_SIZE)
fn seed_reduce(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let ceiling = raster_seed_cost_ceiling();
    let stride = groups.x * RASTER_WORKGROUP_SIZE;
    var best = raster_no_seed();
    for (var cell = id.x; cell < config.cell_count; cell += stride) {
        if raster_label_cost(atomicLoad(&labels[cell])) > ceiling {
            best = raster_better_seed(best, RasterSeedCandidate(seed_distance[cell], cell));
        }
    }
    seed_reduction[local] = best;
    workgroupBarrier();
    for (var half = RASTER_WORKGROUP_SIZE >> 1u; half > 0u; half >>= 1u) {
        if local < half {
            seed_reduction[local] =
                raster_better_seed(seed_reduction[local], seed_reduction[local + half]);
        }
        workgroupBarrier();
    }
    if local == 0u {
        seed_partials[group.x] = seed_reduction[0];
    }
}

/// Reduces the per-workgroup candidates to the farthest eligible cell.
@compute @workgroup_size(1)
fn seed_select() {
    var best = raster_no_seed();
    for (var index = 0u; index < RASTER_SEED_REDUCTION_WORKGROUPS; index++) {
        best = raster_better_seed(best, seed_partials[index]);
    }
    state.chosen_cell = best.cell;
}

/// Places the next plate's seed. A round that finds no eligible cell leaves the
/// plate seedless, which the seed-cell buffer records.
@compute @workgroup_size(1)
fn seed_place() {
    let plate = state.next_plate;
    state.next_plate = plate + 1u;
    var cell = state.chosen_cell;
    if plate == 0u {
        cell = config.first_seed_cell;
    }
    plate_seed_cells[plate] = cell;
    if cell == CUBESPHERE_NO_RASTER_CELL {
        return;
    }
    var start_cost = 0u;
    if plate >= config.major_plate_count {
        start_cost = config.head_start_cost;
    }
    atomicMin(&labels[cell], raster_pack_label(start_cost, plate));
    raster_frontier_push(cell);
}

/// Folds the newest seed into the running farthest-point distance field.
@compute @workgroup_size(RASTER_WORKGROUP_SIZE)
fn seed_spread(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let seed_cell = plate_seed_cells[state.next_plate - 1u];
    if seed_cell == CUBESPHERE_NO_RASTER_CELL {
        return;
    }
    let seed_direction = cubesphere_texel_direction(seed_cell, config.resolution);
    let stride = groups.x * RASTER_WORKGROUP_SIZE;
    for (var cell = id.x; cell < config.cell_count; cell += stride) {
        let offset = cubesphere_texel_direction(cell, config.resolution) - seed_direction;
        let distance = offset.x * offset.x + offset.y * offset.y + offset.z * offset.z;
        seed_distance[cell] = min(seed_distance[cell], distance);
    }
}

/// Publishes the indirect arguments of the next relaxation pass and clears the
/// frontier half that pass will fill.
///
/// An empty frontier means the relaxation reached its fixed point, and zeroing
/// the relaxation's workgroup count retires the rest of the pass budget without
/// a readback.
@compute @workgroup_size(1)
fn prepare_relax() {
    let pass_index = state.pass_index + 1u;
    var count: u32;
    if (pass_index & 1u) == 0u {
        count = atomicLoad(&state.frontier_count_odd);
    } else {
        count = atomicLoad(&state.frontier_count_even);
    }
    if count == 0u {
        state.relax_dispatch_x = 0u;
        return;
    }
    state.pass_index = pass_index;
    state.phase_passes = state.phase_passes + 1u;
    state.longest_relaxation = max(state.longest_relaxation, state.phase_passes);
    if (pass_index & 1u) == 0u {
        atomicStore(&state.frontier_count_even, 0u);
    } else {
        atomicStore(&state.frontier_count_odd, 0u);
    }
    let stride = RASTER_WORKGROUP_SIZE * RASTER_FRONTIER_CHUNK;
    state.relax_dispatch_x = min(
        (count + stride - 1u) / stride,
        RASTER_MAX_DISPATCH_WORKGROUPS,
    );
}

/// Relaxes one frontier cell into its eight links. The label is a lexicographic
/// (cost, plate) minimum packed into one word, so `atomicMin` settles ties on
/// cost and then on the lower plate id regardless of dispatch order.
fn raster_relax_cell(cell: u32) {
    let label = atomicLoad(&labels[cell]);
    let cost = raster_label_cost(label);
    let plate = raster_label_plate(label);
    for (var link = 0u; link < 8u; link++) {
        let neighbor = cubesphere_texel_neighbor(cell, config.resolution, link);
        if neighbor == CUBESPHERE_NO_RASTER_CELL {
            continue;
        }
        let candidate = raster_pack_label(cost + raster_link_growth_cost(cell, neighbor, link), plate);
        if candidate < atomicMin(&labels[neighbor], candidate) {
            raster_frontier_push(neighbor);
        }
    }
}

@compute @workgroup_size(RASTER_WORKGROUP_SIZE)
fn relax(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let count = raster_frontier_input_count();
    let base = raster_frontier_input_half() * config.cell_count;
    let stride = groups.x * RASTER_WORKGROUP_SIZE;
    for (var index = id.x; index < count; index += stride) {
        raster_relax_cell(frontier[base + index]);
    }
}
