//! Direct indexed-indirect draws of leased compute outputs into the 3D pass.
use crate::physical_gpu::GpuWorker;
use crate::physical_gpu_bridge::{
    DrawFrame, GpuBridge, HeightFrame, LOCAL_BLEND_M, LOCAL_SURFACE_BIAS_M, SURFACE_BLEND_SECONDS,
};
use crate::physical_surface_layers::{
    COLOR_FORMAT, DEPTH_FORMAT, FRAME_BYTES, SurfaceCompositor, SurfaceLayer, SurfaceLayers,
    TERRAIN_SHADER,
};
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
use wgpu::util::DeviceExt;

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
    height_pipeline: wgpu::RenderPipeline,
    compositor: SurfaceCompositor,
    height_indices: wgpu::Buffer,
    height_index_count: u32,
    height: Option<Arc<HeightFrame>>,
    previous_height: Option<Arc<HeightFrame>>,
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
                min_binding_size: NonZeroU64::new(FRAME_BYTES),
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
        source: wgpu::ShaderSource::Wgsl(TERRAIN_SHADER.into()),
    });
    const TERRAIN_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0=>Sint32x4,1=>Float32x4,3=>Float32x4];
    let make_pipeline = |vertex_entry, fragment_entry, local| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("voxel direct render"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(vertex_entry),
                compilation_options: Default::default(),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: if local {
                            size_of::<procgen_realtime_pilot::VoxelMeshVertex>() as u64
                        } else {
                            size_of::<procgen_realtime_pilot::HeightVertex>() as u64
                        },
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &TERRAIN_ATTRIBUTES,
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: 16,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![2=>Sint32x4],
                    },
                ][..if local { 2 } else { 1 }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(fragment_entry),
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
    };
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("voxel camera"),
        size: FRAME_BYTES,
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
    let indices = procgen_realtime_pilot::height_indices();
    let height_indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("height grid indices"),
        contents: bytemuck::cast_slice(&indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    commands.insert_resource(GpuDraw {
        height_pipeline: make_pipeline("height_vertex", "height_fragment", false),
        compositor: SurfaceCompositor::new(device, &layout),
        height_indices,
        height_index_count: indices.len() as u32,
        height: None,
        previous_height: None,
        frame: None,
        group,
        uniform,
        pipeline: make_pipeline("vertex", "fragment", true),
    });
    commands.insert_resource(GpuWorker::start(
        device.clone(),
        wgpu::Queue::clone(&queue),
        bridge.clone(),
    ));
}
#[derive(Component)]
struct ViewSurfaces(SurfaceLayers);
fn prepare(
    mut commands: Commands,
    bridge: Res<GpuBridge>,
    mut draw: ResMut<GpuDraw>,
    queue: Res<RenderQueue>,
    device: Res<RenderDevice>,
    views: Query<(Entity, &ExtractedCamera, Option<&ViewSurfaces>), With<GpuView>>,
) {
    let start = Instant::now();
    for (entity, camera, surfaces) in &views {
        let Some(size) = camera.physical_target_size else {
            continue;
        };
        if size.x == 0 || size.y == 0 {
            continue;
        }
        if surfaces.is_none_or(|s| s.0.size != size.to_array()) {
            commands.entity(entity).insert(ViewSurfaces(
                draw.compositor
                    .create_layers(device.wgpu_device(), size.to_array()),
            ));
        }
    }
    for submission in bridge.submissions.lock().unwrap().try_iter().take(
        procgen_realtime_pilot::LOCAL_GPU_WORLD_CONFIG
            .stream
            .max_in_flight
            + 1,
    ) {
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
    draw.height = output.height.clone();
    draw.previous_height = output.previous_height.clone();
    output.stats.scheduler_ms = start.elapsed().as_secs_f64() * 1000.0;
}
fn retain_submitted(draw: Res<GpuDraw>, queue: Res<RenderQueue>) {
    let heights = (draw.height.clone(), draw.previous_height.clone());
    queue.on_submitted_work_done(move || drop(heights));
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
        &'static ViewSurfaces,
    );
    fn run<'w>(
        &self,
        _graph: &mut RenderGraphContext,
        context: &mut RenderContext<'w>,
        (settings, frustum, view, camera, target, depth, surfaces): QueryItem<
            'w,
            '_,
            Self::ViewQuery,
        >,
        world: &'w World,
    ) -> Result<(), NodeRunError> {
        let draw = world.resource::<GpuDraw>();
        let start = Instant::now();
        let matrix = view.clip_from_view * view.world_from_view.to_matrix().inverse();
        let mut uniform = [0u8; FRAME_BYTES as usize];
        uniform[..64].copy_from_slice(bytemuck::cast_slice(&matrix.to_cols_array()));
        uniform[64..80].copy_from_slice(bytemuck::cast_slice(&settings.anchor));
        uniform[80..96].copy_from_slice(bytemuck::cast_slice(&[
            settings.coloring,
            LOCAL_SURFACE_BIAS_M.to_bits(),
            u32::from(draw.previous_height.is_some()),
            0,
        ]));
        let now = world.resource::<GpuBridge>().start.elapsed().as_secs_f32();
        uniform[96..112].copy_from_slice(bytemuck::cast_slice(&[
            now,
            draw.height.as_ref().map_or(0.0, |h| h.born),
            draw.frame.as_ref().map_or(0.0, |f| f.born),
            SURFACE_BLEND_SECONDS,
        ]));
        if let Some(frame) = &draw.frame {
            for end in 0..2 {
                let mut bound = [0f32; 4];
                for (axis, value) in bound[..3].iter_mut().enumerate() {
                    *value = (frame.bounds[end][axis] - settings.anchor[axis]) as f32;
                }
                bound[3] = if end == 0 {
                    LOCAL_BLEND_M
                } else {
                    f32::from(!frame.leases.is_empty())
                };
                uniform[112 + end * 16..128 + end * 16]
                    .copy_from_slice(bytemuck::cast_slice(&bound));
            }
        } else {
            uniform[124..128].copy_from_slice(&LOCAL_BLEND_M.to_le_bytes());
        }
        world
            .resource::<RenderQueue>()
            .write_buffer(&draw.uniform, 0, &uniform);
        let viewport = |pass: &mut wgpu::RenderPass<'_>| {
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
        };
        for (height, layer) in [
            (&draw.previous_height, SurfaceLayer::PreviousHeight),
            (&draw.height, SurfaceLayer::CurrentHeight),
        ] {
            // Clear absent layers too: a retired snapshot must not leave stale pixels.
            let attachment = [Some(surfaces.0.color_attachment(layer))];
            let mut pass =
                context
                    .command_encoder()
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("opaque height layer"),
                        color_attachments: &attachment,
                        depth_stencil_attachment: Some(surfaces.0.depth_attachment(layer)),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
            viewport(&mut pass);
            pass.set_bind_group(0, &draw.group, &[]);
            pass.set_index_buffer(draw.height_indices.slice(..), wgpu::IndexFormat::Uint32);
            if let Some(height) = height {
                pass.set_pipeline(&draw.height_pipeline);
                for (tile, buffer) in &height.tiles {
                    let field = &world.resource::<GpuBridge>().field;
                    let radius = field.config().radius_m;
                    let d = tile
                        .grid_vertex(
                            procgen_cubesphere::TILE_QUADS / 2,
                            procgen_cubesphere::TILE_QUADS / 2,
                        )
                        .unwrap()
                        .direction();
                    let center = Vec3::new(d.x, d.y, d.z) * radius
                        - Vec3::new(
                            settings.anchor[0] as f32,
                            settings.anchor[1] as f32,
                            settings.anchor[2] as f32,
                        );
                    let width = procgen_cubesphere::vertex_spacing(tile.level())
                        * radius
                        * procgen_cubesphere::TILE_QUADS as f32;
                    let bounds = Aabb {
                        center: center.into(),
                        half_extents: Vec3::splat(width + field.config().height_limit_m * 2.0)
                            .into(),
                    };
                    if !frustum.intersects_obb_identity(&bounds) {
                        continue;
                    }

                    pass.set_vertex_buffer(0, buffer.slice(..));
                    pass.draw_indexed(0..draw.height_index_count, 0, 0..1);
                }
            }
        }
        let attachment = [Some(surfaces.0.color_attachment(SurfaceLayer::Local))];
        let mut pass = context
            .command_encoder()
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("opaque local layer"),
                color_attachments: &attachment,
                depth_stencil_attachment: Some(surfaces.0.depth_attachment(SurfaceLayer::Local)),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        viewport(&mut pass);
        pass.set_bind_group(0, &draw.group, &[]);
        pass.set_pipeline(&draw.pipeline);
        let mut drawn = 0;
        if let Some(frame) = &draw.frame {
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
        }
        drop(pass);
        let attachment = [Some(target.get_color_attachment())];
        let mut pass = context
            .command_encoder()
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("smooth surface composition"),
                color_attachments: &attachment,
                depth_stencil_attachment: Some(depth.get_attachment(wgpu::StoreOp::Store)),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        viewport(&mut pass);
        draw.compositor.draw(&mut pass, &surfaces.0, &draw.group);
        drop(pass);
        if let Ok(mut output) = world.resource::<GpuBridge>().output.try_lock() {
            output.stats.surface_target_bytes = surfaces.0.bytes();
            output.stats.drawn = drawn;
            output.stats.draw_ms = start.elapsed().as_secs_f64() * 1000.0;
        }
        Ok(())
    }
}
