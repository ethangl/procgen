use procgen_cubesphere::{CubeFace, FaceEdge, TileAddress};
use procgen_gpu_tests::{readback, request_device_for_backends, validate_wgsl};
use procgen_realtime_pilot::*;
#[test]
fn height_compute_shader_validates() {
    validate_wgsl("physical height", &height_shader());
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
        let mesher = HeightGpuMesher::new(&device, &field);
        let tiles = [
            TileAddress::root(CubeFace::PositiveX),
            TileAddress::new(CubeFace::PositiveX, 8, 128, 128).unwrap(),
            TileAddress::new(CubeFace::PositiveX, 18, 131072, 131072).unwrap(),
            TileAddress::root(CubeFace::NegativeY),
        ];
        let dispatch_mesh = |tiles: &[HeightTile]| {
            let mut encoder = device.create_command_encoder(&Default::default());
            let buffers: Vec<_> = tiles
                .chunks(HEIGHT_GPU_BATCH_TILES)
                .flat_map(|batch| mesher.encode_batch(&device, &mut encoder, batch))
                .collect();
            queue.submit([encoder.finish()]);
            buffers
                .iter()
                .map(|b| readback::<HeightVertex>(&device, &queue, b, HEIGHT_VERTEX_COUNT as usize))
                .collect::<Vec<_>>()
        };
        let dispatch = |tiles: &[TileAddress]| {
            dispatch_mesh(
                &tiles
                    .iter()
                    .map(|&t| HeightTile::new(t, []))
                    .collect::<Vec<_>>(),
            )
        };
        let mut audited: Vec<_> = tiles.iter().map(|&t| HeightTile::new(t, [])).collect();
        for tile in [tiles[1], tiles[2]] {
            for mask in 1..16 {
                audited.push(HeightTile::new(
                    tile,
                    FaceEdge::ALL
                        .into_iter()
                        .enumerate()
                        .filter_map(|(i, e)| (mask & (1 << i) != 0).then_some(e)),
                ));
            }
        }
        let first = dispatch_mesh(&audited);
        let mut reversed = audited.clone();
        reversed.reverse();
        let replay = dispatch_mesh(&reversed);
        let mut max_error = 0.0_f64;
        let mut max_normal_error = 0.0_f32;
        for (i, &tile) in audited.iter().enumerate() {
            assert_eq!(
                first[i],
                replay[audited.len() - 1 - i],
                "tile schedule invariance"
            );
            let cpu = height_tile_vertices(&field, tile);
            for (a, b) in first[i].iter().zip(cpu) {
                assert!(a.normal[3].is_finite() && a.normal[3].abs() <= config.height_limit_m);
                let radius = (0..3)
                    .map(|axis| (a.anchor[axis] as f64 + a.offset[axis] as f64).powi(2))
                    .sum::<f64>()
                    .sqrt();
                // The stored color altitude must describe the surface geometry.
                // Retain the position check's planetary direction precision budget.
                assert!(
                    (radius - config.radius_m as f64 - a.normal[3] as f64).abs()
                        <= 0.02 + 4.0 * f32::EPSILON as f64 * config.radius_m as f64
                );
                let n = procgen_core::Vec3::new(a.normal[0], a.normal[1], a.normal[2]);
                let reference = procgen_core::Vec3::new(b.normal[0], b.normal[1], b.normal[2]);
                assert!(n.is_finite() && (n.length() - 1.0).abs() < 0.0001);
                let error = (n - reference).length();
                max_normal_error = max_normal_error.max(error);
                // About three degrees of normal direction for f32 field differences
                // at planetary coordinates; position tolerances remain unchanged.
                assert!(
                    error < 0.05,
                    "radius {}, tile {tile:?}: normal error {error}",
                    config.radius_m
                );
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
        // Cube edges and corners share lighting as well as position.
        let roots = CubeFace::ALL.map(TileAddress::root);
        let faces = dispatch(&roots);
        let mut shared = std::collections::BTreeMap::new();
        let mut matched = 0;
        for face in faces {
            for vertex in &face[..(HEIGHT_SIDE * HEIGHT_SIDE) as usize] {
                let position = [vertex.anchor[0], vertex.anchor[1], vertex.anchor[2]];
                if let Some(previous) = shared.insert(position, *vertex) {
                    assert_eq!(previous.offset, vertex.offset, "cube edge position");
                    assert_eq!(previous.normal, vertex.normal, "cube edge normal");
                    matched += 1;
                }
            }
        }
        assert!(matched > 12 * HEIGHT_QUADS, "all cube edges exercised");
        // Every face orientation, both halves of every edge, and reversed cube
        // seams must use the actual coarse mesh vertices and straight segments.
        for face in CubeFace::ALL {
            for edge in FaceEdge::ALL {
                for along in [0, 1] {
                    let (x, y) = match edge {
                        FaceEdge::Left => (0, along),
                        FaceEdge::Right => (255, along),
                        FaceEdge::Bottom => (along, 0),
                        FaceEdge::Top => (along, 255),
                    };
                    let fine = TileAddress::new(face, 8, x, y).unwrap();
                    let coarse = fine.edge_neighbor(edge).parent().unwrap();
                    let meshes = dispatch_mesh(&[
                        HeightTile::new(fine, [edge]),
                        HeightTile::new(coarse, []),
                    ]);
                    let surface = |v: &HeightVertex| {
                        (v.anchor[..3].to_vec(), v.offset[..3].to_vec(), v.normal)
                    };
                    let mut previous = None;
                    for i in 0..HEIGHT_SIDE {
                        let index = match edge {
                            FaceEdge::Left => i * HEIGHT_SIDE,
                            FaceEdge::Right => i * HEIGHT_SIDE + HEIGHT_QUADS,
                            FaceEdge::Bottom => i,
                            FaceEdge::Top => HEIGHT_QUADS * HEIGHT_SIDE + i,
                        };
                        let p = surface(&meshes[0][index as usize]);
                        assert!(
                            meshes[1].iter().any(|v| surface(v) == p),
                            "{face:?} {edge:?} sample {i} must coincide with coarse surface"
                        );
                        if i % 2 == 1 {
                            assert_eq!(
                                previous.as_ref(),
                                Some(&p),
                                "redundant edge sample collapses"
                            );
                        }
                        previous = Some(p);
                    }
                }
            }
        }
        println!(
            "radius {}: max height vertex error {max_error:.6} m, normal error {max_normal_error:.6}",
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
