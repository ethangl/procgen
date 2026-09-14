// Roots of t² + 2bt + c. The host computes c = altitude * (2R + altitude).
// q and c/q avoid cancellation for the near root at large planet radii.
// Returns entry and exit distances, or (-1,-1) when the ray misses.
fn ocean_interval(ray: vec3<f32>) -> vec2<f32> {
    let b = dot(frame.ocean_radial.xyz, ray) * frame.ocean_radial.w;
    let c = frame.ocean_sphere.z;
    let discriminant = b*b - c;
    if discriminant < 0.0 { return vec2(-1.0); }
    let root = sqrt(discriminant);
    let q = -b - select(-root, root, b >= 0.0);
    if q == 0.0 { return vec2(0.0); }
    let other = c / q;
    return vec2(min(q,other), max(q,other));
}
fn relative_position(ndc: vec2<f32>, depth: f32) -> vec3<f32> {
    let p = frame.inverse_clip * vec4(ndc,depth,1.0);
    return p.xyz / p.w;
}
fn ocean_surface(surface: Surface, ndc: vec2<f32>, ray: vec3<f32>) -> Surface {
    if frame.ocean_sphere.w == 0.0 { return surface; }
    let interval = ocean_interval(ray);
    if interval.y <= 0.0 { return surface; }
    let underwater = frame.ocean_sphere.y < 0.0;
    let entry = max(interval.x,0.0);
    var terrain_distance = 1e30;
    if surface.depth > 0.0 {
        terrain_distance = length(relative_position(ndc,surface.depth) - frame.eye.xyz);
    }
    if !underwater && terrain_distance <= entry { return surface; }
    let water_end = min(terrain_distance, interval.y);
    let thickness = max(water_end-entry,0.0);
    // Per-channel absorption leaves shallow terrain visible without inventing a shore mask.
    let transmission = exp(-vec3(0.055,0.018,0.009)*thickness);
    let deep = vec3(0.008,0.055,0.085);
    var color = mix(deep, surface.color.rgb, transmission);
    var depth = surface.depth;
    if underwater {
        if terrain_distance > interval.y {
            color = mix(deep,vec3(0.22,0.40,0.52),transmission);
            let clip = frame.clip * vec4(frame.eye.xyz + ray*interval.y,1.0);
            depth = clamp(clip.z/clip.w,0.0,1.0);
        }
        return Surface(vec4(color,1.0),depth);
    }
    let normal = normalize(frame.ocean_radial.xyz + ray*(entry/frame.ocean_radial.w));
    let cosine = clamp(dot(normal,-ray),0.0,1.0);
    let fresnel = 0.02 + 0.98*pow(1.0-cosine,5.0);
    let reflected = reflect(ray,normal);
    let sky = mix(vec3(0.38,0.48,0.58),vec3(0.10,0.24,0.42),max(dot(reflected,normal),0.0));
    let sun = normalize(vec3(1.0,0.5,0.6));
    let daylight = 0.12 + 0.9*max(dot(normal,sun),0.0);
    color = mix(color*daylight,sky*daylight,fresnel);
    color += vec3(1.0,0.88,0.65)*pow(max(dot(reflected,sun),0.0),512.0)*2.0;
    let clip = frame.clip * vec4(frame.eye.xyz + ray*entry,1.0);
    // A camera on the waterline can place the entry before the near plane.
    depth = select(1.0,clamp(clip.z/clip.w,0.0,1.0),clip.w > 0.0);
    return Surface(vec4(color,1.0),depth);
}
