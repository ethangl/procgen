struct Frame {
    clip: mat4x4<f32>, anchor: vec4<i32>, style: vec4<u32>,
    times: vec4<f32>, local_lo: vec4<f32>, local_hi: vec4<f32>,
}
@group(0) @binding(0) var<uniform> frame: Frame;
struct Vertex {
    @builtin(position) clip: vec4<f32>,
    @location(0) position: vec3<f32>,
    @location(1) @interpolate(flat) lod: u32,
}
fn local_weight(p: vec3<f32>) -> f32 {
    let d = min(p-frame.local_lo.xyz,frame.local_hi.xyz-p);
    let edge = min(d.x,min(d.y,d.z));
    return smoothstep(0.0,frame.local_lo.w,edge) * clamp((frame.times.x-frame.times.z)/frame.times.w,0.0,1.0) * frame.local_hi.w;
}
@vertex fn vertex(@location(0) anchor: vec4<i32>, @location(1) offset: vec4<f32>, @location(2) origin: vec4<i32>) -> Vertex {
    var out: Vertex;
    out.position = vec3<f32>(origin.xyz + anchor.xyz - frame.anchor.xyz) + offset.xyz;
    out.clip = frame.clip * vec4<f32>(out.position,1.0);
    out.lod = u32(origin.w);
    return out;
}
@vertex fn height_vertex(@location(0) anchor: vec4<i32>, @location(1) offset: vec4<f32>) -> Vertex {
    var out: Vertex;
    out.position = vec3<f32>(anchor.xyz-frame.anchor.xyz)+offset.xyz;
    // The complete height surface remains below the local overlap. Voxel edges
    // dissolve into it instead of cutting holes in a different triangulation.
    let radial = normalize(vec3<f32>(anchor.xyz)+offset.xyz);
    out.position -= radial * bitcast<f32>(frame.style.y) * local_weight(out.position);
    out.clip = frame.clip * vec4(out.position,1.0);
    out.lod = u32(max(0.0,ceil(log2(offset.w))));
    return out;
}
fn screen_threshold(p: vec2<f32>) -> f32 {
    let xy = vec2<u32>(p);
    var h = xy.x * 1664525u + xy.y * 1013904223u;
    h = (h ^ (h >> 16u)) * 2246822519u;
    return f32(h & 255u)/256.0;
}
fn shade(in: Vertex, front: bool) -> vec4<f32> {
    var normal = normalize(cross(dpdy(in.position),dpdx(in.position)));
    if !front { normal = -normal; }
    if frame.style.x == 2u { return vec4<f32>(normal*0.5+0.5,1.0); }
    if frame.style.x == 1u {
        let hue = f32((in.lod*47u)%360u)/60.0;
        let rgb = clamp(abs((vec3<f32>(hue)+vec3<f32>(0.0,4.0,2.0))%6.0-3.0)-1.0,vec3<f32>(0.0),vec3<f32>(1.0));
        return vec4<f32>(mix(vec3<f32>(0.175),vec3<f32>(0.825),rgb),1.0);
    }
    let light = normalize(vec3<f32>(1.0,0.5,0.6));
    let intensity = 0.12 + 0.9*max(dot(normal,light),0.0);
    return vec4<f32>(vec3<f32>(0.52)*intensity,1.0);
}
@fragment fn fragment(in: Vertex, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    if screen_threshold(in.clip.xy) >= local_weight(in.position) { discard; }
    return shade(in,front);
}
@fragment fn height_fragment(in: Vertex, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let blend = clamp((frame.times.x-frame.times.y)/frame.times.w,0.0,1.0);
    if frame.style.z != 0u && screen_threshold(in.clip.xy) >= blend { discard; }
    return shade(in,front);
}
@fragment fn old_height_fragment(in: Vertex, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let blend = clamp((frame.times.x-frame.times.y)/frame.times.w,0.0,1.0);
    if screen_threshold(in.clip.xy) < blend { discard; }
    return shade(in,front);
}
