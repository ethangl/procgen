// WGSL kernels for the raster plate evolution: crust classification, boundary
// classification, and the simultaneous migration step the evolution repeats.
//
// Compose bindings.wgsl before this source.

/// Per-plate areas one workgroup accumulates before it touches the shared
/// counters, so millions of cells do not contend on a few hundred addresses.
/// Staging changes nothing: integer addition is associative and commutative.
var<workgroup> plate_area_stage: array<atomic<u32>, RASTER_PLATE_ID_COUNT>;

/// Copies the settled growth labels into the ownership the evolution advances.
/// Arrival cost belongs to growth alone, so nothing downstream carries it.
@compute @workgroup_size(RASTER_WORKGROUP_SIZE)
fn derive_ownership(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let stride = raster_cell_stride(groups);
    for (var cell = id.x; cell < config.cell_count; cell += stride) {
        let plate = raster_label_plate(atomicLoad(&labels[cell]));
        atomicStore(&ownership[cell], raster_pack_ownership(plate, plate));
    }
}

/// Sums the quantized area of every cell into its owning plate.
@compute @workgroup_size(RASTER_WORKGROUP_SIZE)
fn reduce_plate_areas(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    for (var plate = local; plate < RASTER_PLATE_ID_COUNT; plate += RASTER_WORKGROUP_SIZE) {
        atomicStore(&plate_area_stage[plate], 0u);
    }
    workgroupBarrier();
    let stride = raster_cell_stride(groups);
    for (var cell = id.x; cell < config.cell_count; cell += stride) {
        atomicAdd(
            &plate_area_stage[raster_cell_plate(cell)],
            raster_cell_area(cell, config.resolution),
        );
    }
    workgroupBarrier();
    for (var plate = local; plate < RASTER_PLATE_ID_COUNT; plate += RASTER_WORKGROUP_SIZE) {
        let staged = atomicLoad(&plate_area_stage[plate]);
        if staged > 0u {
            atomicAdd(&state.plate_areas[plate], staged);
        }
    }
}

fn raster_absolute_difference(left: u32, right: u32) -> u32 {
    return max(left, right) - min(left, right);
}

/// Assigns one immutable crust class per plate.
///
/// Plates are visited in the seeded order the host uploaded and taken as
/// oceanic only when their whole area moves the achieved ocean area closer to
/// the target. Areas are integers, so the whole selection is exact.
@compute @workgroup_size(1)
fn classify_crust() {
    var total = 0u;
    for (var plate = 0u; plate < RASTER_PLATE_ID_COUNT; plate++) {
        total += atomicLoad(&state.plate_areas[plate]);
    }
    let target_area = u32(f32(total) * config.target_ocean_fraction);
    var ocean = 0u;
    var empty = 0u;
    for (var index = 0u; index < config.plate_count; index++) {
        let plate = plates[index].order;
        let area = atomicLoad(&state.plate_areas[plate]);
        plates[plate].area = area;
        empty += u32(area == 0u);
        let candidate = ocean + area;
        if raster_absolute_difference(candidate, target_area)
            < raster_absolute_difference(ocean, target_area) {
            plates[plate].crust = RASTER_CRUST_OCEANIC;
            ocean = candidate;
        } else {
            plates[plate].crust = RASTER_CRUST_CONTINENTAL;
        }
    }
    state.diagnostics.total_area = total;
    state.diagnostics.ocean_area = ocean;
    state.diagnostics.empty_plate_count = empty;
}

/// Clears the class counters so they describe the classification about to run.
@compute @workgroup_size(1)
fn begin_boundaries() {
    for (var border = 0u; border < RASTER_BOUNDARY_CLASS_COUNT; border++) {
        atomicStore(&state.diagnostics.boundary_class_counts[border], 0u);
    }
}

/// Classifies each cell's four borders from the local relative motion of the
/// plates that meet there. No ownership or crust state is advanced.
///
/// Every border is classified twice, once from each of its cells, and both
/// reach the same class from the same inputs. Only the cell that owns the
/// border edge counts it, so the class proportions count each border once.
@compute @workgroup_size(RASTER_WORKGROUP_SIZE)
fn classify_boundaries(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let stride = raster_cell_stride(groups);
    for (var cell = id.x; cell < config.cell_count; cell += stride) {
        let plate = raster_cell_plate(cell);
        let direction = raster_cell_direction(cell);
        var classes = 0u;
        for (var link = 0u; link < CUBESPHERE_BORDER_LINKS_PER_CELL; link++) {
            let neighbor = cubesphere_texel_neighbor(cell, config.resolution, link);
            let other = raster_cell_plate(neighbor);
            var border = RASTER_BOUNDARY_INTERIOR;
            if other != plate {
                border = raster_motion_class(raster_border_motion(
                    direction,
                    plates[plate],
                    raster_cell_direction(neighbor),
                    plates[other],
                ));
            }
            classes = raster_pack_boundary_class(classes, link, border);
            if cell < neighbor {
                atomicAdd(&state.diagnostics.boundary_class_counts[border], 1u);
            }
        }
        boundary_classes[cell] = classes;
    }
}

/// Gathers the migration proposals aimed at each cell and keeps the winner.
///
/// A cell inspects the convergent borders it sits on and keeps the proposals
/// where the neighbouring side advances into it. The winner is written to the
/// cell's pending plate rather than its current one, so the whole step reads
/// one ownership state and writes another.
@compute @workgroup_size(RASTER_WORKGROUP_SIZE)
fn propose_migration(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let stride = raster_cell_stride(groups);
    for (var cell = id.x; cell < config.cell_count; cell += stride) {
        let plate = raster_cell_plate(cell);
        let direction = raster_cell_direction(cell);
        let classes = boundary_classes[cell];
        var winner = plate;
        var best_convergence = 0.0;
        var best_edge = 0u;
        var proposed = false;
        for (var link = 0u; link < CUBESPHERE_BORDER_LINKS_PER_CELL; link++) {
            if raster_boundary_class(classes, link) != RASTER_BOUNDARY_CONVERGENT {
                continue;
            }
            let neighbor = cubesphere_texel_neighbor(cell, config.resolution, link);
            let other = raster_cell_plate(neighbor);
            let motion = raster_border_motion(
                direction,
                plates[plate],
                raster_cell_direction(neighbor),
                plates[other],
            );
            let convergence = raster_convergence(motion);
            if convergence < config.minimum_convergence {
                continue;
            }
            if raster_side_advances(
                plate,
                plates[plate].crust,
                motion.normal_speeds.x,
                other,
                plates[other].crust,
                motion.normal_speeds.y,
            ) {
                continue;
            }
            let edge = cubesphere_border_edge(cell, config.resolution, link);
            if !proposed
                || raster_proposal_precedes(
                    convergence, other, edge, best_convergence, winner, best_edge) {
                winner = other;
                best_convergence = convergence;
                best_edge = edge;
                proposed = true;
            }
        }
        atomicStore(&ownership[cell], raster_pack_ownership(plate, winner));
    }
}

/// Applies the winning proposals, which is what makes the step simultaneous.
@compute @workgroup_size(RASTER_WORKGROUP_SIZE)
fn apply_migration(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let stride = raster_cell_stride(groups);
    var migrated = 0u;
    for (var cell = id.x; cell < config.cell_count; cell += stride) {
        let word = atomicLoad(&ownership[cell]);
        let pending = raster_pending_plate(word);
        migrated += u32(pending != raster_current_plate(word));
        atomicStore(&ownership[cell], raster_pack_ownership(pending, pending));
    }
    atomicAdd(&state.diagnostics.migrated_cell_count, migrated);
}
