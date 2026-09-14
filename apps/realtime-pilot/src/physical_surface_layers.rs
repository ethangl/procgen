//! Viewer-owned opaque surface layers and their smooth final composition.
//! Each surface resolves its own visibility before any coverage is blended.
pub static TERRAIN_SHADER: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "{}\n{}\n{}",
        crate::physical_color::height_color_shader(),
        include_str!("physical_frame.wgsl"),
        include_str!("physical_gpu.wgsl")
    )
});
pub const COMPOSITE_SHADER: &str = concat!(
    include_str!("physical_frame.wgsl"),
    include_str!("physical_composite.wgsl")
);
pub const FRAME_BYTES: u64 = 160;
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[derive(Clone, Copy)]
pub enum SurfaceLayer {
    PreviousHeight = 0,
    CurrentHeight = 1,
    Local = 2,
}
pub struct SurfaceLayers {
    pub size: [u32; 2],
    colors: [wgpu::TextureView; 3],
    depths: [wgpu::TextureView; 3],
    group: wgpu::BindGroup,
}
impl SurfaceLayers {
    /// Texture payload only; excludes driver padding and other viewer targets.
    pub fn bytes(&self) -> u64 {
        u64::from(self.size[0])
            * u64::from(self.size[1])
            * self.colors.len() as u64
            * u64::from(
                COLOR_FORMAT.block_copy_size(None).unwrap()
                    + DEPTH_FORMAT.block_copy_size(None).unwrap(),
            )
    }
    pub fn color_attachment(&self, layer: SurfaceLayer) -> wgpu::RenderPassColorAttachment<'_> {
        wgpu::RenderPassColorAttachment {
            view: &self.colors[layer as usize],
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        }
    }
    pub fn depth_attachment(
        &self,
        layer: SurfaceLayer,
    ) -> wgpu::RenderPassDepthStencilAttachment<'_> {
        wgpu::RenderPassDepthStencilAttachment {
            view: &self.depths[layer as usize],
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(0.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }
    }
}
pub struct SurfaceCompositor {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}
impl SurfaceCompositor {
    pub fn new(device: &wgpu::Device, frame_layout: &wgpu::BindGroupLayout) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surface layer samples"),
            entries: &[
                (0, wgpu::TextureSampleType::Float { filterable: false }),
                (1, wgpu::TextureSampleType::Depth),
            ]
            .map(|(binding, sample_type)| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            }),
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("surface compositor"),
            source: wgpu::ShaderSource::Wgsl(COMPOSITE_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("surface compositor"),
            bind_group_layouts: &[frame_layout, &layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("surface compositor"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
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
        });
        Self { layout, pipeline }
    }
    pub fn create_layers(&self, device: &wgpu::Device, size: [u32; 2]) -> SurfaceLayers {
        let make_texture = |format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("terrain surface layers"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 3,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let colors = make_texture(COLOR_FORMAT);
        let depths = make_texture(DEPTH_FORMAT);
        let color_array = colors.create_view(&Default::default());
        let depth_array = depths.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("surface layer samples"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_array),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&depth_array),
                },
            ],
        });
        let views = |texture: &wgpu::Texture| {
            std::array::from_fn(|i| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: i as u32,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
        };
        SurfaceLayers {
            size,
            colors: views(&colors),
            depths: views(&depths),
            group,
        }
    }
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        layers: &SurfaceLayers,
        frame: &wgpu::BindGroup,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, frame, &[]);
        pass.set_bind_group(1, &layers.group, &[]);
        pass.draw(0..3, 0..1);
    }
}
