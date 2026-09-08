use bytemuck::{Pod, Zeroable};
use procgen_core::{HASH_U32_TEST_VECTORS, ScalarFieldSample3, Vec3};
use procgen_gpu_tests::{readback, request_device};
use procgen_noise::{
    DerivativeDampedConfig, NOISE_DERIVATIVE_ANGLE_TOLERANCE, NOISE_VALUE_TOLERANCE, OctaveConfig,
    OctaveGain, RidgedMultifractalConfig, Validated, WGSL_SOURCE, derivative_damped_fbm_3d, fbm_3d,
    fold_seed_u64_to_u32, gradient_noise_3d, lattice_gradient_3d, ridged_multifractal_3d,
};
use wgpu::util::DeviceExt;

const MODE_BASIS: u32 = 0;
const MODE_FBM: u32 = 1;
const MODE_RIDGED: u32 = 2;
const MODE_DAMPED: u32 = 3;
const MODE_CORE_HASH: u32 = 4;
const MODE_LATTICE_GRADIENT: u32 = 5;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShaderInput {
    words: [u32; 4],
    lattice_cell: [i32; 4],
    position: [f32; 4],
    key: u32,
    mode: u32,
    octaves: u32,
    _padding_0: u32,
    frequency: f32,
    lacunarity: f32,
    gain: f32,
    damping: f32,
    ridge_offset: f32,
    ridge_gain: f32,
    _padding_1: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct ShaderOutput {
    hash: u32,
    _hash_padding: [u32; 3],
    gradient: [f32; 4],
    sample: [f32; 4],
}

#[derive(Clone, Copy)]
struct NoiseParameters {
    octaves: u32,
    frequency: f32,
    lacunarity: f32,
    gain: f32,
    damping: f32,
    ridge_offset: f32,
    ridge_gain: f32,
}

impl NoiseParameters {
    const BASIS: Self = Self {
        octaves: 0,
        frequency: 0.0,
        lacunarity: 0.0,
        gain: 0.0,
        damping: 0.0,
        ridge_offset: 0.0,
        ridge_gain: 0.0,
    };

    fn gain(self) -> OctaveGain {
        OctaveGain::new(self.gain).unwrap()
    }

    fn octave_config(self) -> OctaveConfig {
        OctaveConfig {
            octaves: self.octaves,
            frequency: self.frequency,
            lacunarity: self.lacunarity,
        }
    }

    fn fbm(self) -> Validated<OctaveConfig> {
        self.octave_config().validate().unwrap()
    }

    fn ridged(self) -> Validated<RidgedMultifractalConfig> {
        RidgedMultifractalConfig {
            octaves: self.octave_config(),
            ridge_offset: self.ridge_offset,
            ridge_gain: self.ridge_gain,
        }
        .validate()
        .unwrap()
    }

    fn damped(self) -> Validated<DerivativeDampedConfig> {
        DerivativeDampedConfig {
            octaves: self.octave_config(),
            damping: self.damping,
        }
        .validate()
        .unwrap()
    }
}

#[derive(Clone, Copy)]
enum NoiseKind {
    Basis,
    Fbm,
    Ridged,
    Damped,
}

impl NoiseKind {
    const fn mode(self) -> u32 {
        match self {
            Self::Basis => MODE_BASIS,
            Self::Fbm => MODE_FBM,
            Self::Ridged => MODE_RIDGED,
            Self::Damped => MODE_DAMPED,
        }
    }

    fn canonical_sample(
        self,
        key: u32,
        position: Vec3,
        parameters: NoiseParameters,
    ) -> ScalarFieldSample3 {
        match self {
            Self::Basis => gradient_noise_3d(key, position),
            Self::Fbm => fbm_3d(key, position, parameters.fbm(), parameters.gain()),
            Self::Ridged => ridged_multifractal_3d(
                key,
                position,
                parameters.ridged(),
                parameters.gain(),
                parameters.fbm().full_band(),
            ),
            Self::Damped => derivative_damped_fbm_3d(
                key,
                position,
                parameters.damped(),
                parameters.gain(),
                parameters.fbm().full_band(),
            ),
        }
    }
}

enum Case {
    CoreHash {
        words: [u32; 4],
        expected: u32,
    },
    LatticeGradient {
        label: &'static str,
        key: u32,
        cell: [i32; 3],
        expected: Vec3,
    },
    Noise {
        label: &'static str,
        key: u32,
        kind: NoiseKind,
        position: Vec3,
        parameters: NoiseParameters,
        expected: ScalarFieldSample3,
    },
}

impl Case {
    fn basis(label: &'static str, seed: u64, position: Vec3) -> Self {
        Self::noise(
            label,
            seed,
            NoiseKind::Basis,
            position,
            NoiseParameters::BASIS,
        )
    }

    fn lattice_gradient(label: &'static str, seed: u64, cell: [i32; 3]) -> Self {
        let key = fold_seed_u64_to_u32(seed);
        Self::LatticeGradient {
            label,
            key,
            cell,
            expected: lattice_gradient_3d(key, cell),
        }
    }

    fn noise(
        label: &'static str,
        seed: u64,
        kind: NoiseKind,
        position: Vec3,
        parameters: NoiseParameters,
    ) -> Self {
        let key = fold_seed_u64_to_u32(seed);
        Self::Noise {
            label,
            key,
            kind,
            position,
            parameters,
            expected: kind.canonical_sample(key, position, parameters),
        }
    }

    fn input(&self) -> ShaderInput {
        match *self {
            Self::CoreHash { words, .. } => ShaderInput {
                words,
                mode: MODE_CORE_HASH,
                ..ShaderInput::zeroed()
            },
            Self::LatticeGradient { key, cell, .. } => ShaderInput {
                lattice_cell: [cell[0], cell[1], cell[2], 0],
                key,
                mode: MODE_LATTICE_GRADIENT,
                ..ShaderInput::zeroed()
            },
            Self::Noise {
                key,
                kind,
                position,
                parameters,
                ..
            } => ShaderInput {
                position: [position.x, position.y, position.z, 0.0],
                key,
                mode: kind.mode(),
                octaves: parameters.octaves,
                frequency: parameters.frequency,
                lacunarity: parameters.lacunarity,
                gain: parameters.gain,
                damping: parameters.damping,
                ridge_offset: parameters.ridge_offset,
                ridge_gain: parameters.ridge_gain,
                ..ShaderInput::zeroed()
            },
        }
    }

    fn check(&self, output: &ShaderOutput) -> Option<FloatMeasurement> {
        match *self {
            Self::CoreHash { expected, .. } => {
                assert_eq!(output.hash, expected, "core hash output");
                None
            }
            Self::LatticeGradient {
                label, expected, ..
            } => {
                let expected = [expected.x, expected.y, expected.z];
                for (component, (&actual, &expected)) in
                    output.gradient[..3].iter().zip(&expected).enumerate()
                {
                    assert_eq!(
                        actual.to_bits(),
                        expected.to_bits(),
                        "{label} lattice-gradient component {component}"
                    );
                }
                None
            }
            Self::Noise {
                label, expected, ..
            } => {
                let actual = ScalarFieldSample3 {
                    value: output.sample[0],
                    derivative: Vec3::new(output.sample[1], output.sample[2], output.sample[3]),
                };
                let value_difference = (actual.value - expected.value).abs();
                let derivative_angle = derivative_angle(actual.derivative, expected.derivative);
                assert!(
                    value_difference <= NOISE_VALUE_TOLERANCE,
                    "{label} value: GPU {} CPU {} difference {} exceeds {}",
                    actual.value,
                    expected.value,
                    value_difference,
                    NOISE_VALUE_TOLERANCE
                );
                assert!(
                    derivative_angle <= NOISE_DERIVATIVE_ANGLE_TOLERANCE,
                    "{label} derivative angle {derivative_angle} exceeds {}",
                    NOISE_DERIVATIVE_ANGLE_TOLERANCE
                );
                Some(FloatMeasurement {
                    label,
                    value_difference,
                    derivative_angle,
                })
            }
        }
    }
}

#[derive(Clone, Copy)]
struct FloatMeasurement {
    label: &'static str,
    value_difference: f32,
    derivative_angle: f32,
}

#[test]
fn wgsl_noise_agrees_with_canonical_cpu() {
    let Some((adapter_info, device, queue)) = request_device("procgen noise agreement device")
    else {
        return;
    };
    println!(
        "GPU adapter: {} ({:?}, {:?})",
        adapter_info.name, adapter_info.backend, adapter_info.device_type
    );

    let cases = agreement_cases();
    let inputs: Vec<_> = cases.iter().map(Case::input).collect();
    let outputs = dispatch(&device, &queue, &inputs);
    assert_eq!(outputs.len(), cases.len());

    let measurements: Vec<_> = cases
        .iter()
        .zip(&outputs)
        .filter_map(|(case, output)| case.check(output))
        .collect();
    let maximum_value = measurements
        .iter()
        .max_by(|left, right| left.value_difference.total_cmp(&right.value_difference))
        .unwrap();
    let maximum_derivative = measurements
        .iter()
        .max_by(|left, right| left.derivative_angle.total_cmp(&right.derivative_angle))
        .unwrap();
    println!(
        "noise agreement: {} float samples, maximum value difference {:.9e} at {}, tolerance {:.1e}; maximum derivative angle {:.9e} radians at {}, tolerance {:.1e}",
        measurements.len(),
        maximum_value.value_difference,
        maximum_value.label,
        NOISE_VALUE_TOLERANCE,
        maximum_derivative.derivative_angle,
        maximum_derivative.label,
        NOISE_DERIVATIVE_ANGLE_TOLERANCE,
    );
}

fn agreement_cases() -> Vec<Case> {
    let mut cases: Vec<_> = HASH_U32_TEST_VECTORS
        .iter()
        .map(|&(words, expected)| Case::CoreHash { words, expected })
        .collect();

    // Fold on the host, then exercise signed-cell bitcasts and gradient selection on the GPU.
    cases.extend([
        Case::lattice_gradient("lattice-origin", 0x0000_0000_0000_0001, [0, 0, 0]),
        Case::lattice_gradient("lattice-signed", 0x0000_0001_0000_0000, [-1, 2, -3]),
        Case::lattice_gradient(
            "lattice-extremes",
            0x0123_4567_89ab_cdef,
            [i32::MIN, -17, i32::MAX],
        ),
        Case::lattice_gradient("lattice-wide", u64::MAX, [-32_769, -1, 32_768]),
    ]);

    cases.extend([
        Case::basis("basis-low-half", 1, Vec3::new(0.25, 0.5, 0.75)),
        Case::basis(
            "basis-high-half",
            0x0000_0001_0000_0000,
            Vec3::new(-1.25, 2.5, -9.75),
        ),
        Case::basis(
            "basis-mixed",
            0x0123_4567_89ab_cdef,
            Vec3::new(12.345, -67.89, 0.125),
        ),
    ]);

    let configurations = [
        (
            [
                ("fbm-a", NoiseKind::Fbm),
                ("ridged-a", NoiseKind::Ridged),
                ("damped-a", NoiseKind::Damped),
            ],
            0x0000_0001_0000_0000,
            Vec3::new(-1.25, 2.5, -9.75),
            NoiseParameters {
                octaves: 3,
                frequency: 0.75,
                lacunarity: 2.0,
                gain: 0.5,
                damping: 0.75,
                ridge_offset: 1.0,
                ridge_gain: 2.0,
            },
        ),
        (
            [
                ("fbm-b", NoiseKind::Fbm),
                ("ridged-b", NoiseKind::Ridged),
                ("damped-b", NoiseKind::Damped),
            ],
            0x0123_4567_89ab_cdef,
            Vec3::new(12.345, -67.89, 0.125),
            NoiseParameters {
                octaves: 5,
                frequency: 1.25,
                lacunarity: 1.75,
                gain: 0.625,
                damping: 1.5,
                ridge_offset: 1.2,
                ridge_gain: 1.4,
            },
        ),
        (
            [
                ("fbm-eleven-octave", NoiseKind::Fbm),
                ("ridged-eleven-octave", NoiseKind::Ridged),
                ("damped-eleven-octave", NoiseKind::Damped),
            ],
            u64::MAX,
            Vec3::new(0.577_350_26, -0.577_350_26, 0.577_350_26),
            NoiseParameters {
                octaves: 11,
                frequency: 0.75,
                lacunarity: 2.0,
                gain: 0.5,
                damping: 1.0,
                ridge_offset: 1.0,
                ridge_gain: 2.0,
            },
        ),
    ];
    for (labels, seed, position, parameters) in configurations {
        for (label, kind) in labels {
            cases.push(Case::noise(label, seed, kind, position, parameters));
        }
    }
    cases
}

fn derivative_angle(left: Vec3, right: Vec3) -> f32 {
    left.cross(right).length().atan2(left.dot(right))
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
    lattice_cell: vec4<i32>,
    position: vec4<f32>,
    key: u32,
    mode: u32,
    octaves: u32,
    padding_0: u32,
    frequency: f32,
    lacunarity: f32,
    gain: f32,
    damping: f32,
    ridge_offset: f32,
    ridge_gain: f32,
    padding_1: vec2<u32>,
}}
struct Output {{
    hash: u32,
    hash_padding_0: u32,
    hash_padding_1: u32,
    hash_padding_2: u32,
    gradient: vec4<f32>,
    sample: vec4<f32>,
}}
@group(0) @binding(0) var<storage, read> inputs: array<Input>;
@group(0) @binding(1) var<storage, read_write> outputs: array<Output>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if (id.x >= arrayLength(&inputs)) {{ return; }}
    let input = inputs[id.x];
    var hash = 0u;
    var gradient = vec4(0.0);
    var sample = ScalarFieldSample3(0.0, vec3(0.0));
    if (input.mode == {MODE_CORE_HASH}u) {{
        hash = hash_u32(input.words.x, input.words.y, input.words.z, input.words.w);
    }} else if (input.mode == {MODE_LATTICE_GRADIENT}u) {{
        gradient = vec4(lattice_gradient(input.key, input.lattice_cell.xyz), 0.0);
    }} else if (input.mode == {MODE_BASIS}u) {{
        sample = gradient_noise_3d(input.key, input.position.xyz);
    }} else if (input.mode == {MODE_FBM}u) {{
        sample = fbm_3d(input.key, input.position.xyz, input.octaves, input.frequency, input.lacunarity, input.gain);
    }} else if (input.mode == {MODE_RIDGED}u) {{
        sample = ridged_multifractal_3d(input.key, input.position.xyz, noise_full_octave_band(input.octaves), input.frequency, input.lacunarity, input.gain, input.ridge_offset, input.ridge_gain);
    }} else if (input.mode == {MODE_DAMPED}u) {{
        sample = derivative_damped_fbm_3d(input.key, input.position.xyz, noise_full_octave_band(input.octaves), input.frequency, input.lacunarity, input.gain, input.damping);
    }}
    outputs[id.x] = Output(hash, 0u, 0u, 0u, gradient, vec4(sample.value, sample.derivative));
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
    queue.submit(Some(encoder.finish()));
    readback(device, queue, &output_buffer, inputs.len())
}
