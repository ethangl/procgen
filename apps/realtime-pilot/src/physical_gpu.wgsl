struct Vertex {
    @builtin(position) clip: vec4<f32>,
    @location(0) position: vec3<f32>,
    @location(1) @interpolate(flat) lod: u32,
    @location(2) normal: vec3<f32>,
    @location(3) altitude_m: f32,
}
@vertex fn height_vertex(@location(0) anchor: vec4<i32>, @location(1) offset: vec4<f32>, @location(3) normal: vec4<f32>) -> Vertex {
    var out: Vertex;
    out.position = vec3<f32>(anchor.xyz-frame.anchor.xyz)+offset.xyz;
    out.clip = frame.clip * vec4(out.position,1.0);
    out.lod = u32(max(0.0,ceil(log2(offset.w))));
    out.normal = normal.xyz;
    // Color follows field altitude.
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
@fragment fn height_fragment(in: Vertex) -> @location(0) vec4<f32> {
    return shade(in,normalize(in.normal));
}
