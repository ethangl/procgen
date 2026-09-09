use std::sync::atomic::Ordering;

use super::{TerrainGpuResources, TerrainTileDispatch};
use bevy::{
    ecs::system::SystemParam,
    prelude::*,
    render::{
        Render, RenderStartup, RenderSystems,
        render_asset::RenderAssets,
        render_graph::{self, RenderGraph, RenderLabel},
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
            CachedComputePipelineId, ComputePassDescriptor, ComputePipelineDescriptor,
            PipelineCache, ShaderStages,
            binding_types::{storage_buffer_read_only_sized, storage_buffer_sized},
        },
        renderer::{RenderContext, RenderDevice},
        storage::GpuShaderStorageBuffer,
    },
};
use procgen_terrain::TERRAIN_TILE_SAMPLE_COUNT;

pub(super) fn install(render_app: &mut SubApp) {
    render_app
        .add_systems(RenderStartup, initialize_pipeline)
        .add_systems(
            Render,
            // The world-owned buffers only reach the render world once the
            // geology phase has produced terrain controls.
            prepare_bind_group
                .in_set(RenderSystems::PrepareBindGroups)
                .run_if(resource_exists::<TerrainGpuResources>),
        );
    let mut graph = render_app.world_mut().resource_mut::<RenderGraph>();
    graph.add_node(TerrainComputeLabel, TerrainComputeNode);
    graph.add_node_edge(TerrainComputeLabel, bevy::render::graph::CameraDriverLabel);
}

#[derive(Resource)]
struct TerrainComputePipeline {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
}

fn initialize_pipeline(mut commands: Commands, pipeline_cache: Res<PipelineCache>) {
    let entries = BindGroupLayoutEntries::sequential(
        ShaderStages::COMPUTE,
        (
            storage_buffer_read_only_sized(false, None),
            storage_buffer_read_only_sized(false, None),
            storage_buffer_read_only_sized(false, None),
            storage_buffer_read_only_sized(false, None),
            storage_buffer_sized(false, None),
            storage_buffer_sized(false, None),
        ),
    );
    let layout = BindGroupLayoutDescriptor::new("terrain tile compute layout", &entries);
    let pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("terrain tile compute pipeline".into()),
        layout: vec![layout.clone()],
        shader: super::TERRAIN_COMPUTE_SHADER,
        ..default()
    });
    commands.insert_resource(TerrainComputePipeline { layout, pipeline });
}

#[derive(Resource)]
struct TerrainComputeBindGroup(BindGroup);

#[derive(SystemParam)]
struct BindGroupResources<'w> {
    pipeline: Res<'w, TerrainComputePipeline>,
    pipeline_cache: Res<'w, PipelineCache>,
    resources: Res<'w, TerrainGpuResources>,
    buffers: Res<'w, RenderAssets<GpuShaderStorageBuffer>>,
    render_device: Res<'w, RenderDevice>,
}

fn prepare_bind_group(
    mut commands: Commands,
    existing: Option<Res<TerrainComputeBindGroup>>,
    resources: BindGroupResources,
) {
    if existing.is_some() && !resources.resources.is_changed() {
        return;
    }
    commands.remove_resource::<TerrainComputeBindGroup>();
    let handles = &resources.resources;
    let Some(controls) = resources.buffers.get(&handles.controls) else {
        return;
    };
    let Some(stamps) = resources.buffers.get(&handles.stamps) else {
        return;
    };
    let Some(parameters) = resources.buffers.get(&handles.parameters) else {
        return;
    };
    let Some(jobs) = resources.buffers.get(&handles.jobs) else {
        return;
    };
    let Some(addresses) = resources.buffers.get(&handles.addresses) else {
        return;
    };
    let Some(samples) = resources.buffers.get(&handles.samples) else {
        return;
    };
    let entries = BindGroupEntries::sequential((
        controls.buffer.as_entire_buffer_binding(),
        stamps.buffer.as_entire_buffer_binding(),
        parameters.buffer.as_entire_buffer_binding(),
        jobs.buffer.as_entire_buffer_binding(),
        addresses.buffer.as_entire_buffer_binding(),
        samples.buffer.as_entire_buffer_binding(),
    ));
    let bind_group = resources.render_device.create_bind_group(
        Some("terrain tile compute bind group"),
        &resources
            .pipeline_cache
            .get_bind_group_layout(&resources.pipeline.layout),
        &entries,
    );
    commands.insert_resource(TerrainComputeBindGroup(bind_group));
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct TerrainComputeLabel;

struct TerrainComputeNode;

impl render_graph::Node for TerrainComputeNode {
    fn run(
        &self,
        _graph: &mut render_graph::RenderGraphContext,
        render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), render_graph::NodeRunError> {
        let dispatch = world.resource::<TerrainTileDispatch>();
        let generation = dispatch.generation;
        if dispatch.job_count == 0 || dispatch.is_complete() {
            return Ok(());
        }
        let pipeline_cache = world.resource::<PipelineCache>();
        let pipeline = world.resource::<TerrainComputePipeline>();
        let Some(compute_pipeline) = pipeline_cache.get_compute_pipeline(pipeline.pipeline) else {
            return Ok(());
        };
        let Some(bind_group) = world.get_resource::<TerrainComputeBindGroup>() else {
            return Ok(());
        };
        let invocation_count = dispatch.job_count as usize * TERRAIN_TILE_SAMPLE_COUNT;
        let mut pass =
            render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor {
                    label: Some("terrain tile generation"),
                    ..default()
                });
        pass.set_pipeline(compute_pipeline);
        pass.set_bind_group(0, &bind_group.0, &[]);
        pass.dispatch_workgroups(invocation_count.div_ceil(64) as u32, 1, 1);
        dispatch
            .completed_generation
            .store(generation, Ordering::Release);
        Ok(())
    }
}
