//! Direct indexed-indirect draws of leased compute outputs into the 3D pass.
use crate::physical_gpu::{DrawFrame, GpuBridge, GpuWorker};
use bevy::{
    camera::primitives::{Aabb, Frustum},
    core_pipeline::core_3d::graph::{Core3d, Node3d},
    ecs::query::QueryItem,
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        camera::ExtractedCamera,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        render_graph::{
            NodeRunError, RenderGraphContext, RenderGraphExt, RenderLabel, ViewNode, ViewNodeRunner,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        view::{ExtractedView, ViewDepthTexture, ViewTarget},
    },
};
use std::{num::NonZeroU64, sync::Arc, time::Instant};

#[derive(Component, Clone, ExtractComponent)]
pub struct GpuView {
    pub anchor: [i32; 4],
    pub coloring: u32,
}
pub struct GpuTerrainPlugin(pub GpuBridge);
impl Plugin for GpuTerrainPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.0.clone())
            .add_plugins(ExtractComponentPlugin::<GpuView>::default());
        let render = app.sub_app_mut(RenderApp);
        render
            .insert_resource(self.0.clone())
            .add_systems(RenderStartup, initialize)
            .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources))
            .add_systems(Render, retain_submitted.after(RenderSystems::Render))
            .add_render_graph_node::<ViewNodeRunner<GpuTerrainNode>>(Core3d, GpuTerrainLabel)
            .add_render_graph_edges(
                Core3d,
                (
                    Node3d::MainOpaquePass,
                    GpuTerrainLabel,
                    Node3d::MainTransparentPass,
                ),
            );
    }
}
#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct GpuTerrainLabel;
#[derive(Resource)]
struct GpuDraw {
    frame: Option<Arc<DrawFrame>>,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    pipeline: wgpu::RenderPipeline,
}
fn initialize(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    bridge: Res<GpuBridge>,
) {
    let device = device.wgpu_device();
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("voxel draw layout"),
        entries: &[0].map(|binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(96),
            },
            count: None,
        }),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("voxel render layout"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("voxel direct render"),
        source: wgpu::ShaderSource::Wgsl(include_str!("physical_gpu.wgsl").into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("voxel direct render"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[
                wgpu::VertexBufferLayout {
                    array_stride: 32,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0=>Sint32x4,1=>Float32x4],
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
                format: wgpu::TextureFormat::Rgba16Float,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        multiview: None,
        cache: None,
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("voxel camera"),
        size: 96,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("voxel camera binding"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    commands.insert_resource(GpuDraw {
        frame: None,
        group,
        uniform,
        pipeline,
    });
    commands.insert_resource(GpuWorker::start(
        device.clone(),
        wgpu::Queue::clone(&queue),
        bridge.clone(),
    ));
}
fn prepare(bridge: Res<GpuBridge>, mut draw: ResMut<GpuDraw>, queue: Res<RenderQueue>) {
    let start = Instant::now();
    for submission in bridge.submissions.lock().unwrap().try_iter() {
        submission.submit(&queue);
    }
    let Ok(mut output) = bridge.output.try_lock() else {
        return;
    };
    if let Some(frame) = &output.frame
        && draw
            .frame
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, frame))
    {
        draw.frame = Some(Arc::clone(frame));
    }
    output.stats.scheduler_ms = start.elapsed().as_secs_f64() * 1000.0;
}
fn retain_submitted(draw: Res<GpuDraw>, queue: Res<RenderQueue>) {
    if let Some(frame) = &draw.frame {
        let lease = Arc::clone(frame);
        queue.on_submitted_work_done(move || drop(lease));
    }
}
#[derive(Default)]
struct GpuTerrainNode;
impl ViewNode for GpuTerrainNode {
    type ViewQuery = (
        &'static GpuView,
        &'static Frustum,
        &'static ExtractedView,
        &'static ExtractedCamera,
        &'static ViewTarget,
        &'static ViewDepthTexture,
    );
    fn run<'w>(
        &self,
        _graph: &mut RenderGraphContext,
        context: &mut RenderContext<'w>,
        (settings, frustum, view, camera, target, depth): QueryItem<'w, '_, Self::ViewQuery>,
        world: &'w World,
    ) -> Result<(), NodeRunError> {
        let draw = world.resource::<GpuDraw>();
        let Some(frame) = &draw.frame else {
            return Ok(());
        };
        let start = Instant::now();
        let matrix = view.clip_from_view * view.world_from_view.to_matrix().inverse();
        let mut uniform = [0u8; 96];
        uniform[..64].copy_from_slice(bytemuck::cast_slice(&matrix.to_cols_array()));
        uniform[64..80].copy_from_slice(bytemuck::cast_slice(&settings.anchor));
        uniform[80..96].copy_from_slice(bytemuck::cast_slice(&[settings.coloring, 0, 0, 0]));
        world
            .resource::<RenderQueue>()
            .write_buffer(&draw.uniform, 0, &uniform);
        let attachment = [Some(target.get_color_attachment())];
        let mut pass = context
            .command_encoder()
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("voxel indirect draw"),
                color_attachments: &attachment,
                depth_stencil_attachment: Some(depth.get_attachment(wgpu::StoreOp::Store)),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        if let Some(v) = &camera.viewport {
            pass.set_viewport(
                v.physical_position.x as f32,
                v.physical_position.y as f32,
                v.physical_size.x as f32,
                v.physical_size.y as f32,
                v.depth.start,
                v.depth.end,
            );
        }
        pass.set_pipeline(&draw.pipeline);
        pass.set_bind_group(0, &draw.group, &[]);
        let mut drawn = 0;
        for (i, lease) in frame.leases.iter().enumerate() {
            let a = lease.key.address();
            let p = a.origin();
            let lo = Vec3::new(
                (p.x_m - settings.anchor[0]) as f32,
                (p.y_m - settings.anchor[1]) as f32,
                (p.z_m - settings.anchor[2]) as f32,
            );
            let half = Vec3::splat(a.span_m() as f32 * 0.5);
            let bounds = Aabb {
                center: (lo + half).into(),
                half_extents: half.into(),
            };
            if !frustum.intersects_obb_identity(&bounds) {
                continue;
            }
            drawn += 1;
            pass.set_vertex_buffer(1, frame.origins.slice(i as u64 * 16..(i as u64 + 1) * 16));
            for mesh in [&lease.slot.regular, &lease.slot.transition] {
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed_indirect(&mesh.draw, 0);
            }
        }
        drop(pass);
        if let Ok(mut output) = world.resource::<GpuBridge>().output.try_lock() {
            output.stats.drawn = drawn;
            output.stats.draw_ms = start.elapsed().as_secs_f64() * 1000.0;
        }
        Ok(())
    }
}
