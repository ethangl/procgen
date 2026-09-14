//! Render the production compositor into a small target with known opaque layers.
#[path = "../../../apps/realtime-pilot/src/physical_color.rs"]
mod physical_color;
#[path = "../../../apps/realtime-pilot/src/physical_surface_layers.rs"]
mod surface_layers;
use procgen_gpu_tests::{readback, request_device_for_backends, validate_wgsl};
use surface_layers::*;

#[derive(Clone, Copy)]
struct Sample {
    color: wgpu::Color,
    depth: f32,
}
const EMPTY: Sample = Sample {
    color: wgpu::Color::TRANSPARENT,
    depth: 0.0,
};
const RED: Sample = Sample {
    color: wgpu::Color::RED,
    depth: 0.5,
};
const BLUE: Sample = Sample {
    color: wgpu::Color::BLUE,
    depth: 0.75,
};
const LOCAL: Sample = Sample {
    color: wgpu::Color {
        r: 0.0,
        g: 1.0,
        b: 0.0,
        a: 0.5,
    },
    depth: 0.625,
};
struct Case {
    name: &'static str,
    previous: Sample,
    current: Sample,
    local: Sample,
    blend: f32,
    replacing: bool,
    expected_rgb: [f32; 3],
    expected_depth: f32,
}

#[test]
fn opaque_layers_blend_without_stipple_or_hidden_surface_leaks() {
    validate_wgsl("terrain layer", &TERRAIN_SHADER);
    validate_wgsl("surface compositor", COMPOSITE_SHADER);
    let backend = if cfg!(target_os = "macos") {
        wgpu::Backends::METAL
    } else {
        wgpu::Backends::VULKAN
    };
    let (adapter, device, queue) = request_device_for_backends("surface composition", backend)
        .expect("selected GPU backend required");
    eprintln!("{} {:?}", adapter.name, adapter.backend);
    check_height_colors(&device, &queue);
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
    let cases = [
        Case {
            name: "first snapshot is immediately opaque",
            previous: EMPTY,
            current: RED,
            local: EMPTY,
            blend: 0.0,
            replacing: false,
            expected_rgb: [1.0, 0.0, 0.0],
            expected_depth: 0.5,
        },
        Case {
            name: "height crossfade works when new surface is behind old",
            previous: BLUE,
            current: RED,
            local: EMPTY,
            blend: 0.5,
            replacing: true,
            expected_rgb: [0.5, 0.0, 0.5],
            expected_depth: 0.75,
        },
        Case {
            name: "local overlap is smooth",
            previous: EMPTY,
            current: RED,
            local: LOCAL,
            blend: 1.0,
            replacing: false,
            expected_rgb: [0.5, 0.5, 0.0],
            expected_depth: 0.625,
        },
        Case {
            name: "height occludes hidden local surface",
            previous: EMPTY,
            current: BLUE,
            local: LOCAL,
            blend: 1.0,
            replacing: false,
            expected_rgb: [0.0, 0.0, 1.0],
            expected_depth: 0.75,
        },
        Case {
            name: "coverage rounded to zero cannot write invisible depth",
            previous: EMPTY,
            current: RED,
            local: Sample {
                color: wgpu::Color::TRANSPARENT,
                depth: 0.75,
            },
            blend: 1.0,
            replacing: false,
            expected_rgb: [1.0, 0.0, 0.0],
            expected_depth: 0.5,
        },
        Case {
            name: "local visibility blends separately through height replacement",
            previous: BLUE,
            current: RED,
            local: LOCAL,
            blend: 0.5,
            replacing: true,
            expected_rgb: [0.25, 0.25, 0.5],
            expected_depth: 0.75,
        },
        Case {
            name: "retired height cannot retain invisible depth",
            previous: BLUE,
            current: RED,
            local: EMPTY,
            blend: 1.0,
            replacing: true,
            expected_rgb: [1.0, 0.0, 0.0],
            expected_depth: 0.5,
        },
        Case {
            name: "unborn height cannot write depth",
            previous: RED,
            current: BLUE,
            local: EMPTY,
            blend: 0.0,
            replacing: true,
            expected_rgb: [1.0, 0.0, 0.0],
            expected_depth: 0.5,
        },
        Case {
            name: "silhouette fades over background",
            previous: BLUE,
            current: EMPTY,
            local: EMPTY,
            blend: 0.5,
            replacing: true,
            expected_rgb: [0.5, 0.5, 1.0],
            expected_depth: 0.75,
        },
        Case {
            name: "missing height does not multiply local coverage",
            previous: EMPTY,
            current: EMPTY,
            local: LOCAL,
            blend: 0.5,
            replacing: true,
            expected_rgb: [0.5, 1.0, 0.5],
            expected_depth: 0.625,
        },
        Case {
            name: "empty layers preserve scene",
            previous: EMPTY,
            current: EMPTY,
            local: EMPTY,
            blend: 0.5,
            replacing: false,
            expected_rgb: [1.0, 1.0, 1.0],
            expected_depth: 0.0,
        },
    ];
    // Recreate at a second size, as the viewer does on window resize.
    for size in [[4, 4], [7, 3]] {
        let layers = compositor.create_layers(&device, size);
        assert_eq!(layers.size, size);
        eprintln!("Surface target payload: {} bytes", layers.bytes());
        let texture = |format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let color = texture(COLOR_FORMAT);
        let depth = texture(DEPTH_FORMAT);
        let color_view = color.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        for case in &cases {
            let mut frame = [0u8; FRAME_BYTES as usize];
            frame[88..92].copy_from_slice(&u32::from(case.replacing).to_le_bytes());
            frame[96..100].copy_from_slice(&case.blend.to_le_bytes());
            frame[108..112].copy_from_slice(&1f32.to_le_bytes());
            queue.write_buffer(&uniform, 0, &frame);
            let mut encoder = device.create_command_encoder(&Default::default());
            for (layer, sample) in [
                (SurfaceLayer::PreviousHeight, case.previous),
                (SurfaceLayer::CurrentHeight, case.current),
                (SurfaceLayer::Local, case.local),
            ] {
                let mut color = layers.color_attachment(layer);
                color.ops.load = wgpu::LoadOp::Clear(sample.color);
                let mut depth = layers.depth_attachment(layer);
                depth.depth_ops.as_mut().unwrap().load = wgpu::LoadOp::Clear(sample.depth);
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(color)],
                    depth_stencil_attachment: Some(depth),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(case.name),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
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
            let copy = |encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture, aspect| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: 256 * size[1] as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(256),
                            rows_per_image: Some(size[1]),
                        },
                    },
                    texture.size(),
                );
                buffer
            };
            let color_copy = copy(&mut encoder, &color, wgpu::TextureAspect::All);
            let depth_copy = copy(&mut encoder, &depth, wgpu::TextureAspect::DepthOnly);
            queue.submit([encoder.finish()]);
            let rgb = readback::<u16>(&device, &queue, &color_copy, 128 * size[1] as usize);
            let z = readback::<f32>(&device, &queue, &depth_copy, 64 * size[1] as usize);
            for y in 0..size[1] as usize {
                for x in 0..size[0] as usize {
                    for c in 0..3 {
                        let half = rgb[y * 128 + x * 4 + c];
                        let value = if half == 0 {
                            0.0
                        } else {
                            (1.0 + f32::from(half & 1023) / 1024.0)
                                * 2f32.powi(i32::from(half >> 10) - 15)
                        };
                        // Half-float render target precision, independent of adapter.
                        assert!(
                            (value - case.expected_rgb[c]).abs() < 0.001,
                            "{} pixel {x},{y}: {value}",
                            case.name
                        );
                    }
                    assert!(
                        (z[y * 64 + x] - case.expected_depth).abs() < 0.00001,
                        "{} depth",
                        case.name
                    );
                }
            }
        }
    }
}

fn check_height_colors(device: &wgpu::Device, queue: &wgpu::Queue) {
    use procgen_gpu_tests::{run_compute, storage_output_buffer};
    use wgpu::util::DeviceExt;
    // Cover every ramp segment and clamping outside the configured range.
    let heights: Vec<f32> = (-256..=256).map(|i| i as f32 / 128.0).collect();
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("height color inputs"),
        contents: bytemuck::cast_slice(&heights),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let output = storage_output_buffer::<[f32; 4]>(device, "height colors", heights.len());
    let shader = format!(
        "{}\n{}",
        physical_color::height_color_shader(),
        r#"
        @group(0) @binding(0) var<storage,read> heights: array<f32>;
        @group(0) @binding(1) var<storage,read_write> colors: array<vec4<f32>>;
        @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
            if id.x < arrayLength(&heights) { colors[id.x] = vec4(height_color(heights[id.x]),1.0); }
        }
    "#
    );
    run_compute(
        device,
        queue,
        "height colors",
        &shader,
        &[input.as_entire_binding(), output.as_entire_binding()],
        heights.len() as u32,
    );
    let colors = readback::<[f32; 4]>(device, queue, &output, heights.len());
    for (height, gpu) in heights.iter().zip(colors) {
        let cpu = physical_color::height_color(*height);
        for axis in 0..4 {
            // Several f32 interpolation operations, well below visible color precision.
            assert!((cpu[axis] - gpu[axis]).abs() < 0.000001);
        }
    }
}
