use std::{collections::BTreeMap, time::Instant};

use procgen_core::Vec3;
use procgen_gpu_tests::{
    readback, request_device_for_backends, storage_output_buffer, validate_wgsl,
};
use procgen_realtime_pilot::{
    MAX_DESIGN_OCTAVES, OctaveConfig, PlanetDesignConfig, PlanetDesignField, VOXEL_HALO,
    VOXEL_SAMPLE_COUNT, VOXEL_SAMPLE_SIDE, VoxelChunkAddress, VoxelGpuChunk, VoxelGpuParameters,
    VoxelPosition, VoxelSampleIndex, sample_voxel_chunk, voxel_density_shader,
};
use wgpu::util::DeviceExt;

// Two centimeters is 1/50 of a finest voxel. Coarse unsaturated potentials also
// allow four f32 ULP-scale relative errors, without weakening the surface bound.
const SURFACE_TOLERANCE_M: f32 = 0.02;
const RELATIVE_TOLERANCE: f32 = 4.0 * f32::EPSILON;

#[test]
fn voxel_density_wgsl_validates_without_a_device() {
    validate_wgsl("physical voxel density", &voxel_density_shader());
}

#[test]
fn gpu_density_matches_full_band_cpu_chunks_and_replays_exactly() {
    let gpu = Gpu::new();
    let json: serde_json::Value =
        serde_json::from_str(include_str!("../../../planet-design.json")).unwrap();
    let saved: PlanetDesignConfig = serde_json::from_value(json["design"].clone()).unwrap();
    let field = saved.validate().unwrap();
    let base = surface_chunk(&field, Vec3::X);
    let origin = base.origin();
    let mut addresses: Vec<_> = (0..8)
        .map(|i| {
            VoxelChunkAddress::containing(
                VoxelPosition {
                    x_m: origin.x_m + (i & 1) * base.span_m(),
                    y_m: origin.y_m + ((i >> 1) & 1) * base.span_m(),
                    z_m: origin.z_m + ((i >> 2) & 1) * base.span_m(),
                },
                0,
            )
            .unwrap()
        })
        .collect();
    addresses.extend([
        base.parent().unwrap(),
        surface_chunk(&field, Vec3::new(1.0, 0.3, -0.2)),
        VoxelChunkAddress::root(),
    ]);
    let first = gpu.dispatch(&field, &addresses);
    compare_cpu("saved preset", &field, &addresses, &first.values);
    check_shared_samples(&addresses, &first.values);
    let repeat = gpu.dispatch(&field, &addresses);
    assert_eq!(first.values, repeat.values, "repeat dispatch");
    let mut reversed = addresses.clone();
    reversed.reverse();
    let reordered = gpu.dispatch(&field, &reversed);
    for i in 0..addresses.len() {
        assert_eq!(
            chunk_values(&first.values, i),
            chunk_values(&reordered.values, addresses.len() - 1 - i),
            "batch order must not change a chunk"
        );
    }
    let single = gpu.dispatch(&field, &[addresses[9]]);
    assert_eq!(
        single.values,
        chunk_values(&first.values, 9),
        "single versus batch"
    );
    println!(
        "saved preset, {} samples: first dispatch+wait {:.2} ms, warm {:.2} ms, warm readback {:.2} ms; buffer creation and pipeline compilation excluded",
        first.values.len(),
        first.dispatch_wait_ms,
        repeat.dispatch_wait_ms,
        repeat.readback_ms,
    );

    for (radius, seed) in [
        (100_000.125, 0),
        (8_000_000.0, u64::MAX),
        (4_900_000.0, 1_u64 << 32),
    ] {
        let mut config = PlanetDesignConfig::starter(seed);
        config.radius_m = radius;
        let field = config.validate().unwrap();
        let addresses = [
            surface_chunk(&field, -Vec3::X),
            surface_chunk(&field, Vec3::new(-1.0, 1.0, -1.0)),
            VoxelChunkAddress::root(),
        ];
        let result = gpu.dispatch(&field, &addresses);
        compare_cpu(
            &format!("radius {radius}, seed {seed}"),
            &field,
            &addresses,
            &result.values,
        );
    }

    let mut extremes = saved.clone();
    extremes.radius_m = 8_000_000.0;
    extremes.height_limit_m = 20_000.0;
    extremes.seed = u64::MAX;
    extremes.octaves = (0..MAX_DESIGN_OCTAVES)
        .map(|i| OctaveConfig {
            enabled: i % 5 != 1,
            wavelength_m: 8.0 * 1.5_f32.powi((MAX_DESIGN_OCTAVES - 1 - i) as i32),
            amplitude_m: if i % 5 == 2 {
                0.0
            } else {
                20_000.0 * 0.8_f32.powi(i as i32)
            },
            sharpness: (i % 3) as f32 - 1.0,
            perturbation: 1.0,
            slope_erosion: 1.0,
            // Full altitude damping on the first band makes the whole field
            // identically zero, which cannot exercise the feedback arithmetic.
            altitude_erosion: if i == 0 { 0.0 } else { 0.5 },
            ridge_erosion: 1.0,
        })
        .collect();
    let field = extremes.validate().unwrap();
    let addresses = [
        surface_chunk(&field, Vec3::new(1.0, -0.4, 0.7)),
        VoxelChunkAddress::root(),
    ];
    let result = gpu.dispatch(&field, &addresses);
    compare_cpu(
        "24 bands, disabled/zero bands, extreme controls",
        &field,
        &addresses,
        &result.values,
    );
    for band in &mut extremes.octaves {
        if band.amplitude_m == 0.0 {
            band.enabled = false;
        }
    }
    assert_eq!(
        result.values,
        gpu.dispatch(&extremes.validate().unwrap(), &addresses)
            .values,
        "zero amplitude has no feedback"
    );

    for band in &mut extremes.octaves {
        band.enabled = false;
    }
    let flat = extremes.validate().unwrap();
    let addresses = [
        surface_chunk(&flat, Vec3::X),
        surface_chunk(&flat, Vec3::new(1.0, 1.0, 1.0)),
        VoxelChunkAddress::root(),
    ];
    let result = gpu.dispatch(&flat, &addresses);
    compare_cpu("flat sphere and center", &flat, &addresses, &result.values);
}

fn surface_chunk(field: &PlanetDesignField, direction: Vec3) -> VoxelChunkAddress {
    let direction = direction.normalized();
    let p = direction * (field.config().radius_m + field.elevation_m(direction, 0.0).unwrap());
    VoxelChunkAddress::containing(
        VoxelPosition {
            x_m: p.x.round() as i32,
            y_m: p.y.round() as i32,
            z_m: p.z.round() as i32,
        },
        0,
    )
    .unwrap()
}

fn sample_index(i: usize) -> VoxelSampleIndex {
    VoxelSampleIndex {
        x: (i % VOXEL_SAMPLE_SIDE) as i32 - VOXEL_HALO,
        y: (i / VOXEL_SAMPLE_SIDE % VOXEL_SAMPLE_SIDE) as i32 - VOXEL_HALO,
        z: (i / (VOXEL_SAMPLE_SIDE * VOXEL_SAMPLE_SIDE)) as i32 - VOXEL_HALO,
    }
}
fn chunk_values(values: &[f32], chunk: usize) -> &[f32] {
    &values[chunk * VOXEL_SAMPLE_COUNT..(chunk + 1) * VOXEL_SAMPLE_COUNT]
}
fn compare_cpu(
    label: &str,
    field: &PlanetDesignField,
    addresses: &[VoxelChunkAddress],
    values: &[f32],
) {
    let start = Instant::now();
    let cpu: Vec<_> = addresses
        .iter()
        .map(|&address| sample_voxel_chunk(field, address).unwrap())
        .collect();
    let cpu_ms = start.elapsed().as_secs_f64() * 1000.0;
    let (mut max_error, mut near_error) = (0.0_f32, 0.0_f32);
    for (chunk, volume) in cpu.iter().enumerate() {
        for (i, &actual) in chunk_values(values, chunk).iter().enumerate() {
            let expected = volume.potential(sample_index(i));
            let difference = (actual - expected).abs();
            assert!(actual.is_finite());
            assert!(
                difference <= SURFACE_TOLERANCE_M + RELATIVE_TOLERANCE * expected.abs(),
                "{label}, {:?}, {:?}: GPU {actual}, CPU {expected}, difference {difference} m",
                addresses[chunk],
                sample_index(i)
            );
            max_error = max_error.max(difference);
            if expected.abs() <= 32.0 {
                near_error = near_error.max(difference);
            }
        }
    }
    println!(
        "{label}: {} samples, max potential error {max_error:.6} m, within 32 m of surface {near_error:.6} m, CPU sampling {cpu_ms:.2} ms",
        values.len()
    );
}
fn check_shared_samples(addresses: &[VoxelChunkAddress], values: &[f32]) {
    let mut samples = BTreeMap::new();
    let mut coincident = 0;
    for (chunk, address) in addresses.iter().enumerate() {
        for (i, &value) in chunk_values(values, chunk).iter().enumerate() {
            let p = address.sample_position(sample_index(i));
            if let Some(previous) = samples.insert(p, value) {
                assert_eq!(value, previous, "shared GPU sample {p:?}");
                coincident += 1;
            }
        }
    }
    assert!(coincident > VOXEL_SAMPLE_SIDE * VOXEL_SAMPLE_SIDE);
    println!("{coincident} coincident GPU halo/parent samples agree exactly");
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
}
struct Dispatch {
    values: Vec<f32>,
    dispatch_wait_ms: f64,
    readback_ms: f64,
}
impl Gpu {
    fn new() -> Self {
        let backend = if cfg!(target_os = "macos") {
            wgpu::Backends::METAL
        } else {
            wgpu::Backends::VULKAN
        };
        let (info, device, queue) = request_device_for_backends("voxel agreement", backend)
            .expect("this agreement test requires a Metal or Vulkan GPU");
        println!("GPU adapter: {} ({:?})", info.name, info.backend);
        let source = voxel_density_shader();
        validate_wgsl("voxel density", &source);
        let start = Instant::now();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("voxel density"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("voxel density"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        println!(
            "shader/pipeline creation: {:.2} ms",
            start.elapsed().as_secs_f64() * 1000.0
        );
        Self {
            device,
            queue,
            pipeline,
        }
    }
    fn dispatch(&self, field: &PlanetDesignField, addresses: &[VoxelChunkAddress]) -> Dispatch {
        let parameters = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxel field"),
                contents: bytemuck::bytes_of(&VoxelGpuParameters::new(field)),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let chunks: Vec<_> = addresses.iter().copied().map(VoxelGpuChunk::new).collect();
        let chunks = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("voxel chunks"),
                contents: bytemuck::cast_slice(&chunks),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let count = VOXEL_SAMPLE_COUNT * addresses.len();
        assert!(
            count.div_ceil(64)
                <= self.device.limits().max_compute_workgroups_per_dimension as usize
        );
        let output = storage_output_buffer::<f32>(&self.device, "voxel potentials", count);
        let entries = [
            parameters.as_entire_binding(),
            chunks.as_entire_binding(),
            output.as_entire_binding(),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, resource)| wgpu::BindGroupEntry {
            binding: i as u32,
            resource,
        })
        .collect::<Vec<_>>();
        let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("voxel density"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let start = Instant::now();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(count.div_ceil(64) as u32, 1, 1);
        }
        self.queue.submit([encoder.finish()]);
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let dispatch_wait_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let values = readback(&self.device, &self.queue, &output, count);
        Dispatch {
            values,
            dispatch_wait_ms,
            readback_ms: start.elapsed().as_secs_f64() * 1000.0,
        }
    }
}
