// WGSL kernels for the raster tectonics plate partition: clearing the resident
// buffers, farthest-point seed placement, and the frontier relaxation that
// grows plates over the chamfer links.
//
// Compose bindings.wgsl before this source.

var<workgroup> seed_reduction: array<RasterSeedCandidate, RASTER_WORKGROUP_SIZE>;

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
    let half = pass_index & 1u;
    let slot = atomicAdd(&state.frontier_count[half], 1u);
    frontier[half * config.cell_count + slot] = cell;
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
        state.next_plate = 0u;
        // The first plate is seeded from the configuration, so the host skips
        // its reduction and this is the cell it places.
        state.chosen_cell = config.first_seed_cell;
        atomicStore(&state.frontier_count[0], 0u);
        atomicStore(&state.frontier_count[1], 0u);
        // Only the two diagnostics that accumulate across the whole run are
        // cleared here, because no single stage owns them. Every other counter
        // is written outright by the stage that produces it.
        state.diagnostics.longest_relaxation = 0u;
        atomicStore(&state.diagnostics.migrated_cell_count, 0u);
    }
    let stride = raster_cell_stride(groups);
    for (var plate = id.x; plate < RASTER_PLATE_ID_COUNT; plate += stride) {
        atomicStore(&state.plate_areas[plate], 0u);
    }
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
    atomicStore(&state.frontier_count[pass_index & 1u], 0u);
}

/// Folds the previous plate's seed into the farthest-point distance field and
/// reduces the eligible cells to one candidate per workgroup.
///
/// The two belong in one pass because the distance a cell contributes is the
/// one the fold just wrote. Each cell still applies the same chain of minimums
/// in the same order, so the field is bit-identical to folding it separately.
/// The first plate is seeded from the configuration, so this never runs with
/// no previous seed to fold; the last plate's seed is never folded because
/// nothing reads the field again.
@compute @workgroup_size(RASTER_WORKGROUP_SIZE)
fn seed_reduce(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let ceiling = raster_seed_cost_ceiling();
    let placed = plates[state.next_plate - 1u].seed_cell;
    let seeded = placed != CUBESPHERE_NO_RASTER_CELL;
    var placed_direction = vec3(0.0, 0.0, 0.0);
    if seeded {
        placed_direction = raster_cell_direction(placed);
    }
    let stride = raster_cell_stride(groups);
    var best = raster_no_seed();
    for (var cell = id.x; cell < config.cell_count; cell += stride) {
        var distance = seed_distance[cell];
        if seeded {
            let offset = raster_cell_direction(cell) - placed_direction;
            distance = min(
                distance,
                offset.x * offset.x + offset.y * offset.y + offset.z * offset.z,
            );
            seed_distance[cell] = distance;
        }
        if raster_label_cost(atomicLoad(&labels[cell])) > ceiling {
            best = raster_better_seed(best, RasterSeedCandidate(distance, cell));
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
        state.seed_partials[group.x] = seed_reduction[0];
    }
}

/// Reduces the per-workgroup candidates to the farthest eligible cell.
@compute @workgroup_size(1)
fn seed_select() {
    var best = raster_no_seed();
    for (var index = 0u; index < RASTER_SEED_REDUCTION_WORKGROUPS; index++) {
        best = raster_better_seed(best, state.seed_partials[index]);
    }
    state.chosen_cell = best.cell;
}

/// Places the next plate's seed on the cell the reduction chose. A round that
/// finds no eligible cell leaves its plate seedless, which the plate buffer
/// records.
@compute @workgroup_size(1)
fn seed_place() {
    let plate = state.next_plate;
    state.next_plate = plate + 1u;
    let cell = state.chosen_cell;
    plates[plate].seed_cell = cell;
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

/// Publishes the indirect arguments of the next relaxation pass and clears the
/// frontier half that pass will fill.
///
/// An empty frontier means the relaxation reached its fixed point, and zeroing
/// the relaxation's workgroup count retires the rest of the pass budget without
/// a readback.
@compute @workgroup_size(1)
fn prepare_relax() {
    let count = atomicLoad(&state.frontier_count[state.pass_index & 1u]);
    if count == 0u {
        state.relax_dispatch_x = 0u;
        return;
    }
    let pass_index = state.pass_index + 1u;
    state.pass_index = pass_index;
    state.phase_passes = state.phase_passes + 1u;
    state.diagnostics.longest_relaxation =
        max(state.diagnostics.longest_relaxation, state.phase_passes);
    atomicStore(&state.frontier_count[pass_index & 1u], 0u);
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
    let half = (state.pass_index + 1u) & 1u;
    let count = atomicLoad(&state.frontier_count[half]);
    let base = half * config.cell_count;
    let stride = raster_cell_stride(groups);
    for (var index = id.x; index < count; index += stride) {
        raster_relax_cell(frontier[base + index]);
    }
}
