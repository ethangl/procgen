// Full-band mirror of planet_design.rs and voxel_density.rs. The host supplies
// canonical shape constants and composes the shared hash/noise source above.
struct PilotOctave {
    wavelength_m: f32,
    amplitude_m: f32,
    sharpness: f32,
    perturbation: f32,
    slope_erosion: f32,
    altitude_erosion: f32,
    ridge_erosion: f32,
    enabled: u32,
}
struct PilotVolume {
    wavelength_m: f32,
    amplitude_m: f32,
    sharpness: f32,
    fade_m: f32,
    enabled: u32,
    key: u32,
    pad_0: u32,
    pad_1: u32,
}
struct PilotParameters {
    radius_m: f32,
    height_limit_m: f32,
    key: u32,
    octave_count: u32,
    arithmetic: vec4<f32>,
    volume: PilotVolume,
    octaves: array<PilotOctave, PILOT_MAX_OCTAVES>,
}
struct PilotChunk {
    // Scalar members avoid packed-int3 arithmetic in Metal shader translation.
    origin_x_m: i32,
    origin_y_m: i32,
    origin_z_m: i32,
    spacing_m: i32,
}
@group(0) @binding(0) var<uniform> pilot: PilotParameters;
@group(0) @binding(1) var<storage, read> pilot_chunks: array<PilotChunk>;
@group(0) @binding(2) var<storage, read_write> pilot_potentials: array<f32>;

// Runtime identities retain explicit f32 rounding boundaries through Metal's
// fast-math compilation. They are fixed by the host packing contract, not knobs.
fn pilot_sum(a: f32, b: f32) -> f32 { return f32_add(a, b, F32Arithmetic(pilot.arithmetic.x, pilot.arithmetic.y)); }
fn pilot_product(a: f32, b: f32) -> f32 { return f32_mul(a, b, F32Arithmetic(pilot.arithmetic.x, pilot.arithmetic.y)); }
fn pilot_scale_add(a: vec3<f32>, b: f32, c: vec3<f32>) -> vec3<f32> {
    return f32_scale_add(a, b, c, F32Arithmetic(pilot.arithmetic.x, pilot.arithmetic.y));
}
fn pilot_divide(a: f32, b: f32) -> f32 {
    let q = a / b;
    return fma(fma(-q, b, a), 1.0 / b, q);
}
fn pilot_sqrt(x: f32) -> f32 {
    if x == 0.0 { return 0.0; }
    var inverse = bitcast<f32>(PILOT_SQRT_SEED - (bitcast<u32>(x) >> 1u));
    for (var i = 0u; i < PILOT_SQRT_ITERATIONS; i++) {
        inverse = pilot_product(inverse, pilot_sum(1.5,
            -pilot_product(pilot_product(0.5 * x, inverse), inverse)));
    }
    let root = pilot_product(x, inverse);
    return fma(-fma(root, root, -x), 0.5 * inverse, root);
}
fn pilot_length_squared(p: vec3<f32>) -> f32 {
    return pilot_sum(pilot_sum(pilot_product(p.x, p.x), pilot_product(p.y, p.y)), pilot_product(p.z, p.z));
}

fn pilot_shape(n: f32, sharpness: f32) -> f32 {
    let base = n / PILOT_BASIS_STD;
    var shaped: f32;
    if sharpness >= 0.0 {
        shaped = (n * n - PILOT_SQUARE_MEAN) / PILOT_SQUARE_STD;
    } else {
        shaped = (PILOT_ABS_MEAN - abs(n)) / PILOT_ABS_STD;
    }
    return base + (shaped - base) * abs(sharpness);
}

fn pilot_filtered_height(direction: vec3<f32>, spacing_m: f32) -> f32 {
    var sum = 0.0;
    var slope = vec3(0.0);
    var ridge = vec3(0.0);
    var warp = vec3(0.0);
    for (var i = 0u; i < pilot.octave_count; i++) {
        let o = pilot.octaves[i];
        if o.enabled == 0u || o.amplitude_m == 0.0 { continue; }
        var weight = 1.0;
        if spacing_m > 0.0 {
            let t = clamp((o.wavelength_m / spacing_m - 2.0) * 0.5, 0.0, 1.0);
            weight = t * t * (3.0 - 2.0 * t);
        }
        if weight == 0.0 { continue; }
        let p = pilot_scale_add(direction, pilot_divide(pilot.radius_m, o.wavelength_m), warp);
        let sample = scale_sample(gradient_noise_3d_with_arithmetic(pilot.key, p, F32Arithmetic(pilot.arithmetic.x, pilot.arithmetic.y)), PILOT_NOISE_SCALE);
        let d = sample.derivative * weight;
        slope = pilot_scale_add(d, o.slope_erosion, slope);
        ridge = pilot_scale_add(d, o.ridge_erosion, ridge);
        let altitude = clamp(sum / pilot.height_limit_m, 0.0, 1.0);
        let altitude_weight = altitude * altitude * (3.0 - 2.0 * altitude);
        let damping = (1.0 + (altitude_weight - 1.0) * o.altitude_erosion)
            * (1.0 - o.ridge_erosion / (1.0 + pilot_length_squared(ridge)))
            / (1.0 + pilot_length_squared(slope));
        sum += weight * o.amplitude_m * pilot_shape(sample.value, o.sharpness) * damping;
        warp = pilot_scale_add(d, o.perturbation, warp);
    }
    let relative = sum / pilot.height_limit_m;
    return sum / pilot_sqrt(1.0 + relative * relative);
}
fn pilot_height(direction: vec3<f32>) -> f32 {
    return pilot_filtered_height(direction, 0.0);
}

struct PilotCompensated {
    high: f32,
    low: f32,
}
fn pilot_add(a: PilotCompensated, b: PilotCompensated) -> PilotCompensated {
    let sum = pilot_sum(a.high, b.high);
    let virtual_rhs = pilot_sum(sum, -a.high);
    let error = pilot_sum(pilot_sum(a.high, -pilot_sum(sum, -virtual_rhs)), pilot_sum(b.high, -virtual_rhs));
    let tail = pilot_sum(error, pilot_sum(a.low, b.low));
    let high = pilot_sum(sum, tail);
    return PilotCompensated(high, pilot_sum(tail, -pilot_sum(high, -sum)));
}
fn pilot_square(value: f32) -> PilotCompensated {
    let split = pilot_product(4097.0, value);
    let high = pilot_sum(split, -pilot_sum(split, -value));
    let low = pilot_sum(value, -high);
    let product = pilot_product(value, value);
    let error = pilot_sum(pilot_sum(pilot_sum(pilot_product(high, high), -product), pilot_product(high, low)), pilot_product(low, high));
    return PilotCompensated(product, pilot_sum(error, pilot_product(low, low)));
}

// Mirror of volume_shape_at in voxel_density.rs: the unfaded term, shared by
// the density path and the far-field projection. The shaped noise passes
// through the same x / sqrt(1 + x*x) soft bound the height stack uses, so
// amplitude_m is a true bound on the term and the render bias can rely on it.
fn pilot_volume_shape(p: vec3<f32>) -> f32 {
    if pilot.volume.enabled == 0u || pilot.volume.amplitude_m == 0.0 { return 0.0; }
    let q = vec3(
        pilot_divide(p.x, pilot.volume.wavelength_m),
        pilot_divide(p.y, pilot.volume.wavelength_m),
        pilot_divide(p.z, pilot.volume.wavelength_m),
    );
    let sample = scale_sample(gradient_noise_3d_with_arithmetic(pilot.volume.key, q, F32Arithmetic(pilot.arithmetic.x, pilot.arithmetic.y)), PILOT_NOISE_SCALE);
    let shaped = pilot_shape(sample.value, pilot.volume.sharpness);
    let bounded = shaped / pilot_sqrt(pilot_sum(1.0, pilot_product(shaped, shaped)));
    return pilot_product(pilot.volume.amplitude_m, bounded);
}

// Mirror of volume_at in voxel_density.rs. No spacing filter, by design: the
// density path evaluates the full stack so coincident halo and parent samples
// stay identical across LODs. The fade uses d, positive below the surface, so
// the term reaches zero fade_m above it and cannot float free above that.
fn pilot_volume(p: vec3<f32>, d: f32) -> f32 {
    if pilot.volume.enabled == 0u || pilot.volume.amplitude_m == 0.0 { return 0.0; }
    var fade = 1.0;
    if d < 0.0 {
        if d <= -pilot.volume.fade_m { return 0.0; }
        let t = clamp(pilot_divide(pilot_sum(d, pilot.volume.fade_m), pilot.volume.fade_m), 0.0, 1.0);
        fade = pilot_product(pilot_product(t, t), pilot_sum(3.0, -pilot_product(2.0, t)));
    }
    return pilot_product(pilot_volume_shape(p), fade);
}

// Mirror of PlanetDesignField::surface_height: the band's surface projected
// back onto the radial ray, for the far field to draw. On that surface d is
// zero and the fade is one, so only the shape remains, filtered by the tile's
// footprint with the same smoothstep pilot_filtered_height applies to octaves.
// P is built the way the host builds it, so the two sides round alike.
fn pilot_surface_height(direction: vec3<f32>, footprint_m: f32) -> f32 {
    let height = pilot_filtered_height(direction, footprint_m);
    if pilot.volume.enabled == 0u || pilot.volume.amplitude_m == 0.0 { return height; }
    var weight = 1.0;
    if footprint_m > 0.0 {
        let t = clamp((pilot.volume.wavelength_m / footprint_m - 2.0) * 0.5, 0.0, 1.0);
        weight = t * t * (3.0 - 2.0 * t);
    }
    if weight == 0.0 { return height; }
    let p = pilot_scale_add(direction, pilot_sum(pilot.radius_m, height), vec3(0.0));
    return pilot_sum(height, pilot_product(weight, pilot_volume_shape(p)));
}

fn pilot_potential(position: vec3<i32>) -> f32 {
    let p = vec3<f32>(position);
    let radius_squared = pilot_square(pilot.radius_m);
    let difference = pilot_add(
        pilot_add(pilot_add(pilot_square(p.x), pilot_square(p.y)), pilot_square(p.z)),
        PilotCompensated(-radius_squared.high, -radius_squared.low),
    );
    let distance = pilot_sqrt(pilot_length_squared(p));
    let altitude = pilot_divide(pilot_sum(difference.high, difference.low), pilot_sum(distance, pilot.radius_m));
    var height = 0.0;
    if any(position != vec3(0)) {
        height = pilot_height(pilot_scale_add(p, pilot_divide(1.0, distance), vec3(0.0)));
    }
    let d = height - altitude;
    return d + pilot_volume(p, d);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let chunk_index = id.x / PILOT_SAMPLE_COUNT;
    if chunk_index >= arrayLength(&pilot_chunks) { return; }
    let sample = id.x % PILOT_SAMPLE_COUNT;
    let local = vec3<i32>(vec3<u32>(
        sample % PILOT_SAMPLE_SIDE,
        (sample / PILOT_SAMPLE_SIDE) % PILOT_SAMPLE_SIDE,
        sample / (PILOT_SAMPLE_SIDE * PILOT_SAMPLE_SIDE),
    )) - vec3(PILOT_HALO);
    let chunk = pilot_chunks[chunk_index];
    pilot_potentials[id.x] = pilot_potential(vec3(chunk.origin_x_m, chunk.origin_y_m, chunk.origin_z_m) + local * chunk.spacing_m);
}

// Transition stencils request canonical integer source points. Record xyz is a
// point rather than a chunk origin; spacing is unused by this entry point.
@compute @workgroup_size(64)
fn sample_points(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= arrayLength(&pilot_chunks) { return; }
    let point = pilot_chunks[id.x];
    pilot_potentials[id.x] = pilot_potential(vec3(point.origin_x_m, point.origin_y_m, point.origin_z_m));
}
