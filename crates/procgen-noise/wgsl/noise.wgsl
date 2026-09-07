// WGSL mirror of procgen-noise's canonical CPU implementation.
// Keep expression order aligned with gradient.rs and fractal.rs. Agreement is
// dispatched through wgpu by the test-only procgen-gpu-tests crate.

struct NoiseSample3 {
    value: f32,
    derivative: vec3<f32>,
}

struct AxisWeights {
    weight: vec2<f32>,
    derivative: vec2<f32>,
}

const GRADIENTS = array<vec3<f32>, 12>(
    vec3(1.0, 1.0, 0.0),
    vec3(-1.0, 1.0, 0.0),
    vec3(1.0, -1.0, 0.0),
    vec3(-1.0, -1.0, 0.0),
    vec3(1.0, 0.0, 1.0),
    vec3(-1.0, 0.0, 1.0),
    vec3(1.0, 0.0, -1.0),
    vec3(-1.0, 0.0, -1.0),
    vec3(0.0, 1.0, 1.0),
    vec3(0.0, -1.0, 1.0),
    vec3(0.0, 1.0, -1.0),
    vec3(0.0, -1.0, -1.0),
);

fn hash_u32(word0: u32, word1: u32, word2: u32, word3: u32) -> u32 {
    var value = (word0 + 0x9e3779b9u)
        ^ (word1 * 0x85ebca6bu)
        ^ (word2 * 0xc2b2ae35u)
        ^ (word3 * 0x27d4eb2fu);
    value ^= value >> 16u;
    value *= 0x7feb352du;
    value ^= value >> 15u;
    value *= 0x846ca68bu;
    return value ^ (value >> 16u);
}

fn lattice_gradient(key: u32, cell: vec3<i32>) -> vec3<f32> {
    let index = hash_u32(
        key,
        bitcast<u32>(cell.x),
        bitcast<u32>(cell.y),
        bitcast<u32>(cell.z),
    ) % 12u;
    return GRADIENTS[index];
}

fn axis_weights(value: f32) -> AxisWeights {
    let fade = value * value * value * (value * (value * 6.0 - 15.0) + 10.0);
    let fade_derivative = 30.0 * value * value * (value * (value - 2.0) + 1.0);
    return AxisWeights(
        vec2(1.0 - fade, fade),
        vec2(-fade_derivative, fade_derivative),
    );
}

fn gradient_noise_3d(key: u32, position: vec3<f32>) -> NoiseSample3 {
    let cell = vec3<i32>(floor(position));
    let offset = position - vec3<f32>(cell);
    let wx = axis_weights(offset.x);
    let wy = axis_weights(offset.y);
    let wz = axis_weights(offset.z);
    var value = 0.0;
    var derivative = vec3(0.0);

    for (var corner_z = 0u; corner_z < 2u; corner_z++) {
        for (var corner_y = 0u; corner_y < 2u; corner_y++) {
            for (var corner_x = 0u; corner_x < 2u; corner_x++) {
                let corner = vec3(corner_x, corner_y, corner_z);
                let gradient = lattice_gradient(key, cell + vec3<i32>(corner));
                let displacement = offset - vec3<f32>(corner);
                // Keep these scalar operations explicit so a builtin cannot
                // introduce a different contraction policy than canonical Rust.
                let contribution = gradient.x * displacement.x
                    + gradient.y * displacement.y
                    + gradient.z * displacement.z;
                let weight = wx.weight[corner_x]
                    * wy.weight[corner_y]
                    * wz.weight[corner_z];
                value += contribution * weight;
                derivative = derivative
                    + gradient * weight
                    + vec3(
                        wx.derivative[corner_x]
                            * wy.weight[corner_y]
                            * wz.weight[corner_z],
                        wx.weight[corner_x]
                            * wy.derivative[corner_y]
                            * wz.weight[corner_z],
                        wx.weight[corner_x]
                            * wy.weight[corner_y]
                            * wz.derivative[corner_z],
                    ) * contribution;
            }
        }
    }
    return NoiseSample3(value, derivative);
}

fn scale_sample(sample: NoiseSample3, scale: f32) -> NoiseSample3 {
    return NoiseSample3(sample.value * scale, sample.derivative * scale);
}

fn add_sample(left: NoiseSample3, right: NoiseSample3) -> NoiseSample3 {
    return NoiseSample3(left.value + right.value, left.derivative + right.derivative);
}

fn octave_sample(key: u32, position: vec3<f32>, frequency: f32) -> NoiseSample3 {
    var sample = gradient_noise_3d(key, position * frequency);
    sample.derivative *= frequency;
    return sample;
}

fn fbm_3d(key: u32, position: vec3<f32>, octaves: u32, initial_frequency: f32, lacunarity: f32, gain: f32) -> NoiseSample3 {
    var result = NoiseSample3(0.0, vec3(0.0));
    var frequency = initial_frequency;
    var amplitude = 1.0;
    for (var octave = 0u; octave < octaves; octave++) {
        result = add_sample(result, scale_sample(octave_sample(key, position, frequency), amplitude));
        frequency *= lacunarity;
        amplitude *= gain;
    }
    return result;
}

fn ridged_multifractal_3d(key: u32, position: vec3<f32>, octaves: u32, initial_frequency: f32, lacunarity: f32, gain: f32, ridge_offset: f32, ridge_gain: f32) -> NoiseSample3 {
    var result = NoiseSample3(0.0, vec3(0.0));
    var weight = NoiseSample3(1.0, vec3(0.0));
    var frequency = initial_frequency;
    var amplitude = 1.0;
    for (var octave = 0u; octave < octaves; octave++) {
        let sample = octave_sample(key, position, frequency);
        var absolute_derivative = vec3(0.0);
        if (sample.value > 0.0) {
            absolute_derivative = sample.derivative;
        } else if (sample.value < 0.0) {
            absolute_derivative = -sample.derivative;
        }
        let ridge = ridge_offset - abs(sample.value);
        let signal = NoiseSample3(ridge * ridge, -absolute_derivative * (2.0 * ridge));
        let weighted = NoiseSample3(
            signal.value * weight.value,
            signal.derivative * weight.value + weight.derivative * signal.value,
        );
        result = add_sample(result, scale_sample(weighted, amplitude));

        let next_weight = scale_sample(weighted, ridge_gain);
        if (next_weight.value > 0.0 && next_weight.value < 1.0) {
            weight = next_weight;
        } else {
            weight = NoiseSample3(clamp(next_weight.value, 0.0, 1.0), vec3(0.0));
        }
        frequency *= lacunarity;
        amplitude *= gain;
    }
    return result;
}

fn derivative_damped_fbm_3d(key: u32, position: vec3<f32>, octaves: u32, initial_frequency: f32, lacunarity: f32, gain: f32, damping: f32) -> NoiseSample3 {
    var result = NoiseSample3(0.0, vec3(0.0));
    var frequency = initial_frequency;
    var amplitude = 1.0;
    for (var octave = 0u; octave < octaves; octave++) {
        // As above, preserve Rust's scalar expression order instead of using
        // length() or dot(), whose lowering may contract differently.
        let slope_squared = result.derivative.x * result.derivative.x
            + result.derivative.y * result.derivative.y
            + result.derivative.z * result.derivative.z;
        let attenuation = 1.0 / (1.0 + damping * slope_squared);
        result = add_sample(result, scale_sample(octave_sample(key, position, frequency), amplitude * attenuation));
        frequency *= lacunarity;
        amplitude *= gain;
    }
    return result;
}
