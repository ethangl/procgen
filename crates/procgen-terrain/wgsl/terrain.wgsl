// WGSL mirror of procgen-terrain's canonical CPU height and fixed-level tile
// implementation. Keep expression order aligned with procgen-cubesphere's
// mapping/field modules and procgen-terrain's coast/height/stamp modules.
// Consumers provide terrain_load_control_texel, terrain_load_stamp, and
// terrain_parameters so this source does not own a wgpu bind-group layout.

const TERRAIN_PI_OVER_FOUR: f32 = 0.7853981633974483;
const TERRAIN_SEA_LEVEL: f32 = 0.5;
const TERRAIN_NOISE_VECTOR_BOUND: f32 = 3.4641016;
const TERRAIN_TILE_QUADS: u32 = 64u;

struct TerrainControlTexel {
    channels_0: vec4<f32>,
    channels_1: vec4<f32>,
}

struct TerrainStamp {
    position_strength: vec4<f32>,
    kind_padding: vec4<u32>,
}

struct TerrainParameters {
    dimensions: vec4<u32>,
    noise_keys_0: vec4<u32>,
    noise_keys_1: vec4<u32>,
    detail_octaves: vec4<u32>,
    detail: vec4<f32>,
    abyssal: vec4<f32>,
    coast: vec4<f32>,
    stamp_profiles: array<vec4<f32>, 4>,
}

struct TerrainFaceFrame {
    normal: vec3<f32>,
    u_axis: vec3<f32>,
    v_axis: vec3<f32>,
}

struct TerrainFaceCoordinates {
    face: u32,
    u: f32,
    v: f32,
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

fn terrain_face_frame(face: u32) -> TerrainFaceFrame {
    switch face {
        case 0u: { return TerrainFaceFrame(vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, -1.0), vec3(0.0, 1.0, 0.0)); }
        case 1u: { return TerrainFaceFrame(vec3(-1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 0.0)); }
        case 2u: { return TerrainFaceFrame(vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, -1.0)); }
        case 3u: { return TerrainFaceFrame(vec3(0.0, -1.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0)); }
        case 4u: { return TerrainFaceFrame(vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)); }
        default: { return TerrainFaceFrame(vec3(0.0, 0.0, -1.0), vec3(-1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)); }
    }
}

fn terrain_unit_direction(coordinates: TerrainFaceCoordinates) -> vec3<f32> {
    let frame = terrain_face_frame(coordinates.face);
    let a = tan(coordinates.u * TERRAIN_PI_OVER_FOUR);
    let b = tan(coordinates.v * TERRAIN_PI_OVER_FOUR);
    return normalize(frame.normal + frame.u_axis * a + frame.v_axis * b);
}

fn terrain_dominant_face(direction: vec3<f32>) -> u32 {
    if abs(direction.x) >= abs(direction.y) && abs(direction.x) >= abs(direction.z) {
        return select(1u, 0u, direction.x >= 0.0);
    }
    if abs(direction.y) >= abs(direction.z) {
        return select(3u, 2u, direction.y >= 0.0);
    }
    return select(5u, 4u, direction.z >= 0.0);
}

fn terrain_project_direction(direction: vec3<f32>, face: u32) -> TerrainFaceCoordinates {
    let frame = terrain_face_frame(face);
    let depth = dot(direction, frame.normal);
    return TerrainFaceCoordinates(
        face,
        atan(dot(direction, frame.u_axis) / depth) / TERRAIN_PI_OVER_FOUR,
        atan(dot(direction, frame.v_axis) / depth) / TERRAIN_PI_OVER_FOUR,
    );
}

fn terrain_face_to_texel(coordinate: f32, resolution: u32) -> f32 {
    return (coordinate + 1.0) * 0.5 * f32(resolution) - 0.5;
}

fn terrain_texel_to_face(texel: f32, resolution: u32) -> f32 {
    return 2.0 * (texel + 0.5) / f32(resolution) - 1.0;
}

fn terrain_texel_index(face: u32, x: u32, y: u32, resolution: u32) -> u32 {
    return (face * resolution + y) * resolution + x;
}

fn terrain_nearest_texel(coordinates: TerrainFaceCoordinates, resolution: u32) -> TerrainControlTexel {
    let x = u32(clamp(round(terrain_face_to_texel(coordinates.u, resolution)), 0.0, f32(resolution) - 1.0));
    let y = u32(clamp(round(terrain_face_to_texel(coordinates.v, resolution)), 0.0, f32(resolution) - 1.0));
    return terrain_load_control_texel(terrain_texel_index(coordinates.face, x, y, resolution));
}

fn terrain_add_texels(left: TerrainControlTexel, right: TerrainControlTexel) -> TerrainControlTexel {
    return TerrainControlTexel(left.channels_0 + right.channels_0, left.channels_1 + right.channels_1);
}

fn terrain_scale_texel(texel: TerrainControlTexel, scale: f32) -> TerrainControlTexel {
    return TerrainControlTexel(texel.channels_0 * scale, texel.channels_1 * scale);
}

fn terrain_control_tap(source_face: u32, x: i32, y: i32, resolution: u32) -> TerrainControlTexel {
    let edge = i32(resolution);
    let x_inside = x >= 0 && x < edge;
    let y_inside = y >= 0 && y < edge;
    if x_inside && y_inside {
        return terrain_load_control_texel(terrain_texel_index(source_face, u32(x), u32(y), resolution));
    }
    if !x_inside && !y_inside {
        let corner = terrain_unit_direction(TerrainFaceCoordinates(
            source_face,
            select(1.0, -1.0, x < 0),
            select(1.0, -1.0, y < 0),
        ));
        var sum = TerrainControlTexel(vec4(0.0), vec4(0.0));
        var count = 0.0;
        for (var face = 0u; face < 6u; face++) {
            if dot(corner, terrain_face_frame(face).normal) > 0.0 {
                sum = terrain_add_texels(sum, terrain_nearest_texel(terrain_project_direction(corner, face), resolution));
                count += 1.0;
            }
        }
        return terrain_scale_texel(sum, 1.0 / count);
    }
    let outside = TerrainFaceCoordinates(
        source_face,
        terrain_texel_to_face(f32(x), resolution),
        terrain_texel_to_face(f32(y), resolution),
    );
    let adjacent_direction = terrain_unit_direction(outside);
    let adjacent = terrain_project_direction(adjacent_direction, terrain_dominant_face(adjacent_direction));
    return terrain_nearest_texel(adjacent, resolution);
}

fn terrain_channel(texel: TerrainControlTexel, channel: u32) -> f32 {
    if channel < 4u { return texel.channels_0[channel]; }
    return texel.channels_1.x;
}

fn terrain_control_channel(
    lower_left: TerrainControlTexel,
    lower_right: TerrainControlTexel,
    upper_left: TerrainControlTexel,
    upper_right: TerrainControlTexel,
    channel: u32,
    tx: f32,
    ty: f32,
    tx_derivative: vec3<f32>,
    ty_derivative: vec3<f32>,
) -> ScalarFieldSample3 {
    let ll = terrain_channel(lower_left, channel);
    let lr = terrain_channel(lower_right, channel);
    let ul = terrain_channel(upper_left, channel);
    let ur = terrain_channel(upper_right, channel);
    let lower = ll + (lr - ll) * tx;
    let upper = ul + (ur - ul) * tx;
    let along_x = (lr - ll) * (1.0 - ty) + (ur - ul) * ty;
    let along_y = (ul - ll) * (1.0 - tx) + (ur - lr) * tx;
    return ScalarFieldSample3(
        lower + (upper - lower) * ty,
        tx_derivative * along_x + ty_derivative * along_y,
    );
}

fn terrain_sample_controls(direction: vec3<f32>) -> TerrainControlSample {
    let parameters = terrain_parameters();
    let resolution = parameters.dimensions.x;
    let face = terrain_dominant_face(direction);
    let coordinates = terrain_project_direction(direction, face);
    let x = terrain_face_to_texel(coordinates.u, resolution);
    let y = terrain_face_to_texel(coordinates.v, resolution);
    let x_floor = floor(x);
    let y_floor = floor(y);
    let tx = x - x_floor;
    let ty = y - y_floor;
    let x0 = i32(x_floor);
    let y0 = i32(y_floor);
    let lower_left = terrain_control_tap(face, x0, y0, resolution);
    let lower_right = terrain_control_tap(face, x0 + 1, y0, resolution);
    let upper_left = terrain_control_tap(face, x0, y0 + 1, resolution);
    let upper_right = terrain_control_tap(face, x0 + 1, y0 + 1, resolution);

    let frame = terrain_face_frame(face);
    let depth = dot(direction, frame.normal);
    let inverse_depth_squared = 1.0 / (depth * depth);
    let side_u = dot(direction, frame.u_axis);
    let side_v = dot(direction, frame.v_axis);
    let ratio_u = side_u / depth;
    let ratio_v = side_v / depth;
    let derivative_u = (frame.u_axis * depth - frame.normal * side_u)
        * (inverse_depth_squared / (TERRAIN_PI_OVER_FOUR * (1.0 + ratio_u * ratio_u)));
    let derivative_v = (frame.v_axis * depth - frame.normal * side_v)
        * (inverse_depth_squared / (TERRAIN_PI_OVER_FOUR * (1.0 + ratio_v * ratio_v)));
    let texel_scale = f32(resolution) * 0.5;
    let tx_derivative = derivative_u * texel_scale;
    let ty_derivative = derivative_v * texel_scale;
    return TerrainControlSample(
        terrain_control_channel(lower_left, lower_right, upper_left, upper_right, 0u, tx, ty, tx_derivative, ty_derivative),
        terrain_control_channel(lower_left, lower_right, upper_left, upper_right, 1u, tx, ty, tx_derivative, ty_derivative),
        terrain_control_channel(lower_left, lower_right, upper_left, upper_right, 2u, tx, ty, tx_derivative, ty_derivative),
        terrain_control_channel(lower_left, lower_right, upper_left, upper_right, 3u, tx, ty, tx_derivative, ty_derivative),
        terrain_control_channel(lower_left, lower_right, upper_left, upper_right, 4u, tx, ty, tx_derivative, ty_derivative),
    );
}

fn terrain_sub_sample(left: ScalarFieldSample3, right: ScalarFieldSample3) -> ScalarFieldSample3 {
    return ScalarFieldSample3(left.value - right.value, left.derivative - right.derivative);
}

fn terrain_mul_sample(left: ScalarFieldSample3, right: ScalarFieldSample3) -> ScalarFieldSample3 {
    return ScalarFieldSample3(
        left.value * right.value,
        left.derivative * right.value + right.derivative * left.value,
    );
}

fn terrain_neg_sample(sample: ScalarFieldSample3) -> ScalarFieldSample3 {
    return ScalarFieldSample3(-sample.value, -sample.derivative);
}

fn terrain_coast_taper(base: ScalarFieldSample3, half_width: f32) -> ScalarFieldSample3 {
    var distance = ScalarFieldSample3(base.value - TERRAIN_SEA_LEVEL, base.derivative);
    if distance.value < 0.0 { distance = terrain_neg_sample(distance); }
    if distance.value >= half_width { return ScalarFieldSample3(1.0, vec3(0.0)); }
    let t = scale_sample(distance, 1.0 / half_width);
    return terrain_mul_sample(terrain_mul_sample(t, t), terrain_sub_sample(ScalarFieldSample3(3.0, vec3(0.0)), scale_sample(t, 2.0)));
}

fn terrain_transpose_product(row0: vec3<f32>, row1: vec3<f32>, row2: vec3<f32>, vector: vec3<f32>) -> vec3<f32> {
    return row0 * vector.x + row1 * vector.y + row2 * vector.z;
}

fn terrain_coast_warp(direction: vec3<f32>, weight: ScalarFieldSample3) -> TerrainDomainWarp {
    let parameters = terrain_parameters();
    let frequency = parameters.coast.y;
    let sample_x = gradient_noise_3d(parameters.noise_keys_0.z, direction * frequency);
    let sample_y = gradient_noise_3d(parameters.noise_keys_0.w, direction * frequency);
    let sample_z = gradient_noise_3d(parameters.noise_keys_1.x, direction * frequency);
    let raw = vec3(sample_x.value, sample_y.value, sample_z.value);
    let raw_derivative_0 = sample_x.derivative * frequency;
    let raw_derivative_1 = sample_y.derivative * frequency;
    let raw_derivative_2 = sample_z.derivative * frequency;
    let raw_dot_source = dot(raw, direction);
    let tangent = raw - direction * raw_dot_source;
    let maximum_scale = parameters.coast.z / TERRAIN_NOISE_VECTOR_BOUND;
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

fn terrain_stamp_contribution(direction: vec3<f32>, stamp: TerrainStamp, profile: vec4<f32>) -> ScalarFieldSample3 {
    let displacement = direction - stamp.position_strength.xyz;
    let radius_squared = profile.x * profile.x;
    let normalized_squared = dot(displacement, displacement) / radius_squared;
    if normalized_squared >= 1.0 { return ScalarFieldSample3(0.0, vec3(0.0)); }
    let distance = ScalarFieldSample3(normalized_squared, displacement * (2.0 / radius_squared));
    let support = terrain_sub_sample(ScalarFieldSample3(1.0, vec3(0.0)), distance);
    let squared = terrain_mul_sample(support, support);
    var capped = squared;
    if profile.z == 3.0 { capped = terrain_mul_sample(squared, support); }
    return scale_sample(capped, profile.y * stamp.position_strength.w);
}

fn terrain_height_gpu(direction: vec3<f32>) -> ScalarFieldSample3 {
    let parameters = terrain_parameters();
    let original = terrain_sample_controls(direction);
    let taper = terrain_coast_taper(original.base_elevation, parameters.coast.x);
    let warp = terrain_coast_warp(direction, terrain_sub_sample(ScalarFieldSample3(1.0, vec3(0.0)), taper));
    let controls = terrain_pullback_controls(warp, terrain_sample_controls(warp.direction));
    let gain = controls.octave_gain.value;
    let fbm = terrain_pullback(warp, derivative_damped_fbm_3d(
        parameters.noise_keys_0.x, warp.direction, parameters.detail_octaves.x,
        parameters.detail.x, parameters.detail.y, gain, parameters.detail.z,
    ));
    let ridged = terrain_pullback(warp, ridged_multifractal_3d(
        parameters.noise_keys_0.x, warp.direction, parameters.detail_octaves.x,
        parameters.detail.x, parameters.detail.y, gain, parameters.detail.w, parameters.abyssal.w,
    ));
    let blended = add_sample(fbm, terrain_mul_sample(terrain_sub_sample(ridged, fbm), controls.ridge_weight));
    let abyssal = terrain_pullback(warp, derivative_damped_fbm_3d(
        parameters.noise_keys_0.y, warp.direction, parameters.detail_octaves.y,
        parameters.abyssal.x, parameters.abyssal.y, gain, parameters.abyssal.z,
    ));
    let additive = add_sample(
        terrain_mul_sample(controls.detail_amplitude, blended),
        terrain_mul_sample(controls.abyssal_amplitude, abyssal),
    );
    var height = add_sample(controls.base_elevation, terrain_mul_sample(taper, additive));
    for (var index = 0u; index < parameters.dimensions.y; index++) {
        let stamp = terrain_load_stamp(index);
        height = add_sample(height, terrain_stamp_contribution(direction, stamp, parameters.stamp_profiles[stamp.kind_padding.x]));
    }
    if height.value <= 0.0 || height.value >= 1.0 {
        return ScalarFieldSample3(clamp(height.value, 0.0, 1.0), vec3(0.0));
    }
    return ScalarFieldSample3(height.value, height.derivative - direction * dot(height.derivative, direction));
}

fn terrain_tile_direction(address: vec4<u32>, local: vec2<u32>) -> vec3<f32> {
    let quads_per_axis = TERRAIN_TILE_QUADS << address.y;
    let grid_x = address.z * TERRAIN_TILE_QUADS + local.x;
    let grid_y = address.w * TERRAIN_TILE_QUADS + local.y;
    let u = -1.0 + 2.0 * f32(grid_x) / f32(quads_per_axis);
    let v = -1.0 + 2.0 * f32(grid_y) / f32(quads_per_axis);
    return terrain_unit_direction(TerrainFaceCoordinates(address.x, u, v));
}
