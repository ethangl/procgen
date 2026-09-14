// WGSL mirror of procgen-noise's canonical CPU implementation.
// Keep expression order aligned with gradient.rs and fractal.rs. Agreement is
// dispatched through wgpu by the test-only procgen-gpu-tests crate.

struct ScalarFieldSample3 {
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

fn lattice_gradient(key: u32, cell: vec3<i32>) -> vec3<f32> {
    let index = hash_u32(
        key,
        bitcast<u32>(cell.x),
        bitcast<u32>(cell.y),
        bitcast<u32>(cell.z),
    ) % 12u;
    return GRADIENTS[index];
}

fn axis_weights(value: f32, math: F32Arithmetic) -> AxisWeights {
    let polynomial = f32_add(f32_mul(value, f32_add(f32_mul(value, 6.0, math), -15.0, math), math), 10.0, math);
    let fade = f32_mul(f32_mul(f32_mul(value, value, math), value, math), polynomial, math);
    let derivative_polynomial = f32_add(f32_mul(value, f32_add(value, -2.0, math), math), 1.0, math);
    let fade_derivative = f32_mul(f32_mul(f32_mul(30.0, value, math), value, math), derivative_polynomial, math);
    return AxisWeights(vec2(f32_add(1.0, -fade, math), fade), vec2(-fade_derivative, fade_derivative));
}

fn gradient_noise_3d(key: u32, position: vec3<f32>) -> ScalarFieldSample3 {
    return gradient_noise_3d_with_arithmetic(key, position, F32Arithmetic(1.0, 0.0));
}

// math.one must be 1.0 and math.zero must be 0.0, supplied through a buffer
// when preserving f32 operation boundaries against backend reassociation.
fn gradient_noise_3d_with_arithmetic(key: u32, position: vec3<f32>, math: F32Arithmetic) -> ScalarFieldSample3 {
    let cell = vec3<i32>(floor(position));
    let offset = position - vec3<f32>(cell);
    let wx = axis_weights(offset.x, math);
    let wy = axis_weights(offset.y, math);
    let wz = axis_weights(offset.z, math);
    var value = 0.0;
    var derivative = vec3(0.0);
    for (var corner_z = 0u; corner_z < 2u; corner_z++) {
        for (var corner_y = 0u; corner_y < 2u; corner_y++) {
            for (var corner_x = 0u; corner_x < 2u; corner_x++) {
                let corner = vec3(corner_x, corner_y, corner_z);
                let gradient = lattice_gradient(key, cell + vec3<i32>(corner));
                let displacement = offset - vec3<f32>(corner);
                let contribution = f32_add(f32_add(
                    f32_mul(gradient.x, displacement.x, math),
                    f32_mul(gradient.y, displacement.y, math), math),
                    f32_mul(gradient.z, displacement.z, math), math);
                let weight = f32_mul(f32_mul(wx.weight[corner_x], wy.weight[corner_y], math), wz.weight[corner_z], math);
                value = f32_add(value, f32_mul(contribution, weight, math), math);
                derivative = f32_scale_add(vec3(
                    f32_mul(f32_mul(wx.derivative[corner_x], wy.weight[corner_y], math), wz.weight[corner_z], math),
                    f32_mul(f32_mul(wx.weight[corner_x], wy.derivative[corner_y], math), wz.weight[corner_z], math),
                    f32_mul(f32_mul(wx.weight[corner_x], wy.weight[corner_y], math), wz.derivative[corner_z], math),
                ), contribution, f32_scale_add(gradient, weight, derivative, math), math);
            }
        }
    }
    return ScalarFieldSample3(value, derivative);
}

fn scale_sample(sample: ScalarFieldSample3, scale: f32) -> ScalarFieldSample3 {
    return ScalarFieldSample3(sample.value * scale, sample.derivative * scale);
}

fn add_sample(left: ScalarFieldSample3, right: ScalarFieldSample3) -> ScalarFieldSample3 {
    return ScalarFieldSample3(left.value + right.value, left.derivative + right.derivative);
}

fn sub_sample(left: ScalarFieldSample3, right: ScalarFieldSample3) -> ScalarFieldSample3 {
    return ScalarFieldSample3(left.value - right.value, left.derivative - right.derivative);
}

fn mul_sample(left: ScalarFieldSample3, right: ScalarFieldSample3) -> ScalarFieldSample3 {
    return ScalarFieldSample3(
        left.value * right.value,
        left.derivative * right.value + right.derivative * left.value,
    );
}

fn neg_sample(sample: ScalarFieldSample3) -> ScalarFieldSample3 {
    return ScalarFieldSample3(-sample.value, -sample.derivative);
}

fn octave_sample(key: u32, position: vec3<f32>, frequency: f32) -> ScalarFieldSample3 {
    var sample = gradient_noise_3d(key, position * frequency);
    sample.derivative *= frequency;
    return sample;
}

fn fbm_3d(key: u32, position: vec3<f32>, octaves: u32, initial_frequency: f32, lacunarity: f32, gain: f32) -> ScalarFieldSample3 {
    var result = ScalarFieldSample3(0.0, vec3(0.0));
    var frequency = initial_frequency;
    var amplitude = 1.0;
    for (var octave = 0u; octave < octaves; octave++) {
        result = add_sample(result, scale_sample(octave_sample(key, position, frequency), amplitude));
        frequency *= lacunarity;
        amplitude *= gain;
    }
    return result;
}

struct OctaveBand {
    octaves: u32,
    newest_weight: f32,
}

fn noise_full_octave_band(octaves: u32) -> OctaveBand {
    return OctaveBand(octaves, 1.0);
}

fn noise_octave_band_for_minimum_wavelength(maximum_octaves: u32, initial_frequency: f32, lacunarity: f32, minimum_wavelength: f32) -> OctaveBand {
    var frequency = initial_frequency;
    var octaves = 0u;
    var newest_weight = 0.0;
    while octaves < maximum_octaves {
        // A cubic-gradient lattice feature spans roughly two lattice cells.
        let wavelength = 2.0 / frequency;
        if wavelength < minimum_wavelength { break; }
        octaves += 1u;
        newest_weight = clamp(wavelength / minimum_wavelength - 1.0, 0.0, 1.0);
        frequency *= lacunarity;
    }
    return OctaveBand(octaves, newest_weight);
}

fn ridged_multifractal_3d(key: u32, position: vec3<f32>, band: OctaveBand, initial_frequency: f32, lacunarity: f32, gain: f32, ridge_offset: f32, ridge_gain: f32) -> ScalarFieldSample3 {
    var result = ScalarFieldSample3(0.0, vec3(0.0));
    var weight = ScalarFieldSample3(1.0, vec3(0.0));
    var frequency = initial_frequency;
    var amplitude = 1.0;
    for (var octave = 0u; octave < band.octaves; octave++) {
        let sample = octave_sample(key, position, frequency);
        var absolute_derivative = vec3(0.0);
        if (sample.value > 0.0) {
            absolute_derivative = sample.derivative;
        } else if (sample.value < 0.0) {
            absolute_derivative = -sample.derivative;
        }
        let ridge = ridge_offset - abs(sample.value);
        let signal = ScalarFieldSample3(ridge * ridge, -absolute_derivative * (2.0 * ridge));
        let weighted = ScalarFieldSample3(
            signal.value * weight.value,
            signal.derivative * weight.value + weight.derivative * signal.value,
        );
        let fade = select(1.0, band.newest_weight, octave + 1u == band.octaves);
        result = add_sample(result, scale_sample(weighted, amplitude * fade));

        let next_weight = scale_sample(weighted, ridge_gain);
        if (next_weight.value > 0.0 && next_weight.value < 1.0) {
            weight = next_weight;
        } else {
            weight = ScalarFieldSample3(clamp(next_weight.value, 0.0, 1.0), vec3(0.0));
        }
        frequency *= lacunarity;
        amplitude *= gain;
    }
    return result;
}

fn derivative_damped_fbm_3d(key: u32, position: vec3<f32>, band: OctaveBand, initial_frequency: f32, lacunarity: f32, gain: f32, damping: f32) -> ScalarFieldSample3 {
    var result = ScalarFieldSample3(0.0, vec3(0.0));
    var frequency = initial_frequency;
    var amplitude = 1.0;
    for (var octave = 0u; octave < band.octaves; octave++) {
        // As above, preserve Rust's scalar expression order instead of using
        // length() or dot(), whose lowering may contract differently.
        let slope = result.derivative * (1.0 / initial_frequency);
        let slope_squared = slope.x * slope.x
            + slope.y * slope.y
            + slope.z * slope.z;
        let attenuation = 1.0 / (1.0 + damping * slope_squared);
        let fade = select(1.0, band.newest_weight, octave + 1u == band.octaves);
        result = add_sample(result, scale_sample(octave_sample(key, position, frequency), amplitude * attenuation * fade));
        frequency *= lacunarity;
        amplitude *= gain;
    }
    return result;
}
