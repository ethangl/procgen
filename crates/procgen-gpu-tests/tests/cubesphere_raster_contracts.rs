use bytemuck::{Pod, Zeroable};
use procgen_cubesphere::{
    EQUIANGULAR_TANGENT_TEST_VECTORS, FaceTexel, MAPPING_WGSL_SOURCE, NO_RASTER_CELL,
    RASTER_WGSL_SOURCE, TexelLink,
};
use procgen_gpu_tests::{
    readback, request_device, run_compute, storage_output_buffer, validate_wgsl,
};
use wgpu::util::DeviceExt;

const RESOLUTION: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LinkParameters {
    resolution: u32,
    output_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct LinkOutput {
    cell_id: u32,
    length: u32,
}

#[test]
fn raster_link_wgsl_validates_without_a_device() {
    validate_wgsl("cube-sphere raster-link agreement", &link_shader_source());
}

#[test]
fn tangent_wgsl_validates_without_a_device() {
    validate_wgsl("cube-sphere tangent agreement", &tangent_shader_source());
}

#[test]
fn wgsl_raster_links_match_rust_across_every_seam_and_corner() {
    let Some((adapter_info, device, queue)) = request_device("cube-sphere raster-link device")
    else {
        return;
    };
    println!(
        "GPU adapter: {} ({:?}, {:?})",
        adapter_info.name, adapter_info.backend, adapter_info.device_type
    );

    let cell_count = FaceTexel::cell_count(RESOLUTION).unwrap();
    let output_count = cell_count * TexelLink::ALL.len() as u32;
    let outputs = dispatch_links(&device, &queue, output_count);

    for cell_id in 0..cell_count {
        let texel = FaceTexel::from_cell_id(cell_id, RESOLUTION).unwrap();
        for link in TexelLink::ALL {
            let output = outputs[(cell_id * 8 + link.index()) as usize];
            let expected = texel
                .neighbor(link)
                .map_or(NO_RASTER_CELL, FaceTexel::cell_id);
            assert_eq!(output.cell_id, expected);
            assert_eq!(output.length, link.link_length());
        }
    }
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

fn dispatch_links(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output_count: u32,
) -> Vec<LinkOutput> {
    let source = link_shader_source();
    let parameters = LinkParameters {
        resolution: RESOLUTION,
        output_count,
    };
    let parameter_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("cube-sphere raster-link parameters"),
        contents: bytemuck::bytes_of(&parameters),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let output_buffer = storage_output_buffer::<LinkOutput>(
        device,
        "cube-sphere raster-link outputs",
        output_count as usize,
    );
    run_compute(
        device,
        queue,
        "cube-sphere raster-link agreement",
        &source,
        &[
            parameter_buffer.as_entire_binding(),
            output_buffer.as_entire_binding(),
        ],
        output_count,
    );
    readback(device, queue, &output_buffer, output_count as usize)
}

fn link_shader_source() -> String {
    format!(
        r#"
{MAPPING_WGSL_SOURCE}
{RASTER_WGSL_SOURCE}

struct Parameters {{
    resolution: u32,
    output_count: u32,
}}
struct Output {{
    cell_id: u32,
    length: u32,
}}
@group(0) @binding(0) var<uniform> parameters: Parameters;
@group(0) @binding(1) var<storage, read_write> outputs: array<Output>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if id.x >= parameters.output_count {{ return; }}
    let link = id.x % 8u;
    outputs[id.x] = Output(
        cubesphere_texel_neighbor(id.x / 8u, parameters.resolution, link),
        cubesphere_texel_link_length(link),
    );
}}
"#
    )
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
