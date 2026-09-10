//! Determinism, fixed-point, and structural tests for the raster tectonics
//! pipeline.
//!
//! The expected ownership at small resolution is computed here by a plain
//! shortest-path search over the same link rule, and the structural invariants
//! are recomputed here from the raster's adjacency. That is test scaffolding
//! for a GPU-only stage, not a CPU implementation of it.

use procgen_core::{Vec3, fingerprint, hash_u32, random_streams::PLATE_GROWTH_COST};
use procgen_cubesphere::{FaceTexel, NO_RASTER_CELL, TexelLink};
use procgen_gpu_tests::{
    readback, request_device_with_limits, run_compute, storage_output_buffer, validate_wgsl,
};
use procgen_raster_tectonics::{
    BASE_GROWTH_COST, FMA_CONTRACTION_TEST_VECTORS, PipelineTuning, RasterEvolutionConfig,
    RasterPlate, RasterPlatePartitionConfig, RasterTectonicsConfig, TectonicsPipeline,
    UNCLAIMED_LABEL, boundary_class, crust_class_code, current_plate, first_seed_cell,
    fold_growth_key, growth_label, growth_label_cost, growth_label_plate, pending_plate,
    tectonics_kernel_source,
};
use procgen_tectonics::{
    BoundaryClass, CrustClass, CrustClassificationConfig, PlateKinematicsConfig,
    PlateMigrationConfig,
};
use std::{cmp::Reverse, collections::BinaryHeap};
use wgpu::util::DeviceExt;

/// Resolution the reference search runs at: small enough for a host Dijkstra,
/// large enough that every seam and cube corner carries plate boundaries.
const REFERENCE_RESOLUTION: u32 = 16;
/// Resolution the determinism and invariant tests run at.
const INVARIANCE_RESOLUTION: u32 = 32;

/// Head start large enough at [`REFERENCE_RESOLUTION`] to leave the majors
/// several texels of growth before the minors are seeded.
const REFERENCE_HEAD_START_ARC: f32 = 0.5;

fn reference_config() -> RasterTectonicsConfig {
    RasterTectonicsConfig {
        partition: RasterPlatePartitionConfig {
            major_plate_count: 3,
            minor_plate_count: 5,
            major_head_start_arc: REFERENCE_HEAD_START_ARC,
            ..RasterPlatePartitionConfig::default()
        },
        evolution: RasterEvolutionConfig {
            step_count: 0,
            ..RasterEvolutionConfig::default()
        },
    }
}

/// A configuration whose plates are numerous enough to meet along every kind of
/// boundary and whose evolution runs long enough to move cells.
fn evolving_config() -> RasterTectonicsConfig {
    RasterTectonicsConfig {
        partition: RasterPlatePartitionConfig {
            major_plate_count: 4,
            minor_plate_count: 20,
            seed: 19,
            ..RasterPlatePartitionConfig::default()
        },
        evolution: RasterEvolutionConfig {
            crust: CrustClassificationConfig::new(7),
            kinematics: PlateKinematicsConfig::new(7),
            migration: PlateMigrationConfig::default(),
            step_count: 4,
        },
    }
}

#[test]
fn tectonics_kernels_validate_without_a_device() {
    for tuning in dispatch_shapes() {
        validate_wgsl("raster tectonics", &tectonics_kernel_source(tuning));
    }
}

#[test]
fn pipeline_is_bit_identical_run_to_run_and_across_dispatch_shapes() {
    let Some((device, queue)) = tectonics_device() else {
        return;
    };

    let config = evolving_config();
    let mut expected: Option<Buffers> = None;
    for tuning in dispatch_shapes() {
        let pipeline = TectonicsPipeline::new(&device, INVARIANCE_RESOLUTION, tuning).unwrap();
        for run in 0..2 {
            let outcome = pipeline.run(&device, &queue, &config).unwrap();
            assert!(
                outcome.settled(),
                "the relaxation exhausted its pass budget at {tuning:?}"
            );
            let actual = read_buffers(&device, &queue, &pipeline);
            match &expected {
                None => expected = Some(actual),
                Some(expected) => assert_eq!(
                    *expected, actual,
                    "run {run} at {tuning:?} diverged from the first run"
                ),
            }
        }
    }
}

#[test]
fn partition_settles_the_reference_shortest_path_fixed_point() {
    let Some((device, queue)) = tectonics_device() else {
        return;
    };

    let pipeline =
        TectonicsPipeline::new(&device, REFERENCE_RESOLUTION, PipelineTuning::default()).unwrap();
    for seed in [0, 1, 7, 4_242] {
        let mut config = reference_config();
        config.partition.seed = seed;
        let outcome = pipeline.run(&device, &queue, &config).unwrap();
        assert!(outcome.settled(), "seed {seed} exhausted its pass budget");
        let buffers = read_buffers(&device, &queue, &pipeline);
        let plate_count = config.partition.plate_count();

        assert!(
            buffers
                .labels
                .iter()
                .all(|&label| growth_label_plate(label) < plate_count),
            "seed {seed} left cells unclaimed"
        );
        assert_eq!(
            buffers.labels,
            reference_partition(
                REFERENCE_RESOLUTION,
                &config.partition,
                &buffers.seed_cells()
            ),
            "seed {seed} diverged from the reference search"
        );
    }
}

#[test]
fn evolution_holds_its_structural_invariants() {
    let Some((device, queue)) = tectonics_device() else {
        return;
    };

    let pipeline =
        TectonicsPipeline::new(&device, INVARIANCE_RESOLUTION, PipelineTuning::default()).unwrap();
    let config = evolving_config();
    let run = pipeline.run(&device, &queue, &config).unwrap();
    let buffers = read_buffers(&device, &queue, &pipeline);
    let plate_count = config.partition.plate_count();
    let plates = &buffers.plates[..plate_count as usize];

    for (cell, &ownership) in buffers.ownership.iter().enumerate() {
        assert!(
            current_plate(ownership) < plate_count,
            "cell {cell} is owned by no plate"
        );
        assert_eq!(
            pending_plate(ownership),
            current_plate(ownership),
            "cell {cell} kept a proposal past the step that applied it"
        );
    }

    assert!(
        plates
            .iter()
            .all(|plate| plate.crust == crust_class_code(CrustClass::Oceanic)
                || plate.crust == crust_class_code(CrustClass::Continental)),
        "every plate must carry a crust class"
    );
    assert!(
        plates
            .iter()
            .any(|plate| plate.crust == crust_class_code(CrustClass::Oceanic))
            && plates
                .iter()
                .any(|plate| plate.crust == crust_class_code(CrustClass::Continental)),
        "the target ocean fraction must produce both crust classes"
    );
    let ocean_fraction = run.diagnostics.ocean_fraction;
    let target = config.evolution.crust.target_ocean_fraction;
    assert!(
        (ocean_fraction - target).abs() < 0.1,
        "ocean fraction {ocean_fraction} missed the {target} target"
    );
    assert_eq!(
        run.diagnostics.empty_plate_count, 0,
        "no plate should be starved at this head start"
    );
    assert!(
        run.diagnostics.migrated_cell_count > 0,
        "the evolution must move some cells"
    );

    let mut counts = [0_u32; BoundaryClass::ALL.len()];
    for (cell, &classes) in buffers.boundary_classes.iter().enumerate() {
        let cell = cell as u32;
        let texel = FaceTexel::from_cell_id(cell, INVARIANCE_RESOLUTION).unwrap();
        let plate = current_plate(buffers.ownership[cell as usize]);
        for link in TexelLink::BORDERS {
            let neighbor = texel.neighbor(link).unwrap();
            let other = current_plate(buffers.ownership[neighbor.cell_id() as usize]);
            let class = boundary_class(classes, link);
            assert_eq!(
                class == BoundaryClass::Interior,
                plate == other,
                "cell {cell} link {link:?} classified {class:?} between plates {plate} and {other}"
            );
            assert_eq!(
                class,
                boundary_class(
                    buffers.boundary_classes[neighbor.cell_id() as usize],
                    texel.link_back(link)
                ),
                "the two cells of a border disagree on its class"
            );
            if cell < neighbor.cell_id() {
                counts[class as usize] += 1;
            }
        }
    }
    for class in BoundaryClass::ALL {
        assert_eq!(
            run.diagnostics.count(class),
            counts[class as usize],
            "the device counted {class:?} borders differently from the readback"
        );
    }
    assert!(
        BoundaryClass::ALL
            .into_iter()
            .all(|class| run.diagnostics.count(class) > 0),
        "every boundary class should appear at this plate count"
    );
}

#[test]
fn migration_only_moves_cells_across_convergent_borders() {
    let Some((device, queue)) = tectonics_device() else {
        return;
    };

    let pipeline =
        TectonicsPipeline::new(&device, INVARIANCE_RESOLUTION, PipelineTuning::default()).unwrap();
    let mut config = evolving_config();
    config.evolution.step_count = 0;
    pipeline.run(&device, &queue, &config).unwrap();
    let before = read_buffers(&device, &queue, &pipeline);

    config.evolution.step_count = 1;
    let run = pipeline.run(&device, &queue, &config).unwrap();
    let after = read_buffers(&device, &queue, &pipeline);

    let mut moved = 0;
    for cell in 0..before.ownership.len() as u32 {
        let was = current_plate(before.ownership[cell as usize]);
        let now = current_plate(after.ownership[cell as usize]);
        if was == now {
            continue;
        }
        moved += 1;
        let texel = FaceTexel::from_cell_id(cell, INVARIANCE_RESOLUTION).unwrap();
        let source = TexelLink::BORDERS.into_iter().find(|&link| {
            let neighbor = texel.neighbor(link).unwrap().cell_id();
            boundary_class(before.boundary_classes[cell as usize], link)
                == BoundaryClass::Convergent
                && current_plate(before.ownership[neighbor as usize]) == now
        });
        assert!(
            source.is_some(),
            "cell {cell} moved from plate {was} to {now} without a convergent border to it"
        );
    }
    assert_eq!(
        moved, run.diagnostics.migrated_cell_count,
        "the device counted a different number of ownership changes"
    );
    assert!(moved > 0, "one step must move at least one cell");
}

/// Fingerprints of the default configuration, pinned per seed and face
/// resolution: the growth labels the partition settles on, and the ownership
/// the evolution leaves.
///
/// The growth labels are decided entirely by integer arithmetic, so they must
/// match on both development machines. Ownership additionally depends on
/// integers that float comparisons decide, which is what slice 5 of the pilot
/// measures across the two backends.
const PINNED_FINGERPRINTS: [(u32, u64, u64, u64); 6] = [
    (
        128,
        0,
        7_659_683_194_768_066_236,
        18_415_595_352_329_565_261,
    ),
    (
        128,
        1,
        10_950_105_160_290_849_917,
        17_545_878_357_606_591_297,
    ),
    (
        128,
        4_242,
        222_223_991_584_038_427,
        16_388_713_607_996_835_403,
    ),
    (
        256,
        0,
        16_912_624_269_309_866_239,
        18_041_287_253_728_008_659,
    ),
    (256, 1, 4_257_014_726_406_016_968, 9_194_164_045_646_768_315),
    (
        256,
        4_242,
        17_100_062_096_016_943_217,
        10_300_056_875_508_338_882,
    ),
];

#[test]
fn default_pipeline_has_pinned_fingerprints_per_seed() {
    let Some((device, queue)) = tectonics_device() else {
        return;
    };

    let mut pipeline: Option<TectonicsPipeline> = None;
    for (resolution, seed, growth, ownership) in PINNED_FINGERPRINTS {
        if pipeline
            .as_ref()
            .is_none_or(|built| built.resolution() != resolution)
        {
            pipeline = Some(
                TectonicsPipeline::new(&device, resolution, PipelineTuning::default()).unwrap(),
            );
        }
        let pipeline = pipeline.as_ref().expect("the pipeline was just built");
        let config = RasterTectonicsConfig {
            partition: RasterPlatePartitionConfig {
                seed,
                ..RasterPlatePartitionConfig::default()
            },
            ..RasterTectonicsConfig::default()
        };
        pipeline.run(&device, &queue, &config).unwrap();
        let buffers = read_buffers(&device, &queue, pipeline);
        assert_eq!(
            fingerprint(buffers.labels.iter().map(|&label| u64::from(label))),
            growth,
            "growth labels at resolution {resolution} seed {seed}"
        );
        assert_eq!(
            fingerprint(
                buffers
                    .ownership
                    .iter()
                    .map(|&word| u64::from(current_plate(word)))
            ),
            ownership,
            "evolved ownership at resolution {resolution} seed {seed}"
        );
    }
}

/// Records whether this backend contracts a multiply-add, and refuses a result
/// that neither rounding explains.
///
/// Whether the backend fuses is what decides how much cross-backend agreement
/// the float-decided integers can be expected to hold, so slice 5 records this
/// verdict beside its tolerances rather than assuming one.
#[test]
fn multiply_add_rounds_one_of_the_two_admissible_ways() {
    let Some((device, queue)) = tectonics_device() else {
        return;
    };

    let source = r#"
struct FmaInput {
    a: f32,
    b: f32,
    c: f32,
}
@group(0) @binding(0) var<storage, read> inputs: array<FmaInput>;
@group(0) @binding(1) var<storage, read_write> outputs: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= arrayLength(&inputs) { return; }
    let input = inputs[id.x];
    outputs[id.x] = bitcast<u32>(input.a * input.b + input.c);
}
"#;
    let count = FMA_CONTRACTION_TEST_VECTORS.len();
    let operands: Vec<f32> = FMA_CONTRACTION_TEST_VECTORS
        .iter()
        .flat_map(|&(a, b, c, _, _)| [a, b, c])
        .collect();
    let input_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("fma inputs"),
        contents: bytemuck::cast_slice(&operands),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let output_buffer = storage_output_buffer::<u32>(&device, "fma outputs", count);
    run_compute(
        &device,
        &queue,
        "fma contraction probe",
        source,
        &[
            input_buffer.as_entire_binding(),
            output_buffer.as_entire_binding(),
        ],
        count as u32,
    );
    let actual: Vec<u32> = readback(&device, &queue, &output_buffer, count);

    let mut contracted = Vec::new();
    for (&(a, b, c, rounded, fused), &bits) in FMA_CONTRACTION_TEST_VECTORS.iter().zip(&actual) {
        assert_eq!(
            rounded,
            (a * b + c).to_bits(),
            "the host disagrees with its own table for {a} * {b} + {c}"
        );
        assert!(
            bits == rounded || bits == fused,
            "{a} * {b} + {c} produced 0x{bits:08x}, which is neither the rounded \
             0x{rounded:08x} nor the fused 0x{fused:08x}"
        );
        contracted.push(bits == fused);
    }
    assert!(
        contracted.iter().all(|&fused| fused == contracted[0]),
        "the backend contracted some multiply-adds and not others, so no \
         expression order describes it"
    );
    println!(
        "fused multiply-add contraction: {}",
        if contracted[0] { "enabled" } else { "disabled" }
    );
}

fn dispatch_shapes() -> [PipelineTuning; 4] {
    [
        PipelineTuning {
            workgroup_size: 64,
            frontier_chunk: 4,
        },
        PipelineTuning {
            workgroup_size: 64,
            frontier_chunk: 1,
        },
        PipelineTuning {
            workgroup_size: 32,
            frontier_chunk: 7,
        },
        PipelineTuning {
            workgroup_size: 256,
            frontier_chunk: 2,
        },
    ]
}

fn tectonics_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let (adapter_info, device, queue) =
        request_device_with_limits("raster tectonics device", wgpu::Limits::default())?;
    println!(
        "GPU adapter: {} ({:?}, {:?})",
        adapter_info.name, adapter_info.backend, adapter_info.device_type
    );
    Some((device, queue))
}

/// Every buffer a run leaves resident, read back together so one comparison
/// covers the whole pipeline.
#[derive(Debug, PartialEq)]
struct Buffers {
    labels: Vec<u32>,
    ownership: Vec<u32>,
    boundary_classes: Vec<u32>,
    plates: Vec<RasterPlate>,
}

impl Buffers {
    fn seed_cells(&self) -> Vec<u32> {
        self.plates.iter().map(|plate| plate.seed_cell).collect()
    }
}

fn read_buffers(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &TectonicsPipeline,
) -> Buffers {
    let cells = pipeline.cell_count() as usize;
    Buffers {
        labels: readback(device, queue, pipeline.growth_label_buffer(), cells),
        ownership: readback(device, queue, pipeline.ownership_buffer(), cells),
        boundary_classes: readback(device, queue, pipeline.boundary_class_buffer(), cells),
        plates: readback(
            device,
            queue,
            pipeline.plate_buffer(),
            procgen_raster_tectonics::PLATE_ID_COUNT as usize,
        ),
    }
}

/// Recomputes the packed growth labels the kernels must settle on, given the
/// seed cells they chose, and checks each of those choices was the farthest
/// eligible cell at the time it was placed.
fn reference_partition(
    resolution: u32,
    config: &RasterPlatePartitionConfig,
    seed_cells: &[u32],
) -> Vec<u32> {
    let cell_count = FaceTexel::cell_count(resolution).unwrap();
    let head_start = config.head_start_cost(resolution);
    let directions: Vec<Vec3> = (0..cell_count)
        .map(|cell| {
            FaceTexel::from_cell_id(cell, resolution)
                .unwrap()
                .center_direction()
        })
        .collect();
    let mut seed_distance = vec![f32::MAX; cell_count as usize];

    assert_eq!(
        seed_cells[0],
        first_seed_cell(config.seed, cell_count),
        "the first major seed must come from the shared random stream"
    );
    // Mirror the kernels' own view of what is claimed: nothing but the seeds
    // themselves while the majors are placed, then the majors' settled costs.
    let mut labels = vec![UNCLAIMED_LABEL; cell_count as usize];
    let mut placed = Vec::new();
    for (plate, &cell) in seed_cells
        .iter()
        .take(config.plate_count() as usize)
        .enumerate()
    {
        let plate = plate as u32;
        let major = plate < config.major_plate_count;
        let ceiling = if major { 0 } else { head_start };
        if plate == config.major_plate_count {
            labels = shortest_arrival(resolution, config, &placed, cell_count);
        }
        if plate > 0 {
            let eligible = |candidate: u32| growth_label_cost(labels[candidate as usize]) > ceiling;
            assert!(eligible(cell), "plate {plate} seeded an ineligible cell");
            let best = (0..cell_count)
                .filter(|&candidate| eligible(candidate))
                .map(|candidate| seed_distance[candidate as usize])
                .fold(f32::MIN, f32::max);
            let chosen = seed_distance[cell as usize];
            assert!(
                chosen >= best - 1.0e-6 * best.abs(),
                "plate {plate} seeded {cell} at {chosen} rather than the farthest cell at {best}"
            );
        }
        let start_cost = if major { 0 } else { head_start };
        labels[cell as usize] = labels[cell as usize].min(growth_label(start_cost, plate));
        placed.push((cell, plate, start_cost));
        for (candidate, distance) in seed_distance.iter_mut().enumerate() {
            *distance =
                distance.min(directions[candidate].distance_squared(directions[cell as usize]));
        }
    }
    assert!(
        seed_cells[config.plate_count() as usize..]
            .iter()
            .all(|&cell| cell == NO_RASTER_CELL),
        "plate ids past the configured count must stay seedless"
    );
    shortest_arrival(resolution, config, &placed, cell_count)
}

/// Multi-source shortest arrival over the raster's chamfer links, labelled with
/// the packed `(cost, plate)` word the kernels settle on.
fn shortest_arrival(
    resolution: u32,
    config: &RasterPlatePartitionConfig,
    seeds: &[(u32, u32, u32)],
    cell_count: u32,
) -> Vec<u32> {
    let growth_key = fold_growth_key(config.seed);
    let mut labels = vec![UNCLAIMED_LABEL; cell_count as usize];
    let mut settled = vec![false; cell_count as usize];
    let mut arrivals = BinaryHeap::new();
    for &(cell, plate, start_cost) in seeds {
        let label = growth_label(start_cost, plate);
        if label < labels[cell as usize] {
            labels[cell as usize] = label;
            arrivals.push(Reverse((label, cell)));
        }
    }
    while let Some(Reverse((label, cell))) = arrivals.pop() {
        if std::mem::replace(&mut settled[cell as usize], true) {
            continue;
        }
        let texel = FaceTexel::from_cell_id(cell, resolution).unwrap();
        for link in TexelLink::ALL {
            let Some(neighbor) = texel.neighbor(link).map(FaceTexel::cell_id) else {
                continue;
            };
            let candidate = label
                + growth_label(
                    reference_link_cost(link, cell, neighbor, growth_key, config.growth_roughness),
                    0,
                );
            if candidate < labels[neighbor as usize] {
                labels[neighbor as usize] = candidate;
                arrivals.push(Reverse((candidate, neighbor)));
            }
        }
    }
    labels
}

fn reference_link_cost(
    link: TexelLink,
    cell: u32,
    neighbor: u32,
    growth_key: u32,
    roughness: u32,
) -> u32 {
    let offset = hash_u32(
        cell.min(neighbor),
        cell.max(neighbor),
        PLATE_GROWTH_COST as u32,
        growth_key,
    ) % (2 * roughness + 1);
    link.link_length() * (BASE_GROWTH_COST - roughness + offset)
}
