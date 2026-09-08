use bytemuck::{Pod, Zeroable};
use procgen_core::{ScalarFieldSample3, Vec3};
use procgen_cubesphere::{CubeFace, TILE_QUADS, TILE_VERTICES, TileAddress};
use procgen_gpu_tests::{readback, request_device};
use procgen_terrain::{
    TERRAIN_TILE_SAMPLE_COUNT, TERRAIN_WGSL_DERIVATIVE_ANGLE_TOLERANCE, TERRAIN_WGSL_SOURCE,
    TERRAIN_WGSL_VALUE_TOLERANCE, TerrainCellControls, TerrainControlBake, TerrainGpuParameters,
    TerrainHeightConfig, TerrainNoiseKeys, TerrainStampInput, TerrainStampKind, TerrainTileInputs,
    fixed_level_4_height_config, generate_terrain_tile, pack_control_bake, pack_stamps,
};
use wgpu::util::DeviceExt;

const TEST_SEED: u64 = 0x6d2b_79f5_1234_abcd;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Output {
    sample: [f32; 4],
}

#[test]
fn wgsl_terrain_tiles_agree_with_canonical_cpu_and_share_edges() {
    let Some((adapter_info, device, queue)) = request_device("procgen terrain agreement device")
    else {
        return;
    };
    println!(
        "GPU adapter: {} ({:?}, {:?})",
        adapter_info.name, adapter_info.backend, adapter_info.device_type
    );
    let bake = varying_bake();
    let config = fixed_level_4_height_config();
    let keys = TerrainNoiseKeys::new(TEST_SEED);
    let left_address = TileAddress::new(CubeFace::PositiveZ, 4, 7, 9).unwrap();
    let right_address = TileAddress::new(CubeFace::PositiveZ, 4, 8, 9).unwrap();
    let stamps = test_stamps(left_address);
    let left_inputs = TerrainTileInputs {
        address: left_address,
        controls: &bake,
        stamps: &stamps,
        noise_keys: keys,
    };
    let right_inputs = TerrainTileInputs {
        address: right_address,
        ..left_inputs
    };
    let left_gpu = dispatch_tile(&device, &queue, left_inputs, config);
    let right_gpu = dispatch_tile(&device, &queue, right_inputs, config);
    let left_cpu = generate_terrain_tile(left_inputs, config.validate().unwrap());

    let (maximum_value, maximum_angle) = assert_agreement(&left_gpu, &left_cpu.samples);
    let coast_bake = coast_bake();
    let coast_inputs = TerrainTileInputs {
        controls: &coast_bake,
        ..left_inputs
    };
    let coast_gpu = dispatch_tile(&device, &queue, coast_inputs, config);
    let coast_cpu = generate_terrain_tile(coast_inputs, config.validate().unwrap());
    let (coast_value, coast_angle) = assert_agreement(&coast_gpu, &coast_cpu.samples);
    let maximum_value = maximum_value.max(coast_value);
    let maximum_angle = maximum_angle.max(coast_angle);

    for y in 0..TILE_VERTICES as usize {
        assert_eq!(
            left_gpu[y * TILE_VERTICES as usize + TILE_QUADS as usize].sample,
            right_gpu[y * TILE_VERTICES as usize].sample,
            "same-level GPU edge row {y}"
        );
    }
    println!(
        "terrain agreement: {} samples, maximum value difference {maximum_value:.9e}, maximum derivative angle {maximum_angle:.9e} radians; tolerances {:.1e} and {:.1e}",
        TERRAIN_TILE_SAMPLE_COUNT * 2,
        TERRAIN_WGSL_VALUE_TOLERANCE,
        TERRAIN_WGSL_DERIVATIVE_ANGLE_TOLERANCE,
    );
}

fn assert_agreement(actual: &[Output], expected: &[ScalarFieldSample3]) -> (f32, f32) {
    let mut maximum_value = 0.0_f32;
    let mut maximum_angle = 0.0_f32;
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        let actual = sample(*actual);
        let value_difference = (actual.value - expected.value).abs();
        let derivative_angle = derivative_angle(actual.derivative, expected.derivative);
        maximum_value = maximum_value.max(value_difference);
        maximum_angle = maximum_angle.max(derivative_angle);
        assert!(
            value_difference <= TERRAIN_WGSL_VALUE_TOLERANCE,
            "sample {index} value difference {value_difference:e} exceeds {}: GPU={actual:?}, CPU={expected:?}",
            TERRAIN_WGSL_VALUE_TOLERANCE,
        );
        assert!(
            derivative_angle <= TERRAIN_WGSL_DERIVATIVE_ANGLE_TOLERANCE,
            "sample {index} derivative angle {derivative_angle:e} exceeds {}: GPU={actual:?}, CPU={expected:?}",
            TERRAIN_WGSL_DERIVATIVE_ANGLE_TOLERANCE,
        );
    }
    (maximum_value, maximum_angle)
}

fn varying_bake() -> TerrainControlBake {
    let resolution = 16;
    let faces = CubeFace::ALL.map(|face| {
        (0..resolution)
            .flat_map(|y| {
                (0..resolution).map(move |x| {
                    let u = (x as f32 + 0.5) / resolution as f32;
                    let v = (y as f32 + 0.5) / resolution as f32;
                    TerrainCellControls {
                        base_elevation: 0.7 + face.index() as f32 * 0.01 + u * 0.03 - v * 0.02,
                        detail_amplitude: 0.018 + u * 0.004,
                        ridge_weight: 0.25 + v * 0.4,
                        octave_gain: 0.42 + u * 0.12,
                        abyssal_amplitude: 0.009 - v * 0.003,
                    }
                    .to_channels()
                })
            })
            .collect()
    });
    TerrainControlBake::from_face_texels(resolution, faces).unwrap()
}

fn coast_bake() -> TerrainControlBake {
    let resolution = 16;
    let texel = TerrainCellControls {
        base_elevation: 0.53,
        detail_amplitude: 0.025,
        ridge_weight: 0.45,
        octave_gain: 0.51,
        abyssal_amplitude: 0.008,
    }
    .to_channels();
    TerrainControlBake::from_face_texels(
        resolution,
        CubeFace::ALL.map(|_| vec![texel; (resolution * resolution) as usize]),
    )
    .unwrap()
}

fn test_stamps(address: TileAddress) -> Vec<TerrainStampInput> {
    [
        (
            TerrainStampKind::Hotspot,
            address.grid_vertex(32, 32).unwrap().direction(),
            0.8,
        ),
        (
            TerrainStampKind::VolcanicArc,
            address.grid_vertex(48, 20).unwrap().direction(),
            0.65,
        ),
    ]
    .into_iter()
    .enumerate()
    .map(
        |(source_index, (kind, position, strength))| TerrainStampInput {
            cell: 0,
            kind,
            source_index,
            position,
            strength,
        },
    )
    .collect()
}

fn sample(output: Output) -> ScalarFieldSample3 {
    ScalarFieldSample3 {
        value: output.sample[0],
        derivative: Vec3::new(output.sample[1], output.sample[2], output.sample[3]),
    }
}

fn derivative_angle(left: Vec3, right: Vec3) -> f32 {
    if left.length_squared() == 0.0 && right.length_squared() == 0.0 {
        0.0
    } else {
        left.cross(right).length().atan2(left.dot(right))
    }
}

fn dispatch_tile(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    inputs: TerrainTileInputs<'_>,
    config: TerrainHeightConfig,
) -> Vec<Output> {
    let parameters =
        TerrainGpuParameters::new(inputs.controls, inputs.stamps, inputs.noise_keys, config);
    let controls = pack_control_bake(inputs.controls);
    let stamps = pack_stamps(inputs.stamps);
    let controls_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("terrain agreement controls"),
        contents: bytemuck::cast_slice(&controls),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let stamps_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("terrain agreement stamps"),
        contents: bytemuck::cast_slice(&stamps),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let parameters_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("terrain agreement parameters"),
        contents: bytemuck::bytes_of(&parameters),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let output_size = (TERRAIN_TILE_SAMPLE_COUNT * size_of::<Output>()) as u64;
    let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("terrain agreement output"),
        size: output_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let shader_source = format!(
        r#"
{TERRAIN_WGSL_SOURCE}
@group(0) @binding(0) var<storage, read> control_texels: array<CubesphereFieldTexel>;
@group(0) @binding(1) var<storage, read> stamps: array<TerrainStamp>;
@group(0) @binding(2) var<uniform> parameters: TerrainParameters;
@group(0) @binding(3) var<storage, read_write> output: array<vec4<f32>>;
fn cubesphere_load_field_texel(index: u32) -> CubesphereFieldTexel {{ return control_texels[index]; }}
fn terrain_load_stamp(index: u32) -> TerrainStamp {{ return stamps[index]; }}
fn terrain_parameters() -> TerrainParameters {{ return parameters; }}
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if (id.x >= {TERRAIN_TILE_SAMPLE_COUNT}u) {{ return; }}
    let local = vec2(id.x % {TILE_VERTICES}u, id.x / {TILE_VERTICES}u);
    let direction = cubesphere_tile_direction(vec4({face}u, {level}u, {x}u, {y}u), local);
    let height = terrain_height_gpu(direction);
    output[id.x] = vec4(height.value, height.derivative);
}}
"#,
        face = inputs.address.face().index(),
        level = inputs.address.level(),
        x = inputs.address.x(),
        y = inputs.address.y(),
    );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("terrain agreement shader"),
        source: wgpu::ShaderSource::Wgsl(shader_source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("terrain agreement pipeline"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("terrain agreement bind group"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: controls_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: stamps_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: parameters_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: output_buffer.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(TERRAIN_TILE_SAMPLE_COUNT.div_ceil(64) as u32, 1, 1);
    }
    queue.submit([encoder.finish()]);
    readback(device, queue, &output_buffer, TERRAIN_TILE_SAMPLE_COUNT)
}
