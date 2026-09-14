struct Frame {
    clip: mat4x4<f32>, anchor: vec4<i32>, style: vec4<u32>,
    times: vec4<f32>,
    height_scale: vec4<f32>, // reference radius and height limit, in meters
    inverse_clip: mat4x4<f32>,
    viewport: vec4<f32>, // pixel origin and size
    eye: vec4<f32>, // relative to integer anchor
    ocean_radial: vec4<f32>, // unit radial direction and camera radius
    ocean_sphere: vec4<f32>, // radius, altitude, stable quadratic c, enabled
}
@group(0) @binding(0) var<uniform> frame: Frame;
