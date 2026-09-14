//! Draw immutable GPU height snapshots and composite terrain with oceans.
use crate::physical_gpu::GpuWorker;
use crate::physical_gpu_bridge::{GpuBridge, HeightFrame, SURFACE_BLEND_SECONDS};
use crate::physical_surface_layers::{
    COLOR_FORMAT, DEPTH_FORMAT, FRAME_BYTES, SurfaceCompositor, SurfaceLayer, SurfaceLayers,
    TERRAIN_SHADER,
};
use bevy::{
    camera::primitives::Frustum,
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
    pub eye: procgen_realtime_pilot::MeterPosition,
    pub ocean: crate::ocean::OceanConfig,
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
    field: Arc<procgen_realtime_pilot::PlanetDesignField>,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    height_pipeline: wgpu::RenderPipeline,
    compositor: SurfaceCompositor,
    height_indices: wgpu::Buffer,
    height_index_count: u32,
    height: Option<Arc<HeightFrame>>,
    previous_height: Option<Arc<HeightFrame>>,
}
fn initialize(mut commands: Commands, device: Res<RenderDevice>, bridge: Res<GpuBridge>) {
    let device = device.wgpu_device();
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("height draw layout"),
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
        label: Some("height render layout"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("height render"),
        source: wgpu::ShaderSource::Wgsl(TERRAIN_SHADER.as_str().into()),
    });
    const TERRAIN_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0=>Sint32x4,1=>Float32x4,3=>Float32x4];
    let make_pipeline = || {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("height render"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("height_vertex"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: size_of::<procgen_realtime_pilot::HeightVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &TERRAIN_ATTRIBUTES,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("height_fragment"),
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
        label: Some("height camera"),
        size: FRAME_BYTES,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("height camera binding"),
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
        field: Arc::clone(&bridge.display.lock().unwrap().field),
        height_pipeline: make_pipeline(),
        compositor: SurfaceCompositor::new(device, &layout),
        height_indices,
        height_index_count: indices.len() as u32,
        height: None,
        previous_height: None,
        group,
        uniform,
    });
    commands.insert_resource(GpuWorker::start(device.clone(), bridge.clone()));
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
    for submission in bridge
        .submissions
        .lock()
        .unwrap()
        .try_iter()
        .take(crate::physical_height::HEIGHT_BATCHES_IN_FLIGHT)
    {
        submission.submit(&queue);
    }
    let displayed = bridge.display.lock().unwrap().clone();
    let Ok(mut output) = displayed.output.try_lock() else {
        return;
    };
    draw.field = displayed.field;
    draw.height = output.height.clone();
    draw.previous_height = output.previous_height.clone();
    output.stats.scheduler_ms = start.elapsed().as_secs_f64() * 1000.0;
}
fn retain_submitted(draw: Res<GpuDraw>, queue: Res<RenderQueue>) {
    let heights = (draw.height.clone(), draw.previous_height.clone());
    queue.on_submitted_work_done(move || drop(heights));
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
            0,
            u32::from(draw.previous_height.is_some()),
            0,
        ]));
        let field = &draw.field;
        uniform[112..128].copy_from_slice(bytemuck::cast_slice(&[
            field.config().radius_m,
            field.config().height_limit_m,
            0.0,
            0.0,
        ]));
        uniform[128..192].copy_from_slice(bytemuck::cast_slice(&matrix.inverse().to_cols_array()));
        let viewport = camera.viewport.as_ref().map_or(
            [
                0.0,
                0.0,
                surfaces.0.size[0] as f32,
                surfaces.0.size[1] as f32,
            ],
            |v| {
                [
                    v.physical_position.x as f32,
                    v.physical_position.y as f32,
                    v.physical_size.x as f32,
                    v.physical_size.y as f32,
                ]
            },
        );
        uniform[192..208].copy_from_slice(bytemuck::cast_slice(&viewport));
        let eye = view.world_from_view.translation();
        uniform[208..224].copy_from_slice(bytemuck::cast_slice(&[eye.x, eye.y, eye.z, 0.0]));
        if settings.ocean.enabled {
            let ocean = crate::physical_ocean::OceanCamera::new(
                settings.eye,
                field.config().radius_m,
                settings.ocean.sea_level_m,
            );
            uniform[224..240].copy_from_slice(bytemuck::cast_slice(&ocean.radial));
            uniform[240..256].copy_from_slice(bytemuck::cast_slice(&ocean.sphere));
        }
        let now = world.resource::<GpuBridge>().start.elapsed().as_secs_f32();
        uniform[96..112].copy_from_slice(bytemuck::cast_slice(&[
            now,
            draw.height.as_ref().map_or(0.0, |h| h.born),
            0.0,
            SURFACE_BLEND_SECONDS,
        ]));
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
        let mut drawn_tiles = 0;
        let mut tested_tiles = 0;
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
                tested_tiles += height.tiles.len();
                let mut visible: Vec<_> = height
                    .tiles
                    .values()
                    .filter_map(|tile| {
                        tile.bounds
                            .visible_distance(frustum, settings.anchor, eye)
                            .map(|distance| (distance, tile))
                    })
                    .collect();
                visible.sort_by(|a, b| a.0.total_cmp(&b.0));
                for (_, tile) in visible {
                    pass.set_vertex_buffer(0, tile.buffer.slice(..));
                    pass.draw_indexed(0..draw.height_index_count, 0, 0..1);
                    drawn_tiles += 1;
                }
            }
        }
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
        let displayed = world
            .resource::<GpuBridge>()
            .display
            .lock()
            .unwrap()
            .clone();
        if let Ok(mut output) = displayed.output.try_lock() {
            output.stats.surface_target_bytes = surfaces.0.bytes();
            output.stats.draw_ms = start.elapsed().as_secs_f64() * 1000.0;
            output.stats.height_drawn_tiles = drawn_tiles;
            output.stats.height_tested_tiles = tested_tiles;
        }
        Ok(())
    }
}
