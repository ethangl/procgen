// WGSL mirror of procgen-noise's canonical CPU implementation.
// Keep expression order aligned with gradient.rs and fractal.rs. Agreement is
// dispatched through wgpu by the test-only procgen-gpu-tests crate.

const SEED_DOMAIN_TAG: u32 = 0x53454544u;
const NOISE_DOMAIN_TAG: u32 = 0x4e4f4953u;

struct NoiseSample3 {
    value: f32,
    derivative: vec3<f32>,
}

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

fn fold_seed_u64_to_u32(seed_low: u32, seed_high: u32) -> u32 {
    return hash_u32(seed_low, seed_high, SEED_DOMAIN_TAG, NOISE_DOMAIN_TAG);
}

fn lattice_gradient(seed: u32, cell: vec3<i32>) -> vec3<f32> {
    let index = hash_u32(seed, bitcast<u32>(cell.x), bitcast<u32>(cell.y), bitcast<u32>(cell.z)) % 12u;
    switch index {
        case 0u: { return vec3(1.0, 1.0, 0.0); }
        case 1u: { return vec3(-1.0, 1.0, 0.0); }
        case 2u: { return vec3(1.0, -1.0, 0.0); }
        case 3u: { return vec3(-1.0, -1.0, 0.0); }
        case 4u: { return vec3(1.0, 0.0, 1.0); }
        case 5u: { return vec3(-1.0, 0.0, 1.0); }
        case 6u: { return vec3(1.0, 0.0, -1.0); }
        case 7u: { return vec3(-1.0, 0.0, -1.0); }
        case 8u: { return vec3(0.0, 1.0, 1.0); }
        case 9u: { return vec3(0.0, -1.0, 1.0); }
        case 10u: { return vec3(0.0, 1.0, -1.0); }
        default: { return vec3(0.0, -1.0, -1.0); }
    }
}

// Returns the low/high interpolation weights followed by their derivatives.
fn axis_weights(value: f32) -> vec4<f32> {
    let fade = value * value * value * (value * (value * 6.0 - 15.0) + 10.0);
    let fade_derivative = 30.0 * value * value * (value * (value - 2.0) + 1.0);
    return vec4(1.0 - fade, fade, -fade_derivative, fade_derivative);
}

fn gradient_noise_3d_from_key(seed: u32, position: vec3<f32>) -> NoiseSample3 {
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
                let gradient = lattice_gradient(seed, cell + vec3<i32>(corner));
                let displacement = offset - vec3<f32>(corner);
                let contribution = gradient.x * displacement.x
                    + gradient.y * displacement.y
                    + gradient.z * displacement.z;
                let weight = wx[corner_x] * wy[corner_y] * wz[corner_z];
                value += contribution * weight;
                derivative = derivative
                    + gradient * weight
                    + vec3(
                        wx[corner_x + 2u] * wy[corner_y] * wz[corner_z],
                        wx[corner_x] * wy[corner_y + 2u] * wz[corner_z],
                        wx[corner_x] * wy[corner_y] * wz[corner_z + 2u],
                    ) * contribution;
            }
        }
    }
    return NoiseSample3(value, derivative);
}

fn gradient_noise_3d(seed_low: u32, seed_high: u32, position: vec3<f32>) -> NoiseSample3 {
    return gradient_noise_3d_from_key(fold_seed_u64_to_u32(seed_low, seed_high), position);
}

fn scale_sample(sample: NoiseSample3, scale: f32) -> NoiseSample3 {
    return NoiseSample3(sample.value * scale, sample.derivative * scale);
}

fn add_sample(left: NoiseSample3, right: NoiseSample3) -> NoiseSample3 {
    return NoiseSample3(left.value + right.value, left.derivative + right.derivative);
}

fn octave_sample(seed_low: u32, seed_high: u32, position: vec3<f32>, frequency: f32) -> NoiseSample3 {
    var sample = gradient_noise_3d(seed_low, seed_high, position * frequency);
    sample.derivative *= frequency;
    return sample;
}

fn fbm_3d(seed_low: u32, seed_high: u32, position: vec3<f32>, octaves: u32, initial_frequency: f32, lacunarity: f32, gain: f32) -> NoiseSample3 {
    var result = NoiseSample3(0.0, vec3(0.0));
    var frequency = initial_frequency;
    var amplitude = 1.0;
    for (var octave = 0u; octave < octaves; octave++) {
        result = add_sample(result, scale_sample(octave_sample(seed_low, seed_high, position, frequency), amplitude));
        frequency *= lacunarity;
        amplitude *= gain;
    }
    return result;
}

fn ridged_multifractal_3d(seed_low: u32, seed_high: u32, position: vec3<f32>, octaves: u32, initial_frequency: f32, lacunarity: f32, gain: f32, ridge_offset: f32, ridge_gain: f32) -> NoiseSample3 {
    var result = NoiseSample3(0.0, vec3(0.0));
    var weight = NoiseSample3(1.0, vec3(0.0));
    var frequency = initial_frequency;
    var amplitude = 1.0;
    for (var octave = 0u; octave < octaves; octave++) {
        let sample = octave_sample(seed_low, seed_high, position, frequency);
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

fn derivative_damped_fbm_3d(seed_low: u32, seed_high: u32, position: vec3<f32>, octaves: u32, initial_frequency: f32, lacunarity: f32, gain: f32, damping: f32) -> NoiseSample3 {
    var result = NoiseSample3(0.0, vec3(0.0));
    var frequency = initial_frequency;
    var amplitude = 1.0;
    for (var octave = 0u; octave < octaves; octave++) {
        let slope_squared = result.derivative.x * result.derivative.x
            + result.derivative.y * result.derivative.y
            + result.derivative.z * result.derivative.z;
        let attenuation = 1.0 / (1.0 + damping * slope_squared);
        result = add_sample(result, scale_sample(octave_sample(seed_low, seed_high, position, frequency), amplitude * attenuation));
        frequency *= lacunarity;
        amplitude *= gain;
    }
    return result;
}
