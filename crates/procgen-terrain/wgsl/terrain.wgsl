// WGSL mirror of procgen-terrain's canonical coast, stamp, and height code.
// Consumers provide cubesphere_load_field_texel, terrain_load_stamp, and
// terrain_parameters so this source does not own a wgpu bind-group layout.

const TERRAIN_SEA_LEVEL: f32 = 0.5;
const TERRAIN_NOISE_VECTOR_BOUND: f32 = 3.4641016;
const TERRAIN_STAMP_CAP_CUBIC: u32 = 3u;

struct TerrainStamp {
    position: vec3<f32>,
    strength: f32,
    kind: u32,
    padding_0: u32,
    padding_1: u32,
    padding_2: u32,
}

struct TerrainStampProfile {
    radius: f32,
    amplitude: f32,
    cap: u32,
    padding: u32,
}

struct TerrainParameters {
    control_resolution: u32,
    stamp_count: u32,
    detail_key: u32,
    abyssal_key: u32,
    coast_warp_key_x: u32,
    coast_warp_key_y: u32,
    coast_warp_key_z: u32,
    detail_octaves: u32,
    abyssal_octaves: u32,
    padding_0: u32,
    padding_1: u32,
    padding_2: u32,
    detail_frequency: f32,
    detail_lacunarity: f32,
    detail_derivative_damping: f32,
    detail_ridge_offset: f32,
    detail_ridge_gain: f32,
    abyssal_frequency: f32,
    abyssal_lacunarity: f32,
    abyssal_derivative_damping: f32,
    coast_half_width: f32,
    coast_warp_frequency: f32,
    coast_maximum_warp: f32,
    padding_3: f32,
    stamp_profiles: array<TerrainStampProfile, 4>,
}

struct TerrainControlSample {
    base_elevation: ScalarFieldSample3,
    detail_amplitude: ScalarFieldSample3,
    ridge_weight: ScalarFieldSample3,
    octave_gain: ScalarFieldSample3,
    abyssal_amplitude: ScalarFieldSample3,
}

struct TerrainDomainWarp {
    direction: vec3<f32>,
    jacobian_transpose_0: vec3<f32>,
    jacobian_transpose_1: vec3<f32>,
    jacobian_transpose_2: vec3<f32>,
}

fn terrain_sample_controls(direction: vec3<f32>) -> TerrainControlSample {
    let sample = cubesphere_sample_field(direction, terrain_parameters().control_resolution);
    return TerrainControlSample(
        sample.channels[0],
        sample.channels[1],
        sample.channels[2],
        sample.channels[3],
        sample.channels[4],
    );
}

fn terrain_coast_taper(base: ScalarFieldSample3, half_width: f32) -> ScalarFieldSample3 {
    var distance = ScalarFieldSample3(base.value - TERRAIN_SEA_LEVEL, base.derivative);
    if distance.value < 0.0 { distance = neg_sample(distance); }
    if distance.value >= half_width { return ScalarFieldSample3(1.0, vec3(0.0)); }
    let t = scale_sample(distance, 1.0 / half_width);
    return mul_sample(mul_sample(t, t), sub_sample(ScalarFieldSample3(3.0, vec3(0.0)), scale_sample(t, 2.0)));
}

fn terrain_transpose_product(row0: vec3<f32>, row1: vec3<f32>, row2: vec3<f32>, vector: vec3<f32>) -> vec3<f32> {
    return row0 * vector.x + row1 * vector.y + row2 * vector.z;
}

fn terrain_coast_warp(direction: vec3<f32>, weight: ScalarFieldSample3) -> TerrainDomainWarp {
    let parameters = terrain_parameters();
    let frequency = parameters.coast_warp_frequency;
    let sample_x = gradient_noise_3d(parameters.coast_warp_key_x, direction * frequency);
    let sample_y = gradient_noise_3d(parameters.coast_warp_key_y, direction * frequency);
    let sample_z = gradient_noise_3d(parameters.coast_warp_key_z, direction * frequency);
    let raw = vec3(sample_x.value, sample_y.value, sample_z.value);
    let raw_derivative_0 = sample_x.derivative * frequency;
    let raw_derivative_1 = sample_y.derivative * frequency;
    let raw_derivative_2 = sample_z.derivative * frequency;
    let raw_dot_source = dot(raw, direction);
    let tangent = raw - direction * raw_dot_source;
    let maximum_scale = parameters.coast_maximum_warp / TERRAIN_NOISE_VECTOR_BOUND;
    let scale = maximum_scale * weight.value;
    let scale_derivative = weight.derivative * maximum_scale;
    let displaced = direction + tangent * scale;
    let inverse_length = 1.0 / length(displaced);
    let warped = displaced * inverse_length;
    let projection_derivative = terrain_transpose_product(raw_derivative_0, raw_derivative_1, raw_derivative_2, direction) + raw;
    var rows: array<vec3<f32>, 3>;
    let axes = array(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0));
    for (var index = 0u; index < 3u; index++) {
        let axis = axes[index];
        let normalized = (axis - warped * dot(axis, warped)) * inverse_length;
        rows[index] = normalized
            + scale_derivative * dot(normalized, tangent)
            + (terrain_transpose_product(raw_derivative_0, raw_derivative_1, raw_derivative_2, normalized)
                - normalized * raw_dot_source
                - projection_derivative * dot(normalized, direction)) * scale;
    }
    return TerrainDomainWarp(warped, rows[0], rows[1], rows[2]);
}

fn terrain_pullback(warp: TerrainDomainWarp, sample: ScalarFieldSample3) -> ScalarFieldSample3 {
    return ScalarFieldSample3(
        sample.value,
        terrain_transpose_product(warp.jacobian_transpose_0, warp.jacobian_transpose_1, warp.jacobian_transpose_2, sample.derivative),
    );
}

fn terrain_pullback_controls(warp: TerrainDomainWarp, controls: TerrainControlSample) -> TerrainControlSample {
    return TerrainControlSample(
        terrain_pullback(warp, controls.base_elevation),
        terrain_pullback(warp, controls.detail_amplitude),
        terrain_pullback(warp, controls.ridge_weight),
        terrain_pullback(warp, controls.octave_gain),
        terrain_pullback(warp, controls.abyssal_amplitude),
    );
}

fn terrain_stamp_contribution(direction: vec3<f32>, stamp: TerrainStamp, profile: TerrainStampProfile) -> ScalarFieldSample3 {
    let displacement = direction - stamp.position;
    let radius_squared = profile.radius * profile.radius;
    let normalized_squared = dot(displacement, displacement) / radius_squared;
    if normalized_squared >= 1.0 { return ScalarFieldSample3(0.0, vec3(0.0)); }
    let distance = ScalarFieldSample3(normalized_squared, displacement * (2.0 / radius_squared));
    let support = sub_sample(ScalarFieldSample3(1.0, vec3(0.0)), distance);
    let squared = mul_sample(support, support);
    var capped = squared;
    if profile.cap == TERRAIN_STAMP_CAP_CUBIC { capped = mul_sample(squared, support); }
    return scale_sample(capped, profile.amplitude * stamp.strength);
}

fn terrain_height_gpu(direction: vec3<f32>) -> ScalarFieldSample3 {
    let parameters = terrain_parameters();
    let original = terrain_sample_controls(direction);
    let taper = terrain_coast_taper(original.base_elevation, parameters.coast_half_width);
    let warp = terrain_coast_warp(direction, sub_sample(ScalarFieldSample3(1.0, vec3(0.0)), taper));
    let controls = terrain_pullback_controls(warp, terrain_sample_controls(warp.direction));
    let gain = controls.octave_gain.value;
    let fbm = terrain_pullback(warp, derivative_damped_fbm_3d(
        parameters.detail_key, warp.direction, parameters.detail_octaves,
        parameters.detail_frequency, parameters.detail_lacunarity, gain, parameters.detail_derivative_damping,
    ));
    let ridged = terrain_pullback(warp, ridged_multifractal_3d(
        parameters.detail_key, warp.direction, parameters.detail_octaves,
        parameters.detail_frequency, parameters.detail_lacunarity, gain,
        parameters.detail_ridge_offset, parameters.detail_ridge_gain,
    ));
    let blended = add_sample(fbm, mul_sample(sub_sample(ridged, fbm), controls.ridge_weight));
    let abyssal = terrain_pullback(warp, derivative_damped_fbm_3d(
        parameters.abyssal_key, warp.direction, parameters.abyssal_octaves,
        parameters.abyssal_frequency, parameters.abyssal_lacunarity, gain,
        parameters.abyssal_derivative_damping,
    ));
    let additive = add_sample(
        mul_sample(controls.detail_amplitude, blended),
        mul_sample(controls.abyssal_amplitude, abyssal),
    );
    var height = add_sample(controls.base_elevation, mul_sample(taper, additive));
    for (var index = 0u; index < parameters.stamp_count; index++) {
        let stamp = terrain_load_stamp(index);
        height = add_sample(height, terrain_stamp_contribution(direction, stamp, parameters.stamp_profiles[stamp.kind]));
    }
    if height.value <= 0.0 || height.value >= 1.0 {
        return ScalarFieldSample3(clamp(height.value, 0.0, 1.0), vec3(0.0));
    }
    return ScalarFieldSample3(height.value, height.derivative - direction * dot(height.derivative, direction));
}
