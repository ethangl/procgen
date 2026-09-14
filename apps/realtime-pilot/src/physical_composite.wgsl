@group(1) @binding(0) var colors: texture_2d_array<f32>;
@group(1) @binding(1) var depths: texture_depth_2d_array;

@vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
struct Surface {
    color: vec4<f32>,
    depth: f32,
}
fn read_surface(p: vec2<i32>, layer: i32) -> Surface {
    return Surface(textureLoad(colors,p,layer,0), textureLoad(depths,p,layer,0));
}
struct Output {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}
@fragment fn fragment(@builtin(position) p: vec4<f32>) -> Output {
    let pixel = vec2<i32>(p.xy);
    var ndc = vec2(0.0);
    var ray = vec3(0.0);
    if frame.ocean_sphere.w != 0.0 {
        ndc = ((p.xy-frame.viewport.xy)/frame.viewport.zw)*vec2(2.0,-2.0)+vec2(-1.0,1.0);
        ray = normalize(relative_position(ndc,1.0)-frame.eye.xyz);
    }
    let current = ocean_surface(read_surface(pixel,1),ndc,ray);
    var surface = current;
    if frame.style.z != 0u {
        let previous = ocean_surface(read_surface(pixel,0),ndc,ray);
        let blend = clamp((frame.times.x-frame.times.y)/frame.times.w,0.0,1.0);
        surface.color = mix(previous.color,current.color,blend);
        // Reverse-Z: preserve the nearest contributing surface for later passes.
        // Ignore a fully retired layer so it cannot leave invisible occlusion.
        surface.depth = max(select(0.0,previous.depth,blend < 1.0),
                            select(0.0,current.depth,blend > 0.0));
    }
    if surface.color.a == 0.0 { discard; }
    return Output(surface.color,surface.depth);
}
