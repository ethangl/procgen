//! Draw the viewer's local voxel pipeline into its own layer and composite it.
//! The panel's chunk counts are CPU-side; only pixels prove the layer draws.
#[path = "../../../apps/realtime-pilot/src/physical_color.rs"]
mod physical_color;
#[path = "../../../apps/realtime-pilot/src/physical_surface_layers.rs"]
mod surface_layers;
use procgen_gpu_tests::{readback, request_device_for_backends};
use procgen_realtime_pilot::VoxelMeshVertex;
use surface_layers::*;
use wgpu::util::DeviceExt;

/// Local blend width and surface bias, mirroring `physical_gpu_bridge`.
const LOCAL_BLEND_M: f32 = 16.0;
const LOCAL_SURFACE_BIAS_M: f32 = 1.0;
const ANCHOR: [i32; 4] = [1000, 0, 0, 0];
const ORIGIN: [i32; 4] = [1000, -16, -16, 0];
const SIZE: [u32; 2] = [16, 16];

#[test]
fn local_chunk_geometry_reaches_the_screen_and_the_panel_control_removes_it() {
    let backend = if cfg!(target_os = "macos") {
        wgpu::Backends::METAL
    } else {
        wgpu::Backends::VULKAN
    };
    let (adapter, device, queue) = request_device_for_backends("local voxel pass", backend)
        .expect("selected GPU backend required");
    eprintln!("{} {:?}", adapter.name, adapter.backend);
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: std::num::NonZeroU64::new(FRAME_BYTES),
            },
            count: None,
        }],
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: FRAME_BYTES,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    let compositor = SurfaceCompositor::new(&device, &layout);
    let pipeline = local_pipeline(&device, &layout);

    // One chunk-sized quad across the middle of the view, split as the mesher
    // splits: integer anchors inside the chunk plus a fractional offset.
    let quad: Vec<VoxelMeshVertex> = [[0, 0], [0, 32], [32, 0], [32, 32]]
        .into_iter()
        .map(|[y, z]| VoxelMeshVertex {
            anchor_m: [0, y, z, 0],
            offset_m: [0.0; 4],
            normal: [1.0, 0.0, 0.0, 0.0],
        })
        .collect();
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&quad),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[0u32, 1, 2, 2, 1, 3]),
        usage: wgpu::BufferUsages::INDEX,
    });
    let origins = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&ORIGIN),
        usage: wgpu::BufferUsages::VERTEX,
    });

    for (coverage, drawn) in [(1.0, true), (0.0, false)] {
        queue.write_buffer(&uniform, 0, &frame(coverage));
        let layers = compositor.create_layers(&device, SIZE);
        assert_eq!(layers.size, SIZE);
        // Three RGBA16F/Depth32F layers, 36 bytes per pixel.
        assert_eq!(layers.bytes(), u64::from(SIZE[0]) * u64::from(SIZE[1]) * 36);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: SIZE[0],
                height: SIZE[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: SIZE[0],
                    height: SIZE[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        // Empty height layers, as above the far field's own coverage.
        for layer in [SurfaceLayer::PreviousHeight, SurfaceLayer::CurrentHeight] {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(layers.color_attachment(layer))],
                depth_stencil_attachment: Some(layers.depth_attachment(layer)),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("local layer"),
                color_attachments: &[Some(layers.color_attachment(SurfaceLayer::Local))],
                depth_stencil_attachment: Some(layers.depth_attachment(SurfaceLayer::Local)),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &group, &[]);
            pass.set_pipeline(&pipeline);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.set_vertex_buffer(1, origins.slice(..));
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..6, 0, 0..1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composition"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            compositor.draw(&mut pass, &layers, &group);
        }
        let copy = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256 * u64::from(SIZE[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &copy,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(SIZE[1]),
                },
            },
            target.size(),
        );
        queue.submit([encoder.finish()]);
        let pixels = readback::<u16>(&device, &queue, &copy, 128 * SIZE[1] as usize);
        // The quad covers the middle half of the view; corners stay background.
        let painted = |x: usize, y: usize| (red(pixels[y * 128 + x * 4]) - 1.0).abs() > 0.01;
        assert_eq!(
            painted(SIZE[0] as usize / 2, SIZE[1] as usize / 2),
            drawn,
            "coverage weight {coverage}: center pixel"
        );
        assert!(!painted(0, 0), "coverage weight {coverage}: corner pixel");
    }
}

/// Decode one non-negative Rgba16Float channel, as the compositor test does.
fn red(half: u16) -> f32 {
    if half == 0 {
        return 0.0;
    }
    (1.0 + f32::from(half & 1023) / 1024.0) * 2f32.powi(i32::from(half >> 10) - 15)
}

fn local_pipeline(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(TERRAIN_SHADER.as_str().into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[layout],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[
                wgpu::VertexBufferLayout {
                    array_stride: size_of::<VoxelMeshVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0=>Sint32x4,1=>Float32x4,3=>Float32x4],
                },
                wgpu::VertexBufferLayout {
                    array_stride: 16,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![2=>Sint32x4],
                },
            ],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fragment"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        multiview: None,
        cache: None,
    })
}

/// The viewer's frame uniform: the quad fills the middle half of the view at a
/// fixed depth, and the local region extends well past the 16 m dissolve band.
fn frame(coverage: f32) -> [u8; FRAME_BYTES as usize] {
    let mut frame = [0u8; FRAME_BYTES as usize];
    // Columns: x is ignored, y and z map to NDC, w supplies a constant depth.
    let clip = [
        0f32,
        0.,
        0.,
        0., //
        1. / 32.,
        0.,
        0.,
        0., //
        0.,
        1. / 32.,
        0.,
        0., //
        0.,
        0.,
        0.5,
        1.,
    ];
    frame[..64].copy_from_slice(bytemuck::cast_slice(&clip));
    frame[64..80].copy_from_slice(bytemuck::cast_slice(&ANCHOR));
    frame[80..96].copy_from_slice(bytemuck::cast_slice(&[
        0,
        LOCAL_SURFACE_BIAS_M.to_bits(),
        0,
        0,
    ]));
    // Elapsed well past the fade, so coverage alone decides visibility.
    frame[96..112].copy_from_slice(bytemuck::cast_slice(&[1f32, 0., 0., 0.25]));
    frame[112..128].copy_from_slice(bytemuck::cast_slice(&[-80f32, -80., -80., LOCAL_BLEND_M]));
    frame[128..144].copy_from_slice(bytemuck::cast_slice(&[80f32, 80., 80., coverage]));
    frame[144..160].copy_from_slice(bytemuck::cast_slice(&[300_000f32, 12_000., 0., 0.]));
    frame
}
