use procgen_cubesphere::{CubeFace, FaceEdge, TileAddress};
use procgen_gpu_tests::{readback, readback_many, request_device_for_backends, validate_wgsl};
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
    // Each preset runs twice, with the volume term off and on. The tiles now
    // draw the projected surface, so the CPU and the shader have to agree on
    // the projection as well as on the height stack, at the same tolerances and
    // including the shared-edge exactness checks. Both presets carry the starter
    // block's values; only `enabled` moves here.
    for (source, volume) in [
        (include_str!("../../../planet-design.json"), false),
        (include_str!("../../../planet-design.json"), true),
        (include_str!("../../../planet-design-300km.json"), false),
        (include_str!("../../../planet-design-300km.json"), true),
    ] {
        let json: serde_json::Value = serde_json::from_str(source).unwrap();
        let mut config: PlanetDesignConfig =
            serde_json::from_value(json["design"].clone()).unwrap();
        assert_eq!(
            (
                config.volume.wavelength_m,
                config.volume.amplitude_m,
                config.volume.sharpness,
                config.volume.fade_m,
            ),
            (24.0, 6.0, 0.5, 12.0),
            "both presets carry the starter volume values"
        );
        config.volume.enabled = volume;
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
                // The stored altitude is the projected surface, which can stand
                // off the bounded height by up to the term's amplitude.
                assert!(
                    a.normal[3].is_finite()
                        && a.normal[3].abs()
                            <= config.height_limit_m + config.volume.active_amplitude_m()
                );
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
            "radius {}, volume {volume}: max height vertex error {max_error:.6} m, normal error {max_normal_error:.6}",
            config.radius_m
        );
    }
}

#[test]
fn local_voxel_band_steps_with_bounded_memory_and_reproducible_revisits() {
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
        "local voxel band",
        backend,
        wgpu::Limits {
            max_storage_buffers_per_shader_stage: 8,
            ..wgpu::Limits::downlevel_defaults()
        },
    )
    .expect("selected GPU backend required");
    // The 300 km preset only: this is the viewer's band, and the viewer loads
    // this design. The 4,900 km preset costs about as much again to walk and
    // adds no band behavior of its own; large radii keep their coverage in the
    // selection unit tests and in the streaming audit, which both use it.
    {
        let json: serde_json::Value =
            serde_json::from_str(include_str!("../../../planet-design-300km.json")).unwrap();
        let config: PlanetDesignConfig = serde_json::from_value(json["design"].clone()).unwrap();
        let field = Arc::new(config.validate().unwrap());
        let ground = config.radius_m + field.elevation_m(procgen_core::Vec3::X, 0.0).unwrap();
        let mut world =
            VoxelGpuWorld::new(&device, &queue, Arc::clone(&field), LOCAL_GPU_WORLD_CONFIG)
                .unwrap();
        // The coarse start the viewer's worker also uses. It walks whole
        // balanced partitions and hands the world the band each one draws,
        // because a coverage step only redivides the volume it already holds.
        let mut current = VoxelCoverage::new(vec![VoxelChunkAddress::root()]).unwrap();
        let mut band = cap_voxel_coverage(&current, VOXEL_BAND_MAX_SPACING_M).unwrap();
        let mut initial = None;
        let mut peak = 0;
        let mut steps = 0;
        let mut replacements = 0;
        // Three legs. Lateral travel makes the band leave ground it already
        // held, which the coverage walk has to admit rather than only
        // subdivide; the 192 m leg is the one that grows the band past a
        // thousand chunks and caught the transition-capacity overflow. The
        // third returns to the start so the first and third can be compared.
        for (y, clearance_m) in [(0, 2.0), (192, 2.0), (0, 2.0)] {
            let camera = VoxelPosition {
                x_m: (ground + clearance_m) as i32,
                y_m: y,
                z_m: 0,
            };
            let target = select_voxel_coverage(
                &field,
                camera,
                VOXEL_BAND_REQUESTED_LEAVES,
                MAX_VOXEL_COVERAGE_LEAVES,
            )
            .unwrap();
            let target_band = select_voxel_band(
                &field,
                camera,
                VOXEL_BAND_MAX_SPACING_M,
                VOXEL_BAND_REQUESTED_LEAVES,
            )
            .unwrap();
            let start = Instant::now();
            // One coverage step per settled world, and one closed replacement
            // group per step that moves the band, as the worker does.
            loop {
                if world.settled() {
                    if current.leaves() == target.leaves() {
                        break;
                    }
                    current = current.step_toward(&target, camera).unwrap();
                    let next = cap_voxel_coverage(&current, VOXEL_BAND_MAX_SPACING_M).unwrap();
                    if next.leaves() != band.leaves() {
                        band = next;
                        world.set_coverage(&band, camera).unwrap();
                        replacements += 1;
                    }
                    steps += 1;
                }
                for event in world.update().unwrap() {
                    if let VoxelGpuEvent::Completed { outcome, .. } = event {
                        assert!(matches!(outcome, VoxelGpuOutcome::Ready));
                    }
                }
                peak = peak.max(world.memory_bytes());
                assert!(world.memory_bytes() <= LOCAL_GPU_WORLD_CONFIG.memory_budget_bytes);
                assert!(start.elapsed() < Duration::from_secs(600));
            }
            let update_ms = start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(band.leaves(), target_band.leaves());
            assert_eq!(world.resident_slots().count(), band.leaves().len());
            let leases = world.resident_leases();
            assert_eq!(
                leases.len(),
                target_band.leaves().len(),
                "one draw lease per covered leaf"
            );
            let spacings: std::collections::BTreeSet<_> =
                leases.iter().map(|l| l.key.address().spacing_m()).collect();
            assert!(
                spacings.iter().all(|&m| m <= VOXEL_BAND_MAX_SPACING_M),
                "{spacings:?} exceeds the band's spacing cap"
            );
            assert!(
                spacings.len() >= 2,
                "the band must mix spacings, not {spacings:?}"
            );
            if initial.is_none() {
                // The viewer draws these buffers directly, so every chunk the
                // surface passes through must report triangles to draw.
                let mut crossing = 0;
                let draws = readback_many::<VoxelMeshDraw>(
                    &device,
                    &queue,
                    &leases
                        .iter()
                        .map(|l| (&l.slot.regular.draw, 1))
                        .collect::<Vec<_>>(),
                );
                for (lease, draw) in leases.iter().zip(&draws) {
                    if !chunk_spans_surface(&field, lease.key.address()) {
                        continue;
                    }
                    crossing += 1;
                    assert!(
                        draw[0].index_count > 0,
                        "{:?} spans the surface with no indices",
                        lease.key.address()
                    );
                }
                assert!(crossing > 0, "the ground region must cross the surface");
                println!(
                    "radius {}: {crossing} of {} band chunks cross the surface",
                    config.radius_m,
                    leases.len()
                );
            }
            if (y, clearance_m) == (0, 2.0) {
                // Three batched round trips for the whole band, not three per
                // chunk: the counts come back first, then the vertices they size.
                let slots: Vec<_> = world
                    .resident_slots()
                    .map(|(_, key, slot)| (key.address(), slot))
                    .collect();
                let draws = readback_many::<VoxelMeshDraw>(
                    &device,
                    &queue,
                    &slots
                        .iter()
                        .map(|(_, s)| (&s.regular.draw, 1))
                        .collect::<Vec<_>>(),
                );
                let statuses = readback_many::<VoxelMeshStatus>(
                    &device,
                    &queue,
                    &slots
                        .iter()
                        .map(|(_, s)| (&s.regular.status, 1))
                        .collect::<Vec<_>>(),
                );
                let vertices = readback_many::<VoxelMeshVertex>(
                    &device,
                    &queue,
                    &slots
                        .iter()
                        .zip(&statuses)
                        .map(|((_, s), status)| {
                            (&s.regular.vertices, status[0].vertex_count as usize)
                        })
                        .collect::<Vec<_>>(),
                );
                let mut geometry: Vec<_> = slots
                    .iter()
                    .zip(&draws)
                    .zip(vertices)
                    .map(|(((address, _), draw), vertices)| {
                        (*address, draw[0].index_count, vertices)
                    })
                    .collect();
                geometry.sort_by_key(|g| g.0);
                if let Some(reference) = &initial {
                    assert_eq!(
                        &geometry, reference,
                        "deterministic return to the same band"
                    );
                } else {
                    initial = Some(geometry);
                }
            }
            println!(
                "radius {} y {y} clearance {clearance_m} m: {} band leaves of {} selected, spacings {spacings:?}, {:.1} ms",
                config.radius_m,
                band.leaves().len(),
                current.leaves().len(),
                update_ms
            );
        }
        println!(
            "radius {}: {steps} coverage steps, {replacements} GPU replacements, peak {:.1} MiB",
            config.radius_m,
            peak as f64 / 1048576.0
        );
    }
}

/// True when the density sign changes across a chunk's corners, so marching
/// tetrahedra must emit triangles inside it. One meter of slack absorbs the
/// difference between this direct height query and the shader's mirrored
/// arithmetic, which `voxel_density_agreement` pins far more tightly.
/// Whether the extracted surface has to pass through this chunk, judged at the
/// eight box corners. The canonical CPU potential is the definition of that
/// surface, so it is the right oracle here: a height-only estimate stopped
/// predicting the mesh once the density field gained its volumetric term, which
/// can move the crossing by several meters either way. CPU/GPU agreement on the
/// potential itself is established independently in voxel_density_agreement.
fn chunk_spans_surface(field: &PlanetDesignField, address: VoxelChunkAddress) -> bool {
    let origin = address.origin();
    let span = address.span_m();
    let mut low = f32::MAX;
    let mut high = f32::MIN;
    for corner in 0..8 {
        // Positive is solid, as the density kernel defines it.
        let potential = sample_voxel_potential(
            field,
            VoxelPosition {
                x_m: origin.x_m + (corner & 1) * span,
                y_m: origin.y_m + ((corner >> 1) & 1) * span,
                z_m: origin.z_m + ((corner >> 2) & 1) * span,
            },
        );
        low = low.min(potential);
        high = high.max(potential);
    }
    low < -1.0 && high > 1.0
}
