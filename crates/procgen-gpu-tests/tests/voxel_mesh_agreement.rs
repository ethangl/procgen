mod voxel_mesh_support;

use procgen_core::Vec3;
use procgen_gpu_tests::{readback, validate_wgsl};
use procgen_realtime_pilot::{
    PlanetDesignConfig, VOXEL_MESH_MAX_TRIANGLES, VOXEL_MESH_VERTEX_SLOTS, VOXEL_SAMPLE_COUNT,
    VoxelChunkAddress, VoxelChunkMesh, VoxelMeshConfig, VoxelMeshVertex, VoxelPosition,
    VoxelVolume, build_voxel_chunk_mesh, sample_voxel_chunk, voxel_mesh_shader,
};
use std::{collections::BTreeMap, time::Instant};
use voxel_mesh_support::{Gpu, Source, sample_index};

const CAPACITY: VoxelMeshConfig = VoxelMeshConfig {
    vertex_capacity: 50_000,
    triangle_capacity: 100_000,
};
// Ten micrometers per meter of spacing accommodates f32 interpolation arithmetic.
const POSITION_TOLERANCE: f64 = 0.00001;

#[test]
fn voxel_mesh_wgsl_validates_without_a_device() {
    validate_wgsl("uniform voxel mesh", &voxel_mesh_shader());
}
fn address(x: i32, y: i32, z: i32, lod: u8) -> VoxelChunkAddress {
    VoxelChunkAddress::containing(
        VoxelPosition {
            x_m: x,
            y_m: y,
            z_m: z,
        },
        lod,
    )
    .unwrap()
}
fn fixture(address: VoxelChunkAddress, f: impl Fn([i32; 3]) -> f32) -> VoxelVolume {
    let origin = address.origin();
    VoxelVolume::from_potentials(
        address,
        (0..VOXEL_SAMPLE_COUNT)
            .map(|i| {
                let p = address.sample_position(sample_index(i)).relative_to(origin);
                f([p.x_m, p.y_m, p.z_m])
            })
            .collect(),
    )
    .unwrap()
}
fn positions(mesh: &VoxelChunkMesh) -> Vec<[f64; 3]> {
    mesh.vertices()
        .iter()
        .map(|v| std::array::from_fn(|i| v.anchor_m[i] as f64 + v.offset_m[i] as f64))
        .collect()
}
fn compare(cpu: &VoxelChunkMesh, gpu: &VoxelChunkMesh) {
    assert_eq!(
        cpu.indices(),
        gpu.indices(),
        "canonical triangle order and vertex allocation"
    );
    assert_eq!(cpu.vertices().len(), gpu.vertices().len());
    let tolerance = POSITION_TOLERANCE * cpu.address().spacing_m() as f64;
    for (a, b) in positions(cpu).iter().zip(positions(gpu)) {
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() <= tolerance, "{a:?} vs {b:?}");
        }
    }
    for (a, b) in cpu.vertices().iter().zip(gpu.vertices()) {
        for i in 0..3 {
            // Five decimal digits for interpolation of identical density samples.
            assert!(
                (a.normal[i] - b.normal[i]).abs() <= 0.00001 * a.normal[i].abs().max(1.0),
                "density normal {:?} vs {:?}",
                a.normal,
                b.normal
            );
        }
    }
}
fn edges(mesh: &VoxelChunkMesh) -> BTreeMap<[u32; 2], (u32, i32)> {
    let mut edges = BTreeMap::new();
    for t in mesh.indices().chunks_exact(3) {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let key = [a.min(b), a.max(b)];
            let e = edges.entry(key).or_insert((0, 0));
            e.0 += 1;
            e.1 += if a < b { 1 } else { -1 };
        }
    }
    edges
}
fn check_topology(mesh: &VoxelChunkMesh, closed: bool) {
    let points = positions(mesh);
    let span = mesh.address().span_m() as f64;
    for ([a, b], (count, orientation)) in edges(mesh) {
        if count == 1 && !closed {
            assert!(
                (0..3).any(|axis| [0.0, span]
                    .into_iter()
                    .any(|boundary| points[a as usize][axis] == boundary
                        && points[b as usize][axis] == boundary)),
                "interior boundary edge"
            );
        } else {
            assert_eq!(
                (count, orientation),
                (2, 0),
                "manifold, consistently wound edge"
            );
        }
    }
}

#[test]
fn gpu_mesh_matches_reference_and_is_schedule_invariant() {
    let gpu = Gpu::new();
    let a = address(0, 0, 0, 0);
    let fixtures = [
        ("empty", fixture(a, |_| -1.0), true),
        ("full", fixture(a, |_| 1.0), true),
        ("all zero", fixture(a, |_| 0.0), true),
        ("axis plane", fixture(a, |p| 13.25 - p[0] as f32), false),
        (
            "exact-zero plane",
            fixture(a, |p| (16 - p[0]) as f32),
            false,
        ),
        (
            "oblique zero plane",
            fixture(a, |p| (32 - p[0] - p[1] - p[2]) as f32),
            false,
        ),
        (
            "isolated zero",
            fixture(a, |p| if p == [16, 16, 16] { 0.0 } else { -1.0 }),
            true,
        ),
        (
            "zero edge",
            fixture(a, |p| if p[0] == 16 && p[1] == 16 { 0.0 } else { -1.0 }),
            true,
        ),
        (
            "sphere",
            fixture(a, |p| {
                10.0 - Vec3::new((p[0] - 16) as f32, (p[1] - 16) as f32, (p[2] - 16) as f32)
                    .length()
            }),
            true,
        ),
        (
            "large origin",
            fixture(address(8_000_000, -8_000_000, 0, 0), |p| {
                0.001 - p[0] as f32
            }),
            false,
        ),
        (
            "coarse",
            fixture(address(0, 0, 0, 18), |p| {
                3.125 * (1 << 18) as f32 - p[0] as f32 - 0.3 * p[1] as f32
            }),
            false,
        ),
        ("last scan block", fixture(a, |p| (p[0] - 32) as f32), false),
        (
            "checkerboard",
            fixture(a, |p| {
                if (p[0] + p[1] + p[2]) % 2 == 0 {
                    1.0
                } else {
                    -1.0
                }
            }),
            false,
        ),
    ];
    let mut jobs = Vec::new();
    let mut first = Vec::new();
    for (name, volume, closed) in &fixtures {
        let cpu_start = Instant::now();
        let cpu = build_voxel_chunk_mesh(volume).unwrap();
        let cpu_ms = cpu_start.elapsed().as_secs_f64() * 1000.0;
        let capacity = if *name == "checkerboard" {
            VoxelMeshConfig {
                vertex_capacity: VOXEL_MESH_VERTEX_SLOTS as u32,
                triangle_capacity: VOXEL_MESH_MAX_TRIANGLES as u32,
            }
        } else {
            CAPACITY
        };
        let job = gpu.prepare(Source::Samples(volume), capacity);
        let ms = gpu.dispatch(&[&job], false);
        let audit = gpu.audit(&job);
        let mesh = audit.mesh.unwrap();
        compare(&cpu, &mesh);
        check_topology(&mesh, *closed);
        println!(
            "{name}: {} vertices / {} triangles; CPU {:.2} ms, GPU submit+wait {:.2} ms",
            mesh.vertices().len(),
            mesh.indices().len() / 3,
            cpu_ms,
            ms
        );
        jobs.push(job);
        first.push(mesh);
    }
    let reversed: Vec<_> = jobs.iter().rev().collect();
    let interleaved_ms = gpu.dispatch(&reversed, true);
    for (job, previous) in jobs.iter().zip(&first) {
        let current = gpu.audit(job).mesh.unwrap();
        assert_eq!(
            previous.vertices(),
            current.vertices(),
            "exact GPU replay and batch order"
        );
        assert_eq!(previous.indices(), current.indices());
    }
    let sequential_ms = gpu.dispatch(&jobs.iter().collect::<Vec<_>>(), false);
    println!(
        "13-chunk warm replay, including worst-case checkerboard: batched {interleaved_ms:.2} ms, sequential {sequential_ms:.2} ms"
    );
    for (job, previous) in jobs.iter().zip(&first) {
        let current = gpu.audit(job).mesh.unwrap();
        assert_eq!(previous.vertices(), current.vertices());
        assert_eq!(previous.indices(), current.indices());
    }
    // Each sphere normal must face away from the positive interior.
    let sphere = &first[8];
    let points = positions(sphere);
    for t in sphere.indices().chunks_exact(3) {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| {
            let p = points[i as usize];
            Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)
        });
        assert!((b - a).cross(c - a).dot(a - Vec3::new(16.0, 16.0, 16.0)) > 0.0);
    }
    check_overflow(&gpu, &fixtures[3].1);
    check_shared_boundaries(&gpu);
    check_normal_boundaries(&gpu);
    check_saved_field(&gpu);
}

fn check_overflow(gpu: &Gpu, volume: &VoxelVolume) {
    let reference = build_voxel_chunk_mesh(volume).unwrap();
    let exact = VoxelMeshConfig {
        vertex_capacity: reference.vertices().len() as u32,
        triangle_capacity: reference.indices().len() as u32 / 3,
    };
    let fitting = gpu.prepare(Source::Samples(volume), exact);
    gpu.dispatch(&[&fitting], false);
    compare(&reference, &gpu.audit(&fitting).mesh.unwrap());
    for config in [
        VoxelMeshConfig {
            vertex_capacity: exact.vertex_capacity - 1,
            ..exact
        },
        VoxelMeshConfig {
            triangle_capacity: exact.triangle_capacity - 1,
            ..exact
        },
        VoxelMeshConfig {
            vertex_capacity: 1,
            ..CAPACITY
        },
        VoxelMeshConfig {
            triangle_capacity: 1,
            ..CAPACITY
        },
    ] {
        let job = gpu.prepare(Source::Samples(volume), config);
        let vertices = vec![
            VoxelMeshVertex {
                anchor_m: [-37; 4],
                offset_m: [-37.0; 4],
                normal: [-37.0; 4]
            };
            config.vertex_capacity as usize
        ];
        let indices = vec![0xdeadbeefu32; config.triangle_capacity as usize * 3];
        gpu.queue.write_buffer(
            &job.slot.regular.vertices,
            0,
            bytemuck::cast_slice(&vertices),
        );
        gpu.queue
            .write_buffer(&job.slot.regular.indices, 0, bytemuck::cast_slice(&indices));
        gpu.dispatch(&[&job], false);
        let audit = gpu.audit(&job);
        assert_eq!(audit.status.overflow, 1);
        assert_eq!(audit.status.vertex_count, exact.vertex_capacity);
        assert_eq!(audit.status.index_count, exact.triangle_capacity * 3);
        assert!(audit.mesh.is_none());
        assert_eq!(audit.draw.index_count, 0);
        assert_eq!(
            readback::<VoxelMeshVertex>(
                &gpu.device,
                &gpu.queue,
                &job.slot.regular.vertices,
                vertices.len()
            ),
            vertices
        );
        assert_eq!(
            readback::<u32>(
                &gpu.device,
                &gpu.queue,
                &job.slot.regular.indices,
                indices.len()
            ),
            indices
        );
    }
}

// Exact same-backend seam comparison, with no quantization or pinned float bits.
#[derive(Clone, Copy, PartialEq)]
struct Point([f64; 3]);
impl Eq for Point {}
impl PartialOrd for Point {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Point {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .iter()
            .zip(other.0)
            .map(|(a, b)| a.total_cmp(&b))
            .find(|o| !o.is_eq())
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

fn check_normal_boundaries(gpu: &Gpu) {
    let mut shared = BTreeMap::new();
    let mut matches = 0;
    for i in 0..8 {
        // Translate the fixture to planetary coordinates without losing sub-meter
        // roots: the field itself is evaluated relative to this integer center.
        let a = address(
            4_900_000 + (i & 1) * 32,
            ((i >> 1) & 1) * 32,
            ((i >> 2) & 1) * 32,
            0,
        );
        let origin = a.origin();
        let volume = fixture(a, |p| {
            let q = Vec3::new(
                (p[0] + origin.x_m - 4_900_032) as f32,
                (p[1] + origin.y_m - 32) as f32,
                (p[2] + origin.z_m - 32) as f32,
            );
            24.0 * 24.0 - q.length_squared()
        });
        let job = gpu.prepare(Source::Samples(&volume), CAPACITY);
        gpu.dispatch(&[&job], false);
        let mesh = gpu.audit(&job).mesh.unwrap();
        compare(&build_voxel_chunk_mesh(&volume).unwrap(), &mesh);
        for (p, v) in positions(&mesh).iter().zip(mesh.vertices()) {
            let world = Point([
                p[0] + f64::from(origin.x_m),
                p[1] + f64::from(origin.y_m),
                p[2] + f64::from(origin.z_m),
            ]);
            if let Some(previous) = shared.insert(world, v.normal) {
                assert_eq!(previous, v.normal, "shared face/edge/corner normal");
                matches += 1;
            }
        }
    }
    assert!(matches > 100, "curved shared boundaries exercised");
}

fn check_shared_boundaries(gpu: &Gpu) {
    // Integer-valued oblique planes include face, edge and corner zero roots.
    for plane in [64.0, 64.375] {
        let mut all_edges = BTreeMap::<[Point; 2], (u32, i32)>::new();
        for i in 0..8 {
            let a = address((i & 1) * 32, ((i >> 1) & 1) * 32, ((i >> 2) & 1) * 32, 0);
            let origin = a.origin();
            let volume = fixture(a, |p| {
                plane - (p[0] + origin.x_m + p[1] + origin.y_m + p[2] + origin.z_m) as f32
            });
            let job = gpu.prepare(Source::Samples(&volume), CAPACITY);
            gpu.dispatch(&[&job], false);
            let mesh = gpu.audit(&job).mesh.unwrap();
            compare(&build_voxel_chunk_mesh(&volume).unwrap(), &mesh);
            let points: Vec<_> = positions(&mesh)
                .iter()
                .map(|p| {
                    Point(std::array::from_fn(|axis| {
                        p[axis] + [origin.x_m, origin.y_m, origin.z_m][axis] as f64
                    }))
                })
                .collect();
            for ([a, b], (count, orientation)) in edges(&mesh) {
                let (a, b) = (points[a as usize], points[b as usize]);
                let entry = all_edges.entry([a.min(b), a.max(b)]).or_default();
                entry.0 += count;
                entry.1 += if a < b { orientation } else { -orientation };
            }
        }
        for ([a, b], (count, orientation)) in all_edges {
            if count == 1 {
                assert!(
                    (0..3).any(|axis| [0.0, 64.0]
                        .into_iter()
                        .any(|v| a.0[axis] == v && b.0[axis] == v)),
                    "shared chunk crack"
                );
            } else {
                assert_eq!((count, orientation), (2, 0));
            }
        }
    }
}

fn check_saved_field(gpu: &Gpu) {
    let json: serde_json::Value =
        serde_json::from_str(include_str!("../../../planet-design.json")).unwrap();
    let config: PlanetDesignConfig = serde_json::from_value(json["design"].clone()).unwrap();
    let field = config.validate().unwrap();
    let a = address(4_903_255, 0, 0, 0);
    let job = gpu.prepare(Source::Field(&field, a), CAPACITY);
    let ms = gpu.dispatch(&[&job], false);
    let readback_start = Instant::now();
    let mesh = gpu.audit(&job).mesh.unwrap();
    let readback_ms = readback_start.elapsed().as_secs_f64() * 1000.0;
    let warm_ms = gpu.dispatch(&[&job], false);
    let repeat = gpu.audit(&job).mesh.unwrap();
    assert_eq!(mesh.vertices(), repeat.vertices());
    assert_eq!(mesh.indices(), repeat.indices());
    let volume = gpu.read_volume(&job);
    compare(&build_voxel_chunk_mesh(&volume).unwrap(), &mesh);
    check_topology(&mesh, false);
    assert!(!mesh.indices().is_empty());
    let cpu_start = Instant::now();
    let cpu_volume = sample_voxel_chunk(&field, a).unwrap();
    let cpu = build_voxel_chunk_mesh(&cpu_volume).unwrap();
    let cpu_ms = cpu_start.elapsed().as_secs_f64() * 1000.0;
    // Density-backend agreement uses G1's 0.02 m bound.
    let mut compared = 0;
    let mut density_error = 0.0f64;
    for y in 0..16 {
        for z in 0..16 {
            let (y, z) = (y as f64 * 2.0 + 0.375, z as f64 * 2.0 + 0.625);
            let Some(x) = ray_x(&mesh, y, z) else {
                continue;
            };
            if !(1.0..31.0).contains(&x) {
                continue;
            }
            let reference = ray_x(&cpu, y, z).expect("CPU density surface coverage");
            density_error = density_error.max((x - reference).abs());
            compared += 1;
        }
    }
    assert!(
        compared >= 64,
        "need a useful set of interior surface probes: {compared}"
    );
    assert!(
        density_error <= 0.02,
        "CPU density surface difference {density_error} m"
    );
    println!("{compared} saved surface probes: CPU density difference {density_error:.6} m");
    println!(
        "saved warm density+mesh {warm_ms:.2} ms; CPU density+mesh {cpu_ms:.2} ms; audit readback {readback_ms:.2} ms"
    );
    println!(
        "saved density -> mesh (no intermediate readback): {:.2} ms, {} vertices, {} triangles; CPU-density mesh {} triangles",
        ms,
        mesh.vertices().len(),
        mesh.indices().len() / 3,
        cpu.indices().len() / 3
    );
}

// Independent axis-ray geometry check, evaluated in f64 only in the audit.
fn ray_x(mesh: &VoxelChunkMesh, y: f64, z: f64) -> Option<f64> {
    ray_x_geometry(&positions(mesh), mesh.indices(), y, z)
}
fn ray_x_geometry(p: &[[f64; 3]], indices: &[u32], y: f64, z: f64) -> Option<f64> {
    indices
        .chunks_exact(3)
        .filter_map(|t| {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| p[i as usize]);
            let determinant = (b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]);
            if determinant == 0.0 {
                return None;
            }
            let u = ((y - a[1]) * (c[2] - a[2]) - (z - a[2]) * (c[1] - a[1])) / determinant;
            let v = ((b[1] - a[1]) * (z - a[2]) - (b[2] - a[2]) * (y - a[1])) / determinant;
            (u >= 0.0 && v >= 0.0 && u + v <= 1.0)
                .then_some(a[0] + u * (b[0] - a[0]) + v * (c[0] - a[0]))
        })
        .max_by(f64::total_cmp)
}
