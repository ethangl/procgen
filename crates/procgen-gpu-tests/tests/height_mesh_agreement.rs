use procgen_cubesphere::{CubeFace, TileAddress};
use procgen_gpu_tests::{readback, request_device_for_backends, validate_wgsl};
use procgen_realtime_pilot::*;
#[test]
fn height_compute_and_render_shaders_validate() {
    validate_wgsl("physical height", &height_shader());
    validate_wgsl(
        "physical rendering",
        concat!(
            include_str!("../../../apps/realtime-pilot/src/physical_frame.wgsl"),
            include_str!("../../../apps/realtime-pilot/src/physical_gpu.wgsl"),
        ),
    );
}
#[test]
fn filtered_tiles_match_cpu_and_replay_after_reordered_submissions() {
    let backend = if cfg!(target_os = "macos") {
        wgpu::Backends::METAL
    } else {
        wgpu::Backends::VULKAN
    };
    let (_, device, queue) =
        request_device_for_backends("height mesh", backend).expect("selected GPU backend required");
    for source in [
        include_str!("../../../planet-design.json"),
        include_str!("../../../planet-design-300km.json"),
    ] {
        let json: serde_json::Value = serde_json::from_str(source).unwrap();
        let config: PlanetDesignConfig = serde_json::from_value(json["design"].clone()).unwrap();
        let field = config.validate().unwrap();
        let filter = HeightFilter::new(
            &field,
            VoxelPosition {
                x_m: config.radius_m as i32,
                y_m: 0,
                z_m: 0,
            },
        );
        let mesher = HeightGpuMesher::new(&device, &field);
        let tiles = [
            TileAddress::root(CubeFace::PositiveX),
            TileAddress::new(CubeFace::PositiveX, 8, 128, 128).unwrap(),
            TileAddress::new(CubeFace::PositiveX, 18, 131072, 131072).unwrap(),
            TileAddress::root(CubeFace::NegativeY),
        ];
        let dispatch = |tiles: &[TileAddress]| {
            let mut encoder = device.create_command_encoder(&Default::default());
            let buffers: Vec<_> = tiles
                .iter()
                .map(|&t| mesher.encode(&device, &mut encoder, t, filter))
                .collect();
            queue.submit([encoder.finish()]);
            buffers
                .iter()
                .map(|b| readback::<HeightVertex>(&device, &queue, b, HEIGHT_VERTEX_COUNT as usize))
                .collect::<Vec<_>>()
        };
        let first = dispatch(&tiles);
        let mut reversed = tiles;
        reversed.reverse();
        let replay = dispatch(&reversed);
        let mut max_error = 0.0_f64;
        for (i, &tile) in tiles.iter().enumerate() {
            assert_eq!(
                first[i],
                replay[tiles.len() - 1 - i],
                "tile schedule invariance"
            );
            let cpu = height_tile_vertices(&field, tile, filter);
            for (a, b) in first[i].iter().zip(cpu) {
                for axis in 0..3 {
                    let x = a.anchor[axis] as f64 + a.offset[axis] as f64;
                    let y = b.anchor[axis] as f64 + b.offset[axis] as f64;
                    let error = (x - y).abs();
                    max_error = max_error.max(error);
                    // Direction normalization at planetary magnitude permits four
                    // f32 epsilon units, plus 2 cm for height-field arithmetic.
                    assert!(
                        error <= 0.02 + 4.0 * f32::EPSILON as f64 * config.radius_m as f64,
                        "radius {}, tile {tile:?}, error {error}",
                        config.radius_m
                    );
                }
            }
            for edge in 0..4 {
                for along in 0..HEIGHT_SIDE {
                    let skirt =
                        first[i][(HEIGHT_SIDE * HEIGHT_SIDE + edge * HEIGHT_SIDE + along) as usize];
                    let radius = (0..3)
                        .map(|a| (skirt.anchor[a] as f64 + skirt.offset[a] as f64).powi(2))
                        .sum::<f64>()
                        .sqrt();
                    assert!(radius < config.radius_m as f64 - config.height_limit_m as f64);
                }
            }
        }
        assert!(height_indices().iter().all(|&i| i < HEIGHT_VERTEX_COUNT));
        // Same-level shared vertices agree exactly on one device.
        let adjacent = [
            tiles[1],
            TileAddress::new(CubeFace::PositiveX, 8, 129, 128).unwrap(),
        ];
        let values = dispatch(&adjacent);
        for y in 0..HEIGHT_SIDE {
            assert_eq!(
                values[0][(y * HEIGHT_SIDE + HEIGHT_QUADS) as usize],
                values[1][(y * HEIGHT_SIDE) as usize]
            );
        }
        let mixed = [tiles[1], tiles[1].children().unwrap()[0]];
        let joined = dispatch(&mixed);
        for y in 0..=HEIGHT_QUADS / 2 {
            for x in 0..=HEIGHT_QUADS / 2 {
                let a = joined[0][(y * HEIGHT_SIDE + x) as usize];
                let b = joined[1][(y * 2 * HEIGHT_SIDE + x * 2) as usize];
                assert_eq!(&a.anchor[..3], &b.anchor[..3], "shared coarse/fine anchor");
                assert_eq!(
                    &a.offset[..3],
                    &b.offset[..3],
                    "continuous filter at coarse/fine sample"
                );
            }
        }
        println!(
            "radius {}: max height vertex error {max_error:.6} m",
            config.radius_m
        );
    }
}

#[test]
fn local_voxel_travel_has_bounded_memory_and_reproducible_revisits() {
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let backend = if cfg!(target_os = "macos") {
        wgpu::Backends::METAL
    } else {
        wgpu::Backends::VULKAN
    };
    let (_, device, queue) = procgen_gpu_tests::request_device_with_limits(
        "local voxel travel",
        backend,
        wgpu::Limits {
            max_storage_buffers_per_shader_stage: 8,
            ..wgpu::Limits::downlevel_defaults()
        },
    )
    .expect("selected GPU backend required");
    for source in [
        include_str!("../../../planet-design.json"),
        include_str!("../../../planet-design-300km.json"),
    ] {
        let json: serde_json::Value = serde_json::from_str(source).unwrap();
        let config: PlanetDesignConfig = serde_json::from_value(json["design"].clone()).unwrap();
        let field = Arc::new(config.validate().unwrap());
        let x =
            (config.radius_m + field.elevation_m(procgen_core::Vec3::X, 0.0).unwrap() + 2.0) as i32;
        let mut world =
            VoxelGpuWorld::new(&device, &queue, Arc::clone(&field), LOCAL_GPU_WORLD_CONFIG)
                .unwrap();
        let mut initial = None;
        let mut peak = 0;
        for y in [0, 64, 192, 0, 64, 192, 0] {
            let camera = VoxelPosition {
                x_m: x,
                y_m: y,
                z_m: 0,
            };
            world
                .set_coverage(&local_voxel_coverage(&field, camera).unwrap(), camera)
                .unwrap();
            let start = Instant::now();
            loop {
                for event in world.update().unwrap() {
                    if let VoxelGpuEvent::Completed { outcome, .. } = event {
                        assert!(matches!(outcome, VoxelGpuOutcome::Ready));
                    }
                }
                peak = peak.max(world.memory_bytes());
                assert!(world.memory_bytes() <= LOCAL_GPU_WORLD_CONFIG.memory_budget_bytes);
                if world.settled() {
                    break;
                }
                assert!(start.elapsed() < Duration::from_secs(30));
                std::thread::sleep(Duration::from_millis(1));
            }
            let update_ms = start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(world.resident_slots().count(), 125);
            if y == 0 {
                let mut geometry = Vec::new();
                for (_, key, slot) in world.resident_slots() {
                    let draw = readback::<VoxelMeshDraw>(&device, &queue, &slot.regular.draw, 1)[0];
                    let status =
                        readback::<VoxelMeshStatus>(&device, &queue, &slot.regular.status, 1)[0];
                    let vertices = if status.vertex_count == 0 {
                        Vec::new()
                    } else {
                        readback::<VoxelMeshVertex>(
                            &device,
                            &queue,
                            &slot.regular.vertices,
                            status.vertex_count as usize,
                        )
                    };
                    geometry.push((key.address(), draw.index_count, vertices));
                }
                geometry.sort_by_key(|g| g.0);
                if let Some(reference) = &initial {
                    assert_eq!(
                        &geometry, reference,
                        "deterministic return to same local chunks"
                    );
                } else {
                    initial = Some(geometry);
                }
            }
            println!(
                "radius {} y {y}: local replacement {:.1} ms",
                config.radius_m, update_ms
            );
        }
        println!(
            "radius {}: repeated local travel peak {:.1} MiB",
            config.radius_m,
            peak as f64 / 1048576.0
        );
    }
}
