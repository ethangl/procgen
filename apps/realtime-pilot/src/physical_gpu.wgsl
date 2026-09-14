struct Vertex {
    @builtin(position) clip: vec4<f32>,
    @location(0) position: vec3<f32>,
    @location(1) @interpolate(flat) lod: u32,
    @location(2) normal: vec3<f32>,
    @location(3) altitude_m: f32,
}
fn local_weight(p: vec3<f32>) -> f32 {
    let d = min(p-frame.local_lo.xyz,frame.local_hi.xyz-p);
    let edge = min(d.x,min(d.y,d.z));
    return smoothstep(0.0,frame.local_lo.w,edge) * clamp((frame.times.x-frame.times.z)/frame.times.w,0.0,1.0) * frame.local_hi.w;
}
@vertex fn vertex(@location(0) anchor: vec4<i32>, @location(1) offset: vec4<f32>, @location(2) origin: vec4<i32>, @location(3) normal: vec4<f32>) -> Vertex {
    var out: Vertex;
    out.position = vec3<f32>(origin.xyz + anchor.xyz - frame.anchor.xyz) + offset.xyz;
    out.clip = frame.clip * vec4<f32>(out.position,1.0);
    out.lod = u32(origin.w);
    out.normal = normal.xyz;
    let planet_position = vec3<f32>(origin.xyz + anchor.xyz) + offset.xyz;
    out.altitude_m = length(planet_position) - frame.height_scale.x;
    return out;
}
@vertex fn height_vertex(@location(0) anchor: vec4<i32>, @location(1) offset: vec4<f32>, @location(3) normal: vec4<f32>) -> Vertex {
    var out: Vertex;
    out.position = vec3<f32>(anchor.xyz-frame.anchor.xyz)+offset.xyz;
    // The complete height surface remains below the local overlap. Voxel edges
    // dissolve into it instead of cutting holes in a different triangulation.
    let radial = normalize(vec3<f32>(anchor.xyz)+offset.xyz);
    out.position -= radial * bitcast<f32>(frame.style.y) * local_weight(out.position);
    out.clip = frame.clip * vec4(out.position,1.0);
    out.lod = u32(max(0.0,ceil(log2(offset.w))));
    out.normal = normal.xyz;
    // Skirts retain their top altitude, independent of the overlap depth bias.
    out.altitude_m = normal.w;
    return out;
}
fn shade(in: Vertex, normal: vec3<f32>) -> vec4<f32> {
    if frame.style.x == 2u { return vec4<f32>(normal*0.5+0.5,1.0); }
    if frame.style.x == 1u {
        let hue = f32((in.lod*47u)%360u)/60.0;
        let rgb = clamp(abs((vec3<f32>(hue)+vec3<f32>(0.0,4.0,2.0))%6.0-3.0)-1.0,vec3<f32>(0.0),vec3<f32>(1.0));
        return vec4<f32>(mix(vec3<f32>(0.175),vec3<f32>(0.825),rgb),1.0);
    }
    let light = normalize(vec3<f32>(1.0,0.5,0.6));
    let intensity = 0.12 + 0.9*max(dot(normal,light),0.0);
    var color = vec3<f32>(0.52);
    if frame.style.x == 3u { color = height_color(in.altitude_m/frame.height_scale.y); }
    return vec4<f32>(color*intensity,1.0);
}
@fragment fn fragment(in: Vertex, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let weight = local_weight(in.position);
    if weight == 0.0 { discard; }
    // Keep the nearest local surface opaque within its own depth layer.
    // Alpha is the coverage used by the final surface compositor.
    var normal = normalize(cross(dpdy(in.position),dpdx(in.position)));
    if !front { normal = -normal; }
    // A critical density point has no gradient direction. Transition vertices
    // also request geometric shading until they have a shared normal stencil.
    if dot(in.normal,in.normal) > 0.0 { normal = normalize(in.normal); }
    return vec4<f32>(shade(in,normal).rgb, weight);
}
@fragment fn height_fragment(in: Vertex) -> @location(0) vec4<f32> {
    return shade(in,normalize(in.normal));
}
