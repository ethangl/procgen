use bytemuck::{Pod, Zeroable};
use procgen_cubesphere::{
    EQUIANGULAR_TANGENT_TEST_VECTORS, FaceTexel, MAPPING_WGSL_SOURCE, NO_RASTER_CELL,
    RASTER_WGSL_SOURCE, TEXEL_SOLID_ANGLE_TOLERANCE, TexelLink,
};
use procgen_gpu_tests::{
    readback, request_device, run_compute, storage_output_buffer, validate_wgsl,
};
use wgpu::util::DeviceExt;

/// Face resolutions the raster contract is checked at: one small enough to
/// enumerate every seam and cube corner, one large enough to sample the area
/// element where the pilot runs.
const RESOLUTIONS: [u32; 2] = [8, 128];

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
    /// Canonical border-edge id, or [`NO_RASTER_CELL`] for a diagonal link.
    border_edge: u32,
    /// Bits of the source cell's solid angle, repeated for every link so the
    /// contract needs one dispatch.
    solid_angle: u32,
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
fn wgsl_raster_adjacency_and_area_match_rust_across_every_seam_and_corner() {
    let Some((adapter_info, device, queue)) = request_device("cube-sphere raster-link device")
    else {
        return;
    };
    println!(
        "GPU adapter: {} ({:?}, {:?})",
        adapter_info.name, adapter_info.backend, adapter_info.device_type
    );

    let mut divergence = 0.0_f32;
    for resolution in RESOLUTIONS {
        let cell_count = FaceTexel::cell_count(resolution).unwrap();
        let links = TexelLink::ALL.len() as u32;
        let outputs = dispatch_links(&device, &queue, resolution, cell_count * links);

        for cell_id in 0..cell_count {
            let texel = FaceTexel::from_cell_id(cell_id, resolution).unwrap();
            for link in TexelLink::ALL {
                let output = outputs[(cell_id * links + link.index()) as usize];
                assert_eq!(
                    output.cell_id,
                    texel
                        .neighbor(link)
                        .map_or(NO_RASTER_CELL, FaceTexel::cell_id)
                );
                assert_eq!(output.length, link.link_length());
                assert_eq!(
                    output.border_edge,
                    if link.is_border() {
                        texel.border_edge(link)
                    } else {
                        NO_RASTER_CELL
                    }
                );
                let area = texel.solid_angle();
                divergence =
                    divergence.max((f32::from_bits(output.solid_angle) - area).abs() / area);
            }
        }
    }
    // The area element divides and takes a square root, so it is the one raster
    // quantity the mirrors agree on within a tolerance rather than exactly.
    assert!(
        divergence <= TEXEL_SOLID_ANGLE_TOLERANCE,
        "solid angles diverged by {divergence:e}, past {TEXEL_SOLID_ANGLE_TOLERANCE:e}"
    );
    println!("maximum relative solid-angle divergence: {divergence:e}");
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
    resolution: u32,
    output_count: u32,
) -> Vec<LinkOutput> {
    let source = link_shader_source();
    let parameters = LinkParameters {
        resolution,
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
    border_edge: u32,
    solid_angle: u32,
}}
@group(0) @binding(0) var<uniform> parameters: Parameters;
@group(0) @binding(1) var<storage, read_write> outputs: array<Output>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if id.x >= parameters.output_count {{ return; }}
    let link = id.x % 8u;
    let cell_id = id.x / 8u;
    var border_edge = CUBESPHERE_NO_RASTER_CELL;
    if link < CUBESPHERE_BORDER_LINKS_PER_CELL {{
        border_edge = cubesphere_border_edge(cell_id, parameters.resolution, link);
    }}
    outputs[id.x] = Output(
        cubesphere_texel_neighbor(cell_id, parameters.resolution, link),
        cubesphere_texel_link_length(link),
        border_edge,
        bitcast<u32>(cubesphere_texel_solid_angle(cell_id, parameters.resolution)),
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
