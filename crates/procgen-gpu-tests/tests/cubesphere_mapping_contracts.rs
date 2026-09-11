use procgen_cubesphere::{EQUIANGULAR_TANGENT_TEST_VECTORS, MAPPING_WGSL_SOURCE};
use procgen_gpu_tests::{
    readback, request_device, run_compute, storage_output_buffer, validate_wgsl,
};
use wgpu::util::DeviceExt;

#[test]
fn tangent_wgsl_validates_without_a_device() {
    validate_wgsl("cube-sphere tangent agreement", &tangent_shader_source());
}

#[test]
fn wgsl_polynomial_tangent_matches_pinned_rust_vectors() {
    let Some((adapter_info, device, queue)) = request_device("cube-sphere tangent device") else {
        return;
    };
    println!(
        "GPU adapter: {} ({:?}, {:?})",
        adapter_info.name, adapter_info.backend, adapter_info.device_type
    );

    let actual = dispatch_tangents(&device, &queue);
    for (&(coordinate, expected_bits), &actual_bits) in
        EQUIANGULAR_TANGENT_TEST_VECTORS.iter().zip(&actual)
    {
        assert_eq!(
            actual_bits, expected_bits,
            "polynomial tangent at {coordinate}"
        );
    }
}

fn dispatch_tangents(device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u32> {
    let source = tangent_shader_source();
    let coordinates = EQUIANGULAR_TANGENT_TEST_VECTORS.map(|(coordinate, _)| coordinate);
    let input_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("cube-sphere tangent inputs"),
        contents: bytemuck::cast_slice(&coordinates),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let output_buffer =
        storage_output_buffer::<u32>(device, "cube-sphere tangent outputs", coordinates.len());
    run_compute(
        device,
        queue,
        "cube-sphere tangent agreement",
        &source,
        &[
            input_buffer.as_entire_binding(),
            output_buffer.as_entire_binding(),
        ],
        coordinates.len() as u32,
    );
    readback(device, queue, &output_buffer, coordinates.len())
}

fn tangent_shader_source() -> String {
    format!(
        r#"
{MAPPING_WGSL_SOURCE}

@group(0) @binding(0) var<storage, read> inputs: array<f32>;
@group(0) @binding(1) var<storage, read_write> outputs: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if id.x >= arrayLength(&inputs) {{ return; }}
    outputs[id.x] = bitcast<u32>(cubesphere_equiangular_tangent(inputs[id.x]));
}}
"#
    )
}
