struct Frame {
    clip: mat4x4<f32>, anchor: vec4<i32>, style: vec4<u32>,
    times: vec4<f32>, local_lo: vec4<f32>, local_hi: vec4<f32>,
}
@group(0) @binding(0) var<uniform> frame: Frame;
