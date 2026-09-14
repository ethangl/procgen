use procgen_core::Vec3;
use procgen_gpu_tests::{readback, request_device_with_limits, validate_wgsl};
use procgen_realtime_pilot::*;
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

const CONFIG: VoxelGpuMeshConfig = VoxelGpuMeshConfig {
    regular: VoxelMeshConfig {
        vertex_capacity: 50_000,
        triangle_capacity: 100_000,
    },
    transition: VoxelTransitionConfig {
        triangle_capacity: 32_768,
    },
};
#[test]
fn voxel_transition_wgsl_validates_without_a_device() {
    validate_wgsl("voxel transition", &voxel_transition_shader());
    validate_wgsl(
        "voxel direct rendering",
        include_str!("../../../apps/realtime-pilot/src/physical_gpu.wgsl"),
    );
}
fn position(x: i32, y: i32, z: i32) -> VoxelPosition {
    VoxelPosition {
        x_m: x,
        y_m: y,
        z_m: z,
    }
}
fn parent(p: VoxelPosition) -> VoxelChunkAddress {
    VoxelChunkAddress::containing(p, 2).unwrap()
}
fn partition(root: VoxelChunkAddress, refine: usize) -> VoxelCoverage {
    let mut leaves = root.children().unwrap().to_vec();
    let child = leaves.remove(refine);
    leaves.extend(child.children().unwrap());
    VoxelCoverage::new(leaves).unwrap()
}
fn sample_index(i: usize) -> VoxelSampleIndex {
    VoxelSampleIndex {
        x: (i % VOXEL_SAMPLE_SIDE) as i32 - VOXEL_HALO,
        y: (i / VOXEL_SAMPLE_SIDE % VOXEL_SAMPLE_SIDE) as i32 - VOXEL_HALO,
        z: (i / VOXEL_SAMPLE_SIDE.pow(2)) as i32 - VOXEL_HALO,
    }
}
fn fixture(a: VoxelChunkAddress, f: impl Fn(VoxelPosition) -> f32) -> VoxelVolume {
    VoxelVolume::from_potentials(
        a,
        (0..VOXEL_SAMPLE_COUNT)
            .map(|i| f(a.sample_position(sample_index(i))))
            .collect(),
    )
    .unwrap()
}
fn device() -> (wgpu::Device, wgpu::Queue) {
    let backend = if cfg!(target_os = "macos") {
        wgpu::Backends::METAL
    } else {
        wgpu::Backends::VULKAN
    };
    let (adapter, device, queue) = request_device_with_limits(
        "voxel streaming audit",
        backend,
        wgpu::Limits {
            max_storage_buffers_per_shader_stage: 8,
            ..wgpu::Limits::downlevel_defaults()
        },
    )
    .expect("GPU streaming audit requires the selected backend");
    println!(
        "streaming adapter: {} / {:?}",
        adapter.name, adapter.backend
    );
    (device, queue)
}
fn mesh(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    address: VoxelChunkAddress,
    buffers: &VoxelGpuMeshBuffers,
) -> VoxelChunkMesh {
    let status = readback::<VoxelMeshStatus>(device, queue, &buffers.status, 1)[0];
    assert_eq!(status.overflow, 0, "chunk {address:?}");
    let draw = readback::<VoxelMeshDraw>(device, queue, &buffers.draw, 1)[0];
    assert_eq!(draw.index_count, status.index_count);
    let vertices = if status.vertex_count == 0 {
        vec![]
    } else {
        readback(
            device,
            queue,
            &buffers.vertices,
            status.vertex_count as usize,
        )
    };
    let indices = if status.index_count == 0 {
        vec![]
    } else {
        readback(device, queue, &buffers.indices, status.index_count as usize)
    };
    VoxelChunkMesh::from_parts(address, vertices, indices).unwrap()
}
fn compare(a: &VoxelChunkMesh, b: &VoxelChunkMesh) {
    assert_eq!(a.indices(), b.indices());
    assert_eq!(a.vertices().len(), b.vertices().len());
    let spacing = a.address().spacing_m() as f32;
    for (a, b) in a.vertices().iter().zip(b.vertices()) {
        assert_eq!(a.anchor_m, b.anchor_m);
        for axis in 0..3 {
            assert!(
                (a.offset_m[axis] - b.offset_m[axis]).abs() <= 0.00001 * spacing,
                "interpolation mismatch"
            );
        }
    }
}
#[derive(Clone, Copy, PartialEq, Debug)]
struct Point([f64; 3]);
impl Eq for Point {}
impl PartialOrd for Point {
    fn partial_cmp(&self, rhs: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(rhs))
    }
}
impl Ord for Point {
    fn cmp(&self, rhs: &Self) -> std::cmp::Ordering {
        self.0
            .iter()
            .zip(rhs.0)
            .map(|(a, b)| a.total_cmp(&b))
            .find(|o| !o.is_eq())
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}
fn check_edges(meshes: &[VoxelChunkMesh], support: VoxelChunkAddress, closed: bool) {
    let mut edges = BTreeMap::<[Point; 2], (u32, i32)>::new();
    for mesh in meshes {
        let origin = mesh.address().origin();
        let points: Vec<_> = mesh
            .vertices()
            .iter()
            .map(|v| {
                Point(std::array::from_fn(|i| {
                    [origin.x_m, origin.y_m, origin.z_m][i] as f64
                        + v.anchor_m[i] as f64
                        + v.offset_m[i] as f64
                }))
            })
            .collect();
        for t in mesh.indices().chunks_exact(3) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (a, b) = (points[a as usize], points[b as usize]);
                assert_ne!(a, b);
                let entry = edges.entry([a.min(b), a.max(b)]).or_default();
                entry.0 += 1;
                entry.1 += if a < b { 1 } else { -1 };
            }
        }
    }
    let origin = support.origin();
    let lo = [origin.x_m, origin.y_m, origin.z_m];
    for ([a, b], counts) in edges {
        if counts.0 == 1 && !closed {
            assert!(
                (0..3).any(|i| [lo[i] as f64, (lo[i] + support.span_m()) as f64]
                    .into_iter()
                    .any(|v| a.0[i] == v && b.0[i] == v)),
                "internal GPU seam {a:?} {b:?}"
            );
        } else {
            assert_eq!(counts, (2, 0), "GPU edge {a:?} {b:?}");
        }
    }
}

#[test]
fn gpu_transitions_match_cpu_close_seams_and_reject_overflow() {
    let (device, queue) = device();
    let mesher = VoxelGpuMesher::new(&device, &queue);
    let root = parent(position(0, 0, 0));
    for case in 0..6 {
        let support = if case == 5 {
            VoxelChunkAddress::containing(position(0, 0, 0), 3).unwrap()
        } else {
            root
        };
        let coverage = if case == 5 {
            let mut leaves = support.children().unwrap().to_vec();
            let mut fine = leaves.remove(0).children().unwrap().to_vec();
            let finest = fine.remove(1).children().unwrap();
            leaves.extend(fine);
            leaves.extend(finest);
            VoxelCoverage::balanced(leaves, 256).unwrap()
        } else {
            partition(root, if case == 1 { 7 } else { 0 })
        };
        let f = |p: VoxelPosition| match case {
            0 | 1 => {
                50.0 - Vec3::new(
                    (p.x_m - 64) as f32,
                    (p.y_m - 64) as f32,
                    (p.z_m - 64) as f32,
                )
                .length()
            }
            2 => 64.0 - p.x_m as f32,
            3 => 64.125 - (p.x_m + p.y_m + p.z_m) as f32,
            4 => {
                0.6 - Vec3::new(
                    (p.x_m - 64) as f32,
                    (p.y_m - 31) as f32,
                    (p.z_m - 31) as f32,
                )
                .length()
            }
            _ => {
                100.0
                    - Vec3::new(
                        (p.x_m - 128) as f32,
                        (p.y_m - 128) as f32,
                        (p.z_m - 128) as f32,
                    )
                    .length()
            }
        };
        let mut meshes = Vec::new();
        let mut transition_triangles = 0;
        for key in coverage.keys() {
            let plan = VoxelTransitionPlan::new(key);
            let volume = fixture(plan.key().address(), f);
            let samples = VoxelTransitionSamples::new(
                plan.sample_points().iter().copied().map(f).collect(),
                &plan,
            )
            .unwrap();
            let slot = VoxelGpuSlot::new(&device, CONFIG).unwrap();
            let work = mesher
                .prepare(
                    &plan,
                    VoxelGpuInput::Samples {
                        volume: &volume,
                        transition: &samples,
                    },
                    &slot,
                    CONFIG,
                )
                .unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            mesher.encode(&mut encoder, &work, &slot);
            queue.submit([encoder.finish()]);
            let regular = mesh(&device, &queue, plan.key().address(), &slot.regular);
            let transition = mesh(&device, &queue, plan.key().address(), &slot.transition);
            compare(&build_voxel_regular_mesh(&volume, &plan).unwrap(), &regular);
            compare(
                &build_voxel_transition_mesh(&plan, &samples).unwrap(),
                &transition,
            );
            transition_triangles += transition.indices().len() / 3;
            meshes.extend([regular, transition]);
        }
        assert!(transition_triangles > 0);
        check_edges(&meshes, support, !(2..4).contains(&case));
        println!(
            "GPU transition case {case}: {transition_triangles} transition triangles; closed internal seams"
        );
    }
    let coverage = partition(root, 0);
    let plan = coverage
        .keys()
        .map(VoxelTransitionPlan::new)
        .find(|p| p.replaced_cells() > 0)
        .unwrap();
    let f = |p: VoxelPosition| 64.125 - (p.x_m + p.y_m + p.z_m) as f32;
    let volume = fixture(plan.key().address(), f);
    let samples =
        VoxelTransitionSamples::new(plan.sample_points().iter().copied().map(f).collect(), &plan)
            .unwrap();
    let config = VoxelGpuMeshConfig {
        transition: VoxelTransitionConfig {
            triangle_capacity: 1,
        },
        ..CONFIG
    };
    let slot = VoxelGpuSlot::new(&device, config).unwrap();
    let sentinel = vec![
        VoxelMeshVertex {
            anchor_m: [-17; 4],
            offset_m: [-17.0; 4]
        };
        3
    ];
    queue.write_buffer(
        &slot.transition.vertices,
        0,
        bytemuck::cast_slice(&sentinel),
    );
    queue.write_buffer(
        &slot.transition.indices,
        0,
        bytemuck::cast_slice(&[0xdeadbeefu32; 3]),
    );
    let work = mesher
        .prepare(
            &plan,
            VoxelGpuInput::Samples {
                volume: &volume,
                transition: &samples,
            },
            &slot,
            config,
        )
        .unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    mesher.encode(&mut encoder, &work, &slot);
    queue.submit([encoder.finish()]);
    let status = readback::<VoxelMeshStatus>(&device, &queue, &slot.transition.status, 1)[0];
    assert_eq!(status.overflow, 1);
    assert_eq!(
        readback::<VoxelMeshVertex>(&device, &queue, &slot.transition.vertices, 3),
        sentinel
    );
    assert_eq!(
        readback::<u32>(&device, &queue, &slot.transition.indices, 3),
        vec![0xdeadbeef; 3]
    );
    assert_eq!(
        readback::<VoxelMeshDraw>(&device, &queue, &slot.transition.draw, 1)[0].index_count,
        0
    );
}

#[derive(Clone)]
struct Snapshot {
    regular_vertices: Vec<VoxelMeshVertex>,
    regular_indices: Vec<u32>,
    transition_vertices: Vec<VoxelMeshVertex>,
    transition_indices: Vec<u32>,
}
fn snapshot(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &VoxelGpuWorld,
    support: VoxelChunkAddress,
) -> BTreeMap<VoxelMeshKey, Snapshot> {
    let mut result = BTreeMap::new();
    let mut meshes = Vec::new();
    for (_, key, slot) in world.resident_slots() {
        let regular = mesh(device, queue, key.address(), &slot.regular);
        let transition = mesh(device, queue, key.address(), &slot.transition);
        result.insert(
            key.clone(),
            Snapshot {
                regular_vertices: regular.vertices().to_vec(),
                regular_indices: regular.indices().to_vec(),
                transition_vertices: transition.vertices().to_vec(),
                transition_indices: transition.indices().to_vec(),
            },
        );
        meshes.extend([regular, transition]);
    }
    if !meshes.is_empty() {
        check_edges(&meshes, support, false);
    }
    result
}
#[derive(Default)]
struct Run {
    submitted: usize,
    first_publication_ms: f64,
    settled_ms: f64,
    preparation_ms: f64,
    encoding_ms: f64,
    peak_bytes: u64,
}
fn settle(world: &mut VoxelGpuWorld) -> Run {
    let start = Instant::now();
    let mut run = Run::default();
    loop {
        for event in world.update().unwrap() {
            match event {
                VoxelGpuEvent::Submitted {
                    preparation_ms,
                    encoding_ms,
                    ..
                } => {
                    run.submitted += 1;
                    run.preparation_ms += preparation_ms;
                    run.encoding_ms += encoding_ms;
                }
                VoxelGpuEvent::Completed {
                    outcome, gpu_times, ..
                } => {
                    assert!(matches!(outcome, VoxelGpuOutcome::Ready));
                    if let Some(times) = gpu_times {
                        assert!(times.density_ms > 0.0 && times.density_ms.is_finite());
                        assert!(times.extraction_ms > 0.0 && times.extraction_ms.is_finite());
                    }
                }
                VoxelGpuEvent::Published(p) => {
                    if !p.installed.is_empty() && run.first_publication_ms == 0.0 {
                        run.first_publication_ms = start.elapsed().as_secs_f64() * 1000.0;
                    }
                }
            }
        }
        run.peak_bytes = run.peak_bytes.max(world.memory_bytes());
        if world.settled() {
            run.settled_ms = start.elapsed().as_secs_f64() * 1000.0;
            return run;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "GPU residency stalled"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn saved_terrain_stream_reuses_retires_cancels_and_reproduces_slots() {
    let (device, queue) = device();
    let json: serde_json::Value =
        serde_json::from_str(include_str!("../../../planet-design.json")).unwrap();
    let design: PlanetDesignConfig = serde_json::from_value(json["design"].clone()).unwrap();
    let field = Arc::new(design.validate().unwrap());
    let config = VoxelGpuWorldConfig {
        stream: VoxelStreamConfig {
            max_slots: 128,
            max_in_flight: 2,
        },
        mesh: VoxelGpuMeshConfig {
            regular: VoxelMeshConfig {
                vertex_capacity: 30_000,
                triangle_capacity: 60_000,
            },
            transition: VoxelTransitionConfig {
                triangle_capacity: 16_384,
            },
        },
        memory_budget_bytes: 512 * 1024 * 1024,
    };
    let mut world = VoxelGpuWorld::new(&device, &queue, field.clone(), config).unwrap();
    let root = VoxelChunkAddress::containing(position(4_903_255, 16, 16), 3).unwrap();
    let patch = |focus: VoxelPosition| {
        let mut leaves: Vec<_> = root
            .children()
            .unwrap()
            .into_iter()
            .flat_map(|c| c.children().unwrap())
            .collect();
        let fine = VoxelChunkAddress::containing(focus, 1).unwrap();
        let index = leaves.iter().position(|&a| a == fine).unwrap();
        leaves.remove(index);
        leaves.extend(fine.children().unwrap());
        VoxelCoverage::new(leaves).unwrap()
    };
    let a = patch(position(4_903_255, 16, 16));
    let b = patch(position(4_903_255, 80, 16));
    world.set_coverage(&a, position(4_903_255, 16, 16)).unwrap();
    let cold = settle(&mut world);
    assert_eq!(cold.submitted, a.leaves().len());
    let first = snapshot(&device, &queue, &world, root);
    let tickets: BTreeMap<_, _> = world
        .resident_slots()
        .map(|(t, k, _)| (k.clone(), t))
        .collect();
    world.set_coverage(&a, position(4_903_255, 32, 16)).unwrap();
    let small = settle(&mut world);
    assert_eq!(small.submitted, 0);
    assert_eq!(
        tickets,
        world
            .resident_slots()
            .map(|(t, k, _)| (k.clone(), t))
            .collect()
    );
    // Simulate a renderer submission that reads a resident vertex buffer. Its
    // completion must participate in retirement, just like an actual draw.
    let read_source = &world.resident_slots().next().unwrap().2.regular.vertices;
    let sink = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("draw lifetime probe"),
        size: 32,
        usage: wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(read_source, 0, &sink, 0, 32);
    queue.submit([encoder.finish()]);
    world.track_draw_submission();
    let drawn = world.resident_leases();
    world.set_coverage(&b, position(4_903_255, 80, 16)).unwrap();
    let moved = settle(&mut world);
    let second = snapshot(&device, &queue, &world, root);
    assert!(
        world.retiring() > 0,
        "draw leases must prevent slot reuse after GPU completion"
    );
    for lease in &drawn {
        let old = &first[&lease.key];
        if old.regular_vertices.is_empty() {
            continue;
        }
        assert_eq!(
            readback::<VoxelMeshVertex>(
                &device,
                &queue,
                &lease.slot.regular.vertices,
                old.regular_vertices.len()
            ),
            old.regular_vertices
        );
    }
    drop(drawn);
    world.update().unwrap();
    assert_eq!(world.retiring(), 0);
    assert!(
        moved.submitted > 0 && moved.submitted < b.leaves().len(),
        "{} of {} chunks changed",
        moved.submitted,
        b.leaves().len()
    );
    for (ticket, key, _) in world.resident_slots() {
        if let Some(&old) = tickets.get(key) {
            assert_eq!(ticket, old);
        }
    }
    assert_eq!(second.len(), b.leaves().len());
    world.set_coverage(&a, position(4_903_255, 16, 16)).unwrap();
    let revisit = settle(&mut world);
    let replay = snapshot(&device, &queue, &world, root);
    assert_eq!(first.len(), replay.len());
    for (key, old) in &first {
        let new = &replay[key];
        assert_eq!(old.regular_vertices, new.regular_vertices);
        assert_eq!(old.regular_indices, new.regular_indices);
        assert_eq!(old.transition_vertices, new.transition_vertices);
        assert_eq!(old.transition_indices, new.transition_indices);
    }
    // Reselect while real commands are in flight, then return before they finish.
    world.set_coverage(&b, position(4_903_255, 80, 16)).unwrap();
    world.update().unwrap();
    assert!(world.in_flight() > 0);
    // GPU commands may complete while this audit reads the old draw buffers, but
    // no replacement can become visible until the owner processes their receipts.
    let waiting = snapshot(&device, &queue, &world, root);
    assert_eq!(waiting.len(), first.len());
    for (key, old) in &first {
        assert_eq!(old.regular_vertices, waiting[key].regular_vertices);
        assert_eq!(old.transition_vertices, waiting[key].transition_vertices);
    }
    world.set_coverage(&a, position(4_903_255, 16, 16)).unwrap();
    let cancelled = settle(&mut world);
    assert_eq!(cancelled.submitted, 0);
    let replay = snapshot(&device, &queue, &world, root);
    for (key, old) in &first {
        assert_eq!(old.regular_vertices, replay[key].regular_vertices);
        assert_eq!(old.transition_vertices, replay[key].transition_vertices);
    }
    for (name, run) in [
        ("cold", cold),
        ("16 m move", small),
        ("64 m move", moved),
        ("revisit", revisit),
    ] {
        println!(
            "saved route {name}: {} changed chunks, first publication {:.2} ms, settled {:.2} ms, prepare {:.2} ms, encode {:.2} ms, peak {:.2} MiB",
            run.submitted,
            run.first_publication_ms,
            run.settled_ms,
            run.preparation_ms,
            run.encoding_ms,
            run.peak_bytes as f64 / (1024.0 * 1024.0)
        );
        assert!(run.peak_bytes <= config.memory_budget_bytes);
    }
    let start = Instant::now();
    let selected = select_voxel_coverage(&field, position(4_903_255, 0, 0), 512, 4096).unwrap();
    assert!(selected.leaves().iter().any(|a| a.lod() == 0));
    println!(
        "planet selection: {} balanced leaves, {:.2} ms CPU",
        selected.leaves().len(),
        start.elapsed().as_secs_f64() * 1000.0
    );
}

#[test]
fn gpu_failures_keep_previous_coverage_and_memory_reservations() {
    let (device, queue) = device();
    let json: serde_json::Value =
        serde_json::from_str(include_str!("../../../planet-design.json")).unwrap();
    let design: PlanetDesignConfig = serde_json::from_value(json["design"].clone()).unwrap();
    let field = Arc::new(design.validate().unwrap());
    let root = parent(position(4_903_255, 16, 16));
    let initial = VoxelCoverage::new(vec![root]).unwrap();
    let target = partition(root, 1);
    let config = VoxelGpuWorldConfig {
        stream: VoxelStreamConfig {
            max_slots: 48,
            max_in_flight: 2,
        },
        mesh: VoxelGpuMeshConfig {
            transition: VoxelTransitionConfig {
                triangle_capacity: 1,
            },
            ..CONFIG
        },
        memory_budget_bytes: 128 * 1024 * 1024,
    };
    let mut world = VoxelGpuWorld::new(&device, &queue, field.clone(), config).unwrap();
    world
        .set_coverage(&initial, position(4_903_255, 16, 16))
        .unwrap();
    settle(&mut world);
    let old = world.resident_slots().next().unwrap().0;
    world
        .set_coverage(&target, position(4_903_255, 16, 16))
        .unwrap();
    let start = Instant::now();
    loop {
        let events = world.update().unwrap();
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, VoxelGpuEvent::Published(_)))
        );
        assert_eq!(world.resident_slots().next().unwrap().0, old);
        if world.failed().next().is_some() && world.in_flight() == 0 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(1));
    }
    snapshot(&device, &queue, &world, root);
    // One resident plus its initial work fits; an unpublished replacement cannot.
    let plan = VoxelTransitionPlan::new(initial.key(root).unwrap());
    let budget = config.mesh.slot_bytes() + VoxelGpuMesher::work_bytes(&plan);
    let mut limited = VoxelGpuWorld::new(
        &device,
        &queue,
        field,
        VoxelGpuWorldConfig {
            memory_budget_bytes: budget,
            ..config
        },
    )
    .unwrap();
    limited
        .set_coverage(&initial, position(4_903_255, 16, 16))
        .unwrap();
    settle(&mut limited);
    let ticket = limited.resident_slots().next().unwrap().0;
    limited
        .set_coverage(&target, position(4_903_255, 16, 16))
        .unwrap();
    assert!(matches!(limited.update(), Err(VoxelGpuError::MemoryBudget)));
    assert_eq!(limited.resident_slots().next().unwrap().0, ticket);
    assert!(limited.memory_bytes() <= budget);
}

#[test]
fn native_device_orbit_meshes_stay_inside_their_chunks() {
    use procgen_gpu_tests::block_on;
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: if cfg!(target_os = "macos") {
            wgpu::Backends::METAL
        } else {
            wgpu::Backends::VULKAN
        },
        ..Default::default()
    });
    let adapter = block_on(instance.request_adapter(&Default::default())).unwrap();
    let mut features = adapter.features();
    if adapter.get_info().device_type == wgpu::DeviceType::DiscreteGpu {
        features.remove(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS);
    }
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: features,
        required_limits: adapter.limits(),
        experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
        ..Default::default()
    }))
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(include_str!("../../../planet-design.json")).unwrap();
    let design: PlanetDesignConfig = serde_json::from_value(json["design"].clone()).unwrap();
    let field = Arc::new(design.validate().unwrap());
    let camera = position(14_700_000, 0, 0);
    let coverage = select_voxel_coverage(&field, camera, 512, 4096).unwrap();
    let mut world = VoxelGpuWorld::new(
        &device,
        &queue,
        field,
        VoxelGpuWorldConfig {
            stream: VoxelStreamConfig {
                max_slots: 128,
                max_in_flight: 2,
            },
            mesh: CONFIG,
            memory_budget_bytes: 1024 * 1024 * 1024,
        },
    )
    .unwrap();
    let mut current = VoxelCoverage::new(vec![VoxelChunkAddress::root()]).unwrap();
    world.set_coverage(&current, camera).unwrap();
    let mut pending = Vec::new();
    world
        .update_with_submit(|submission| pending.push(submission))
        .unwrap();
    assert!(!pending.is_empty());
    device.poll(wgpu::PollType::Poll).unwrap();
    world
        .update_with_submit(|submission| pending.push(submission))
        .unwrap();
    assert_eq!(
        world.resident_slots().count(),
        0,
        "encoding alone cannot publish work"
    );
    for submission in pending {
        submission.submit(&queue);
    }
    settle(&mut world);
    while current.leaves() != coverage.leaves() {
        current = current.step_toward(&coverage, camera).unwrap();
        world.set_coverage(&current, camera).unwrap();
        settle(&mut world);
    }
    for (_, key, slot) in world.resident_slots() {
        for part in [&slot.regular, &slot.transition] {
            let mesh = mesh(&device, &queue, key.address(), part);
            let span = key.address().span_m() as f64;
            for vertex in mesh.vertices() {
                let p = key.address().origin();
                let origin = [p.x_m, p.y_m, p.z_m];
                let r = (0..3)
                    .map(|i| {
                        (origin[i] as f64 + vertex.anchor_m[i] as f64 + vertex.offset_m[i] as f64)
                            .powi(2)
                    })
                    .sum::<f64>()
                    .sqrt();
                assert!(
                    (4_500_000.0..5_300_000.0).contains(&r),
                    "invalid orbit radius {r}, {key:?}, {vertex:?}"
                );
                for axis in 0..3 {
                    let local = vertex.anchor_m[axis] as f64 + vertex.offset_m[axis] as f64;
                    assert!(
                        (0.0..=span).contains(&local),
                        "{key:?} invalid local vertex {vertex:?}"
                    );
                }
            }
        }
    }
    println!("orbit vertices within radial bounds");
}
