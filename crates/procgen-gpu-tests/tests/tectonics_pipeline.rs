//! Determinism and fixed-point tests for the raster tectonics plate partition.
//!
//! The expected ownership at small resolution is computed here by a plain
//! shortest-path search over the same link rule. That is test scaffolding for a
//! GPU-only stage, not a CPU implementation of it.

use procgen_core::{Vec3, fingerprint, hash_u32, random_streams::PLATE_GROWTH_COST};
use procgen_cubesphere::{FaceTexel, TexelLink};
use procgen_gpu_tests::{readback, request_device_with_limits, validate_wgsl};
use procgen_raster_tectonics::{
    BASE_GROWTH_COST, PipelineTuning, PlatePartitionPipeline, RasterPlatePartitionConfig,
    UNCLAIMED_LABEL, first_seed_cell, fold_growth_key, growth_label, growth_label_cost,
    growth_label_plate, partition_kernel_source,
};
use std::{cmp::Reverse, collections::BinaryHeap};

/// Resolution the reference search runs at: small enough for a host Dijkstra,
/// large enough that every seam and cube corner carries plate boundaries.
const REFERENCE_RESOLUTION: u32 = 16;
/// Resolution the determinism tests run at.
const INVARIANCE_RESOLUTION: u32 = 32;

/// Head start large enough at [`REFERENCE_RESOLUTION`] to leave the majors
/// several texels of growth before the minors are seeded.
const REFERENCE_HEAD_START_ARC: f32 = 0.5;

fn reference_config() -> RasterPlatePartitionConfig {
    RasterPlatePartitionConfig {
        major_plate_count: 3,
        minor_plate_count: 5,
        major_head_start_arc: REFERENCE_HEAD_START_ARC,
        ..RasterPlatePartitionConfig::default()
    }
}

#[test]
fn partition_kernels_validate_without_a_device() {
    for tuning in dispatch_shapes() {
        validate_wgsl("raster plate partition", &partition_kernel_source(tuning));
    }
}

#[test]
fn partition_is_bit_identical_run_to_run_and_across_dispatch_shapes() {
    let Some((device, queue)) = partition_device() else {
        return;
    };

    let config = RasterPlatePartitionConfig {
        major_plate_count: 4,
        minor_plate_count: 8,
        seed: 19,
        ..RasterPlatePartitionConfig::default()
    };
    let mut expected: Option<(Vec<u32>, Vec<u32>)> = None;
    for tuning in dispatch_shapes() {
        let pipeline = PlatePartitionPipeline::new(&device, INVARIANCE_RESOLUTION, tuning).unwrap();
        for run in 0..2 {
            let outcome = pipeline.run(&device, &queue, &config).unwrap();
            assert!(
                outcome.settled(),
                "the relaxation exhausted its pass budget at {tuning:?}"
            );
            let actual = read_partition(&device, &queue, &pipeline, &config);
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
    let Some((device, queue)) = partition_device() else {
        return;
    };

    let pipeline =
        PlatePartitionPipeline::new(&device, REFERENCE_RESOLUTION, PipelineTuning::default())
            .unwrap();
    for seed in [0, 1, 7, 4_242] {
        let config = RasterPlatePartitionConfig {
            seed,
            ..reference_config()
        };
        let outcome = pipeline.run(&device, &queue, &config).unwrap();
        assert!(outcome.settled(), "seed {seed} exhausted its pass budget");
        let (labels, seed_cells) = read_partition(&device, &queue, &pipeline, &config);

        assert!(
            labels
                .iter()
                .all(|&label| growth_label_plate(label) < config.plate_count()),
            "seed {seed} left cells unclaimed"
        );
        assert_eq!(
            labels,
            reference_partition(REFERENCE_RESOLUTION, &config, &seed_cells),
            "seed {seed} diverged from the reference search"
        );
    }
}

/// Growth-label fingerprints of the default configuration, pinned per seed and
/// face resolution. They must match on both development machines.
const PINNED_FINGERPRINTS: [(u32, u64, u64); 6] = [
    (128, 0, 7_659_683_194_768_066_236),
    (128, 1, 10_950_105_160_290_849_917),
    (128, 4_242, 222_223_991_584_038_427),
    (256, 0, 16_912_624_269_309_866_239),
    (256, 1, 4_257_014_726_406_016_968),
    (256, 4_242, 17_100_062_096_016_943_217),
];

#[test]
fn default_partition_has_pinned_fingerprints_per_seed() {
    let Some((device, queue)) = partition_device() else {
        return;
    };

    let mut pipeline: Option<PlatePartitionPipeline> = None;
    for (resolution, seed, expected) in PINNED_FINGERPRINTS {
        if pipeline
            .as_ref()
            .is_none_or(|built| built.resolution() != resolution)
        {
            pipeline = Some(
                PlatePartitionPipeline::new(&device, resolution, PipelineTuning::default())
                    .unwrap(),
            );
        }
        let pipeline = pipeline.as_ref().expect("the pipeline was just built");
        let config = RasterPlatePartitionConfig {
            seed,
            ..RasterPlatePartitionConfig::default()
        };
        pipeline.run(&device, &queue, &config).unwrap();
        let labels: Vec<u32> = readback(
            &device,
            &queue,
            pipeline.growth_label_buffer(),
            pipeline.cell_count() as usize,
        );
        assert_eq!(
            fingerprint(labels.iter().map(|&label| u64::from(label))),
            expected,
            "resolution {resolution} seed {seed}"
        );
    }
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

fn partition_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let (adapter_info, device, queue) =
        request_device_with_limits("raster tectonics device", wgpu::Limits::default())?;
    println!(
        "GPU adapter: {} ({:?}, {:?})",
        adapter_info.name, adapter_info.backend, adapter_info.device_type
    );
    Some((device, queue))
}

fn read_partition(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &PlatePartitionPipeline,
    config: &RasterPlatePartitionConfig,
) -> (Vec<u32>, Vec<u32>) {
    (
        readback(
            device,
            queue,
            pipeline.growth_label_buffer(),
            pipeline.cell_count() as usize,
        ),
        readback(
            device,
            queue,
            pipeline.plate_seed_buffer(),
            config.plate_count() as usize,
        ),
    )
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
    for (plate, &cell) in seed_cells.iter().enumerate() {
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
