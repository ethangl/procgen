//! Exercise the viewer's actual water shader on Metal/Vulkan, without terrain generation.
#[path = "../../../apps/realtime-pilot/src/physical_ocean.rs"]
mod physical_ocean;
use procgen_core::Vec3;
use procgen_gpu_tests::{
    readback, request_device_for_backends, run_compute, storage_output_buffer,
};
use procgen_realtime_pilot::{MeterPosition, VoxelPosition};
use wgpu::util::DeviceExt;

const SOURCE: &str = concat!(
    include_str!("../../../apps/realtime-pilot/src/physical_frame.wgsl"),
    "\nstruct Surface { color: vec4<f32>, depth: f32 }\n",
    include_str!("../../../apps/realtime-pilot/src/physical_ocean.wgsl"),
    r#"
    struct Probe { ray: vec4<f32>, terrain: vec4<f32> }
    struct Result { interval: vec4<f32>, color: vec4<f32> }
    @group(0) @binding(1) var<storage,read> probes: array<Probe>;
    @group(0) @binding(2) var<storage,read_write> results: array<Result>;
    @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
        if id.x >= arrayLength(&probes) { return; }
        let p = probes[id.x];
        let ray = normalize(p.ray.xyz);
        let interval = ocean_interval(ray);
        let surface = ocean_surface(Surface(vec4(p.terrain.rgb,1.0),p.terrain.w), ray.xy / -ray.z,ray);
        results[id.x] = Result(vec4(interval,surface.depth,0.0),surface.color);
    }
    "#
);

#[test]
fn sphere_roots_shore_occlusion_and_underwater_depth() {
    let backend = if cfg!(target_os = "macos") {
        wgpu::Backends::METAL
    } else {
        wgpu::Backends::VULKAN
    };
    let (adapter, device, queue) =
        request_device_for_backends("ocean", backend).expect("selected GPU backend required");
    eprintln!("{} {:?}", adapter.name, adapter.backend);
    for radius in [100_000, 300_000, 8_000_000] {
        for altitude in [-2.0, -0.02, 0.0, 0.02, 2.0, 600_000.0] {
            let eye = MeterPosition::new(
                VoxelPosition {
                    x_m: 0,
                    y_m: 0,
                    z_m: radius,
                },
                Vec3::Z * altitude,
            );
            let camera = physical_ocean::OceanCamera::new(eye, radius as f32, 0.0);
            let mut frame = [0f32; 72];
            // Reverse-Z perspective, near 1 m, eye at the relative origin.
            frame[..16].copy_from_slice(&[
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 0., -1., 0., 0., 1., 0.,
            ]);
            frame[40..56].copy_from_slice(&[
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 0., 1., 0., 0., -1., 0.,
            ]);
            frame[64..68].copy_from_slice(&camera.radial);
            frame[68..72].copy_from_slice(&camera.sphere);
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&frame),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let mut probes: Vec<[[f32; 4]; 2]> = (-32..=32)
                .map(|i| {
                    let ray = Vec3::new(1.0, 0.0, i as f32 / 32.0).normalized();
                    [[ray.x, ray.y, ray.z, 0.0], [1., 0., 0., 0.0]]
                })
                .collect();
            if altitude > 0.0 {
                let d = radius as f64 + altitude as f64;
                let horizon_z =
                    -((altitude as f64 * (2.0 * radius as f64 + altitude as f64)) / (d * d)).sqrt();
                // Rays just above and below the horizon, including centimeter altitudes.
                for factor in [0.99, 1.01] {
                    let z = (horizon_z * factor).max(-1.0) as f32;
                    let x = (1.0 - z * z).sqrt();
                    probes.push([[x, 0.0, z, 0.0], [1., 0., 0., 0.0]]);
                }
            }
            let ray_count = probes.len();
            // Downward rays: dry ground, shallow submerged ground, deep ground; then looking up.
            probes.extend([
                [[0., 0., -1., 0.], [1., 0., 0., 1.0]],
                [[0., 0., -1., 0.], [1., 0., 0., 0.25]],
                [[0., 0., -1., 0.], [1., 0., 0., 0.001]],
                [[0., 0., 1., 0.], [1., 0., 0., 0.0]],
            ]);
            let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&probes),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let output =
                storage_output_buffer::<[[f32; 4]; 2]>(&device, "ocean probes", probes.len());
            run_compute(
                &device,
                &queue,
                "ocean probes",
                SOURCE,
                &[
                    uniform.as_entire_binding(),
                    input.as_entire_binding(),
                    output.as_entire_binding(),
                ],
                probes.len() as u32,
            );
            let results = readback::<[[f32; 4]; 2]>(&device, &queue, &output, probes.len());
            for (probe, result) in probes.iter().zip(&results).take(ray_count) {
                let ray = probe[0];
                // Independent f64 geometric reference; relative tolerance covers f32 roots far from the camera.
                let z = ray[2] as f64 / ((ray[0] as f64).powi(2) + (ray[2] as f64).powi(2)).sqrt();
                let r = radius as f64;
                let h = altitude as f64;
                let b = (r + h) * z;
                let discriminant = b * b - h * (2.0 * r + h);
                if discriminant < 0.0 {
                    assert_eq!(&result[0][..2], &[-1., -1.]);
                } else {
                    for (actual, expected) in result[0][..2]
                        .iter()
                        .zip([-b - discriminant.sqrt(), -b + discriminant.sqrt()])
                    {
                        assert!(
                            (*actual as f64 - expected).abs() < 0.001 + expected.abs() * 0.00001,
                            "radius {radius}, altitude {altitude}, ray {ray:?}: {actual} vs {expected}"
                        );
                    }
                }
            }
            for result in &results[ray_count..] {
                assert!(result.iter().flatten().all(|v| v.is_finite()));
            }
            if altitude == 2.0 {
                assert_eq!(
                    results[ray_count][1],
                    [1., 0., 0., 1.],
                    "dry terrain must hide water"
                );
                assert_eq!(results[ray_count][0][2], 1.0);
                for result in &results[ray_count + 1..ray_count + 3] {
                    assert!(
                        (result[0][2] - 0.5).abs() < 0.00001,
                        "water writes its own depth"
                    );
                }
                assert!(
                    results[ray_count + 1][1][0] > results[ray_count + 2][1][0],
                    "shallow bottom stays visible"
                );
                assert_eq!(
                    results[ray_count + 3][0][2],
                    0.0,
                    "sky ray does not hit ocean behind eye"
                );
            } else if altitude == -2.0 {
                assert_eq!(
                    results[ray_count][0][2], 1.0,
                    "submerged bottom keeps terrain depth"
                );
                assert!(results[ray_count][1][0] < 1.0, "underwater attenuation");
                assert!(
                    results[ray_count + 3][1][2] > results[ray_count + 3][1][0],
                    "looking out is water tinted"
                );
            }
        }
    }
}
