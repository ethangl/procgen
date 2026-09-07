use std::{
    future::Future,
    sync::{Arc, mpsc},
    task::{Context, Poll, Wake, Waker},
    thread,
};

use bytemuck::{Pod, Zeroable};
use procgen_core::{HASH_U32_TEST_VECTORS, Vec3, hash_u32};
use procgen_noise::{
    DerivativeDampedConfig, NoiseSample3, OctaveConfig, OctaveGain, RidgedMultifractalConfig,
    WGSL_SOURCE, derivative_damped_fbm_3d, fbm_3d, fold_seed_u64_to_u32, gradient_noise_3d,
    ridged_multifractal_3d,
};
use wgpu::util::DeviceExt;

/// Provisional absolute CPU/GPU agreement tolerance in normalized units.
///
/// The terrain-detail plan sets this at `1e-5`; measurements printed by this
/// test will inform a later Metal-and-CUDA calibration.
const PROVISIONAL_NOISE_AGREEMENT_TOLERANCE: f32 = 1.0e-5;

const MODE_BASIS: u32 = 0;
const MODE_FBM: u32 = 1;
const MODE_RIDGED: u32 = 2;
const MODE_DAMPED: u32 = 3;
const MODE_CORE_HASH: u32 = 4;
const MODE_LATTICE_HASH: u32 = 5;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShaderInput {
    words: [u32; 4],
    seed: [u32; 2],
    mode: u32,
    octaves: u32,
    position: [f32; 4],
    config: [f32; 4],
    ridge: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct ShaderOutput {
    hashes: [u32; 4],
    sample: [f32; 4],
}

#[derive(Clone, Copy)]
struct FloatCase {
    label: &'static str,
    expected: NoiseSample3,
}

#[test]
fn wgsl_noise_agrees_with_canonical_cpu() {
    let instance = wgpu::Instance::default();
    let adapter = match block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    })) {
        Ok(adapter) => adapter,
        Err(error) => {
            eprintln!("SKIPPED: no compatible GPU adapter exists: {error}");
            return;
        }
    };
    let adapter_info = adapter.get_info();
    println!(
        "GPU adapter: {} ({:?}, {:?})",
        adapter_info.name, adapter_info.backend, adapter_info.device_type
    );

    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("procgen noise agreement device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
    }))
    .expect("a compatible adapter must provide a baseline compute device");

    let (inputs, float_cases) = agreement_cases();
    let outputs = dispatch(&device, &queue, &inputs);
    assert_eq!(outputs.len(), inputs.len());

    for (index, &([word0, word1, word2, word3], expected)) in
        HASH_U32_TEST_VECTORS.iter().enumerate()
    {
        assert_eq!(
            outputs[index].hashes[0], expected,
            "core hash vector {index}"
        );
        assert_eq!(hash_u32(word0, word1, word2, word3), expected);
    }

    let lattice_start = HASH_U32_TEST_VECTORS.len();
    for (case_index, input) in inputs[lattice_start..lattice_start + 4].iter().enumerate() {
        let output = outputs[lattice_start + case_index];
        let key = fold_seed_u64_to_u32(join_seed(input.seed));
        let expected = hash_u32(key, input.words[0], input.words[1], input.words[2]);
        assert_eq!(output.hashes[1], key, "seed fold vector {case_index}");
        assert_eq!(
            output.hashes[2], expected,
            "lattice hash vector {case_index}"
        );
    }

    let float_start = lattice_start + 4;
    let mut maximum_difference = 0.0_f32;
    let mut maximum_label = "";
    let mut maximum_component = 0;
    for (case_index, case) in float_cases.iter().enumerate() {
        let actual = outputs[float_start + case_index].sample;
        let expected = sample_components(case.expected);
        for component in 0..4 {
            let difference = (actual[component] - expected[component]).abs();
            if difference > maximum_difference {
                maximum_difference = difference;
                maximum_label = case.label;
                maximum_component = component;
            }
            assert!(
                difference <= PROVISIONAL_NOISE_AGREEMENT_TOLERANCE,
                "{} component {component}: GPU {} CPU {} difference {} exceeds {}",
                case.label,
                actual[component],
                expected[component],
                difference,
                PROVISIONAL_NOISE_AGREEMENT_TOLERANCE
            );
        }
    }
    println!(
        "noise agreement: {} float samples, maximum absolute difference {:.9e} at {} component {}, tolerance {:.1e}",
        float_cases.len(),
        maximum_difference,
        maximum_label,
        maximum_component,
        PROVISIONAL_NOISE_AGREEMENT_TOLERANCE
    );
}

fn agreement_cases() -> (Vec<ShaderInput>, Vec<FloatCase>) {
    let mut inputs = Vec::new();
    for &([word0, word1, word2, word3], _) in &HASH_U32_TEST_VECTORS {
        inputs.push(ShaderInput {
            words: [word0, word1, word2, word3],
            mode: MODE_CORE_HASH,
            ..ShaderInput::zeroed()
        });
    }

    // These explicitly exercise both seed halves and negative lattice-address bits.
    for (seed, coordinates) in [
        (0x0000_0000_0000_0001, [0, 0, 0]),
        (0x0000_0001_0000_0000, [-1, 2, -3]),
        (0x0123_4567_89ab_cdef, [i32::MIN, -17, i32::MAX]),
        (u64::MAX, [-32_769, -1, 32_768]),
    ] {
        inputs.push(ShaderInput {
            words: [
                coordinates[0] as u32,
                coordinates[1] as u32,
                coordinates[2] as u32,
                0,
            ],
            seed: split_seed(seed),
            mode: MODE_LATTICE_HASH,
            ..ShaderInput::zeroed()
        });
    }

    let positions = [
        Vec3::new(0.25, 0.5, 0.75),
        Vec3::new(-1.25, 2.5, -9.75),
        Vec3::new(12.345, -67.89, 0.125),
    ];
    let seeds = [1_u64, 0x0000_0001_0000_0000, 0x0123_4567_89ab_cdef];
    let basis_labels = ["basis-a", "basis-b", "basis-c"];
    let mut float_cases = Vec::new();
    for (case_index, (&seed, &position)) in seeds.iter().zip(&positions).enumerate() {
        push_float_case(
            &mut inputs,
            &mut float_cases,
            basis_labels[case_index],
            seed,
            position,
            MODE_BASIS,
            0,
            [0.0; 4],
            [0.0; 4],
            gradient_noise_3d(seed, position),
        );
    }

    let configurations = [
        (3, 0.75, 2.0, 0.5, 1.0, 2.0, 0.75),
        (5, 1.25, 1.75, 0.625, 1.2, 1.4, 1.5),
    ];
    for (
        configuration_index,
        &(octaves, frequency, lacunarity, gain, ridge_offset, ridge_gain, damping),
    ) in configurations.iter().enumerate()
    {
        let position = positions[configuration_index + 1];
        let seed = seeds[configuration_index + 1];
        let octave_config = OctaveConfig {
            octaves,
            frequency,
            lacunarity,
        };
        let octave_gain = OctaveGain::new(gain).unwrap();
        let config = [frequency, lacunarity, gain, damping];
        let ridge = [ridge_offset, ridge_gain, 0.0, 0.0];
        push_float_case(
            &mut inputs,
            &mut float_cases,
            if configuration_index == 0 {
                "fbm-a"
            } else {
                "fbm-b"
            },
            seed,
            position,
            MODE_FBM,
            octaves,
            config,
            ridge,
            fbm_3d(
                seed,
                position,
                octave_config.validate().unwrap(),
                octave_gain,
            ),
        );
        push_float_case(
            &mut inputs,
            &mut float_cases,
            if configuration_index == 0 {
                "ridged-a"
            } else {
                "ridged-b"
            },
            seed,
            position,
            MODE_RIDGED,
            octaves,
            config,
            ridge,
            ridged_multifractal_3d(
                seed,
                position,
                RidgedMultifractalConfig {
                    octaves: octave_config,
                    ridge_offset,
                    ridge_gain,
                }
                .validate()
                .unwrap(),
                octave_gain,
            ),
        );
        push_float_case(
            &mut inputs,
            &mut float_cases,
            if configuration_index == 0 {
                "damped-a"
            } else {
                "damped-b"
            },
            seed,
            position,
            MODE_DAMPED,
            octaves,
            config,
            ridge,
            derivative_damped_fbm_3d(
                seed,
                position,
                DerivativeDampedConfig {
                    octaves: octave_config,
                    damping,
                }
                .validate()
                .unwrap(),
                octave_gain,
            ),
        );
    }
    (inputs, float_cases)
}

#[allow(clippy::too_many_arguments)]
fn push_float_case(
    inputs: &mut Vec<ShaderInput>,
    cases: &mut Vec<FloatCase>,
    label: &'static str,
    seed: u64,
    position: Vec3,
    mode: u32,
    octaves: u32,
    config: [f32; 4],
    ridge: [f32; 4],
    expected: NoiseSample3,
) {
    let input = ShaderInput {
        seed: split_seed(seed),
        mode,
        octaves,
        position: [position.x, position.y, position.z, 0.0],
        config,
        ridge,
        ..ShaderInput::zeroed()
    };
    inputs.push(input);
    cases.push(FloatCase { label, expected });
}

fn dispatch(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    inputs: &[ShaderInput],
) -> Vec<ShaderOutput> {
    let harness = format!(
        r#"
{WGSL_SOURCE}

struct Input {{
    words: vec4<u32>,
    seed: vec2<u32>,
    mode: u32,
    octaves: u32,
    position: vec4<f32>,
    config: vec4<f32>,
    ridge: vec4<f32>,
}}
struct Output {{ hashes: vec4<u32>, sample: vec4<f32> }}
@group(0) @binding(0) var<storage, read> inputs: array<Input>;
@group(0) @binding(1) var<storage, read_write> outputs: array<Output>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if (id.x >= arrayLength(&inputs)) {{ return; }}
    let input = inputs[id.x];
    let key = fold_seed_u64_to_u32(input.seed.x, input.seed.y);
    var hashes = vec4(0u);
    var sample = NoiseSample3(0.0, vec3(0.0));
    if (input.mode == {MODE_CORE_HASH}u) {{
        hashes.x = hash_u32(input.words.x, input.words.y, input.words.z, input.words.w);
    }} else if (input.mode == {MODE_LATTICE_HASH}u) {{
        hashes.y = key;
        hashes.z = hash_u32(key, input.words.x, input.words.y, input.words.z);
    }} else if (input.mode == {MODE_BASIS}u) {{
        sample = gradient_noise_3d(input.seed.x, input.seed.y, input.position.xyz);
    }} else if (input.mode == {MODE_FBM}u) {{
        sample = fbm_3d(input.seed.x, input.seed.y, input.position.xyz, input.octaves, input.config.x, input.config.y, input.config.z);
    }} else if (input.mode == {MODE_RIDGED}u) {{
        sample = ridged_multifractal_3d(input.seed.x, input.seed.y, input.position.xyz, input.octaves, input.config.x, input.config.y, input.config.z, input.ridge.x, input.ridge.y);
    }} else if (input.mode == {MODE_DAMPED}u) {{
        sample = derivative_damped_fbm_3d(input.seed.x, input.seed.y, input.position.xyz, input.octaves, input.config.x, input.config.y, input.config.z, input.config.w);
    }}
    outputs[id.x] = Output(hashes, vec4(sample.value, sample.derivative));
}}
"#
    );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("procgen noise mirror"),
        source: wgpu::ShaderSource::Wgsl(harness.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("procgen noise agreement pipeline"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let input_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("procgen noise inputs"),
        contents: bytemuck::cast_slice(inputs),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let output_size = (inputs.len() * size_of::<ShaderOutput>()) as u64;
    let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("procgen noise outputs"),
        size: output_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("procgen noise readback"),
        size: output_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("procgen noise agreement bindings"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output_buffer.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("procgen noise agreement encoder"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("procgen noise agreement pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(inputs.len().div_ceil(64) as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output_buffer, 0, &readback, 0, output_size);
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    let (sender, receiver) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        sender.send(result).unwrap()
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("GPU polling failed");
    receiver
        .recv()
        .unwrap()
        .expect("GPU readback mapping failed");
    let mapped = slice.get_mapped_range();
    let outputs = bytemuck::cast_slice::<u8, ShaderOutput>(&mapped).to_vec();
    drop(mapped);
    readback.unmap();
    outputs
}

fn split_seed(seed: u64) -> [u32; 2] {
    [seed as u32, (seed >> 32) as u32]
}

fn join_seed(seed: [u32; 2]) -> u64 {
    u64::from(seed[0]) | (u64::from(seed[1]) << 32)
}

fn sample_components(sample: NoiseSample3) -> [f32; 4] {
    [
        sample.value,
        sample.derivative.x,
        sample.derivative.y,
        sample.derivative.z,
    ]
}

fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWake(thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}
