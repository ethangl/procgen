struct Frame { clip: mat4x4<f32>, anchor: vec4<i32>, style: vec4<u32> }
@group(0) @binding(0) var<uniform> frame: Frame;
struct Vertex {
    @builtin(position) clip: vec4<f32>,
    @location(0) position: vec3<f32>,
    @location(1) @interpolate(flat) lod: u32,
}
@vertex fn vertex(@location(0) anchor: vec4<i32>, @location(1) offset: vec4<f32>, @location(2) origin: vec4<i32>) -> Vertex {
    var out: Vertex;
    out.position = vec3<f32>(origin.xyz + anchor.xyz - frame.anchor.xyz) + offset.xyz;
    out.clip = frame.clip * vec4<f32>(out.position,1.0);
    out.lod = u32(origin.w);
    return out;
}
@fragment fn fragment(in: Vertex, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
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
