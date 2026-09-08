use std::sync::atomic::{AtomicU64, Ordering};

use super::{DiagnosticLayer, ReliefSettings, SURFACE_RADIUS, SurfaceSelection};
use crate::{camera::ViewerCamera, model::GeneratedWorld};
use bevy::{
    asset::{RenderAssetUsages, uuid_handle},
    camera::visibility::NoFrustumCulling,
    ecs::system::SystemParam,
    mesh::{Indices, MeshTag, PrimitiveTopology},
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_graph::{self, RenderGraph, RenderLabel},
        render_resource::{
            AsBindGroup, BindGroup, BindGroupEntry, BindGroupLayoutDescriptor,
            BindGroupLayoutEntry, BindingResource, BindingType, BufferBindingType, BufferSize,
            CachedComputePipelineId, ComputePassDescriptor, ComputePipelineDescriptor,
            PipelineCache, ShaderStages, ShaderType, UniformBuffer,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        storage::{GpuShaderStorageBuffer, ShaderStorageBuffer},
    },
    shader::{Shader, ShaderRef},
};
use procgen_core::Vec3 as ProcgenVec3;
use procgen_cubesphere::{CubeFace, TILE_QUADS, TILE_VERTICES, TileAddress};
use procgen_terrain::{
    TERRAIN_TILE_SAMPLE_COUNT, TERRAIN_WGSL_SOURCE, TerrainGpuParameters, TerrainNoiseKeys,
    fixed_level_4_height_config, pack_control_bake, pack_stamps,
};

/// Camera distance at or below which adjusted elevation uses fixed-level GPU tiles.
pub const TERRAIN_TILE_ZOOM_THRESHOLD: f32 = 1.75;
pub const TERRAIN_TILE_LEVEL: u8 = 4;

const MAX_VISIBLE_TILES: usize = 512;
const TERRAIN_SAMPLE_CAPACITY: usize = MAX_VISIBLE_TILES * TERRAIN_TILE_SAMPLE_COUNT;

const TERRAIN_VERTEX_SHADER: Handle<Shader> = uuid_handle!("f793fce0-ef68-49dd-8c98-7bba8432bc76");
const TERRAIN_COMPUTE_SHADER: Handle<Shader> = uuid_handle!("c7c57793-ae7c-4eb7-a1a9-9b8fb2d5a651");

#[derive(Clone, Copy, Debug, ShaderType)]
struct GpuControlTexel {
    channels_0: Vec4,
    channels_1: Vec4,
}

#[derive(Clone, Copy, Debug, ShaderType)]
struct GpuStamp {
    position_strength: Vec4,
    kind_padding: UVec4,
}

#[derive(Clone, Copy, Debug, ShaderType)]
struct GpuTerrainParameters {
    dimensions: UVec4,
    noise_keys_0: UVec4,
    noise_keys_1: UVec4,
    detail_octaves: UVec4,
    detail: Vec4,
    abyssal: Vec4,
    coast: Vec4,
    stamp_profiles: [Vec4; 4],
}

impl From<TerrainGpuParameters> for GpuTerrainParameters {
    fn from(value: TerrainGpuParameters) -> Self {
        Self {
            dimensions: value.dimensions.into(),
            noise_keys_0: value.noise_keys_0.into(),
            noise_keys_1: value.noise_keys_1.into(),
            detail_octaves: value.detail_octaves.into(),
            detail: value.detail.into(),
            abyssal: value.abyssal.into(),
            coast: value.coast.into(),
            stamp_profiles: value.stamp_profiles.map(Into::into),
        }
    }
}

#[derive(Clone, Copy, Debug, ShaderType)]
struct TerrainDisplayParameters {
    relief_exaggeration: f32,
    surface_radius: f32,
    padding: Vec2,
}

#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
struct TerrainTileExtension {
    #[storage(100, read_only)]
    controls: Handle<ShaderStorageBuffer>,
    #[storage(101, read_only)]
    stamps: Handle<ShaderStorageBuffer>,
    #[uniform(102)]
    parameters: GpuTerrainParameters,
    #[storage(103, read_only)]
    addresses: Handle<ShaderStorageBuffer>,
    #[storage(104, read_only)]
    samples: Handle<ShaderStorageBuffer>,
    #[uniform(105)]
    display: TerrainDisplayParameters,
}

impl MaterialExtension for TerrainTileExtension {
    fn vertex_shader() -> ShaderRef {
        TERRAIN_VERTEX_SHADER.into()
    }
}

type TerrainTileMaterial = ExtendedMaterial<StandardMaterial, TerrainTileExtension>;

#[derive(Resource, Clone, ExtractResource)]
struct TerrainGpuResources {
    controls: Handle<ShaderStorageBuffer>,
    stamps: Handle<ShaderStorageBuffer>,
    addresses: Handle<ShaderStorageBuffer>,
    samples: Handle<ShaderStorageBuffer>,
    parameters: GpuTerrainParameters,
    world_revision: u64,
}

#[derive(Resource, Clone, ExtractResource, ShaderType)]
struct TerrainTileDispatch {
    tile_count: u32,
    generation: u32,
    padding: UVec2,
}

impl Default for TerrainTileDispatch {
    fn default() -> Self {
        Self {
            tile_count: 0,
            generation: 0,
            padding: UVec2::ZERO,
        }
    }
}

#[derive(Resource, Default)]
struct TerrainTileEntities(Vec<Entity>);

#[derive(Resource, Default)]
struct TerrainCoverageState {
    addresses: Vec<TileAddress>,
    world_revision: u64,
}

#[derive(Component)]
pub(super) struct TerrainTileSurface;

pub(super) struct TerrainTileRenderPlugin;

impl Plugin for TerrainTileRenderPlugin {
    fn build(&self, app: &mut App) {
        register_shaders(app);
        app.add_plugins((
            MaterialPlugin::<TerrainTileMaterial>::default(),
            ExtractResourcePlugin::<TerrainGpuResources>::default(),
            ExtractResourcePlugin::<TerrainTileDispatch>::default(),
        ))
        .init_resource::<TerrainTileEntities>()
        .init_resource::<TerrainCoverageState>()
        .init_resource::<TerrainTileDispatch>()
        .add_systems(
            Update,
            (
                initialize_gpu_world.run_if(resource_changed::<GeneratedWorld>),
                update_tile_coverage.run_if(
                    resource_changed::<GeneratedWorld>
                        .or(resource_changed::<SurfaceSelection>)
                        .or(resource_changed::<ReliefSettings>)
                        .or(camera_changed),
                ),
            )
                .chain(),
        );

        let render_app = app.sub_app_mut(RenderApp);
        render_app
            .add_systems(RenderStartup, initialize_compute_pipeline)
            .add_systems(
                Render,
                prepare_compute_bind_group.in_set(RenderSystems::PrepareBindGroups),
            );
        let mut graph = render_app.world_mut().resource_mut::<RenderGraph>();
        graph.add_node(TerrainComputeLabel, TerrainComputeNode::default());
        graph.add_node_edge(TerrainComputeLabel, bevy::render::graph::CameraDriverLabel);
    }
}

pub(super) fn camera_changed(camera: Single<Ref<Transform>, With<ViewerCamera>>) -> bool {
    camera.is_changed()
}

fn register_shaders(app: &mut App) {
    let vertex_source = format!(
        "{TERRAIN_WGSL_SOURCE}\n{}",
        include_str!("terrain_tiles.wgsl")
    );
    let compute_source = format!(
        "{TERRAIN_WGSL_SOURCE}\n{}",
        include_str!("terrain_compute.wgsl")
    );
    let mut shaders = app.world_mut().resource_mut::<Assets<Shader>>();
    shaders
        .insert(
            TERRAIN_VERTEX_SHADER.id(),
            Shader::from_wgsl(vertex_source, "procgen-terrain-viewer.wgsl"),
        )
        .unwrap();
    shaders
        .insert(
            TERRAIN_COMPUTE_SHADER.id(),
            Shader::from_wgsl(compute_source, "procgen-terrain-compute.wgsl"),
        )
        .unwrap();
}

fn initialize_gpu_world(
    mut commands: Commands,
    world: Res<GeneratedWorld>,
    relief: Res<ReliefSettings>,
    previous: Option<Res<TerrainGpuResources>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    mut materials: ResMut<Assets<TerrainTileMaterial>>,
) {
    let world_revision = previous
        .as_ref()
        .map_or(1, |resources| resources.world_revision + 1);
    let controls = pack_control_bake(&world.terrain_control_bake)
        .into_iter()
        .map(|texel| GpuControlTexel {
            channels_0: texel.channels_0.into(),
            channels_1: texel.channels_1.into(),
        })
        .collect::<Vec<_>>();
    let mut stamps = pack_stamps(&world.terrain_controls.stamps)
        .into_iter()
        .map(|stamp| GpuStamp {
            position_strength: stamp.position_strength.into(),
            kind_padding: stamp.kind_padding.into(),
        })
        .collect::<Vec<_>>();
    if stamps.is_empty() {
        stamps.push(GpuStamp {
            position_strength: Vec4::ZERO,
            kind_padding: UVec4::ZERO,
        });
    }
    let controls = buffers.add(ShaderStorageBuffer::from(controls));
    let stamps = buffers.add(ShaderStorageBuffer::from(stamps));
    let addresses = buffers.add(ShaderStorageBuffer::from(vec![
        UVec4::ZERO;
        MAX_VISIBLE_TILES
    ]));
    let samples = buffers.add(ShaderStorageBuffer::with_size(
        TERRAIN_SAMPLE_CAPACITY * size_of::<Vec4>(),
        RenderAssetUsages::default(),
    ));
    let parameters = GpuTerrainParameters::from(TerrainGpuParameters::new(
        &world.terrain_control_bake,
        &world.terrain_controls.stamps,
        TerrainNoiseKeys::new(world.config.fibonacci.seed),
        fixed_level_4_height_config(),
    ));
    let material = materials.add(ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            ..default()
        },
        extension: TerrainTileExtension {
            controls: controls.clone(),
            stamps: stamps.clone(),
            parameters,
            addresses: addresses.clone(),
            samples: samples.clone(),
            display: TerrainDisplayParameters {
                relief_exaggeration: relief.exaggeration,
                surface_radius: SURFACE_RADIUS,
                padding: Vec2::ZERO,
            },
        },
    });
    commands.insert_resource(TerrainTileAssets {
        mesh: Handle::default(),
        material,
    });
    commands.insert_resource(TerrainGpuResources {
        controls,
        stamps,
        addresses,
        samples,
        parameters,
        world_revision,
    });
}

#[derive(Resource)]
struct TerrainTileAssets {
    mesh: Handle<Mesh>,
    material: Handle<TerrainTileMaterial>,
}

#[derive(SystemParam)]
struct TerrainCoverageAssets<'w, 's> {
    commands: Commands<'w, 's>,
    tile_entities: ResMut<'w, TerrainTileEntities>,
    tile_assets: ResMut<'w, TerrainTileAssets>,
    meshes: ResMut<'w, Assets<Mesh>>,
    buffers: ResMut<'w, Assets<ShaderStorageBuffer>>,
    materials: ResMut<'w, Assets<TerrainTileMaterial>>,
}

fn update_tile_coverage(
    camera: Single<&Transform, With<ViewerCamera>>,
    selection: Res<SurfaceSelection>,
    relief: Res<ReliefSettings>,
    resources: Res<TerrainGpuResources>,
    mut dispatch: ResMut<TerrainTileDispatch>,
    mut coverage: ResMut<TerrainCoverageState>,
    mut assets: TerrainCoverageAssets,
) {
    if assets.tile_assets.mesh == Handle::default() {
        assets.tile_assets.mesh = assets.meshes.add(tile_grid_mesh());
    }
    let material_handle = assets.tile_assets.material.clone();
    if let Some(material) = assets.materials.get_mut(&material_handle) {
        material.extension.display.relief_exaggeration = relief.exaggeration;
    }
    let active = terrain_tiles_active(camera.translation.length(), selection.selected());
    let addresses = if active {
        visible_fixed_level_tiles(camera.translation)
    } else {
        Vec::new()
    };
    assert!(addresses.len() <= MAX_VISIBLE_TILES);
    if coverage.addresses == addresses && coverage.world_revision == resources.world_revision {
        return;
    }
    let encoded = addresses
        .iter()
        .copied()
        .map(encode_address)
        .chain(std::iter::repeat(UVec4::ZERO))
        .take(MAX_VISIBLE_TILES)
        .collect::<Vec<_>>();
    assets
        .buffers
        .get_mut(&resources.addresses)
        .expect("terrain address buffer must remain alive")
        .set_data(encoded);

    for entity in assets.tile_entities.0.drain(..) {
        assets.commands.entity(entity).despawn();
    }
    let mesh = assets.tile_assets.mesh.clone();
    let material = assets.tile_assets.material.clone();
    let entities = addresses
        .iter()
        .enumerate()
        .map(|(slot, _)| {
            assets
                .commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    MeshTag(slot as u32),
                    NoFrustumCulling,
                    TerrainTileSurface,
                ))
                .id()
        })
        .collect();
    assets.tile_entities.0 = entities;
    let tile_count = addresses.len() as u32;
    coverage.addresses = addresses;
    coverage.world_revision = resources.world_revision;
    dispatch.tile_count = tile_count;
    dispatch.generation = dispatch.generation.wrapping_add(1);
}

pub(super) fn terrain_tiles_active(
    camera_distance: f32,
    selected: Option<DiagnosticLayer>,
) -> bool {
    camera_distance <= TERRAIN_TILE_ZOOM_THRESHOLD
        && selected == Some(DiagnosticLayer::IsostaticElevation)
}

fn visible_fixed_level_tiles(camera_position: Vec3) -> Vec<TileAddress> {
    let camera_distance = camera_position.length();
    let camera_direction = camera_position / camera_distance;
    let horizon = SURFACE_RADIUS / camera_distance;
    let tiles_per_axis = 1_u32 << TERRAIN_TILE_LEVEL;
    CubeFace::ALL
        .into_iter()
        .flat_map(|face| {
            (0..tiles_per_axis).flat_map(move |y| {
                (0..tiles_per_axis).filter_map(move |x| {
                    let address = TileAddress::new(face, TERRAIN_TILE_LEVEL, x, y).unwrap();
                    tile_may_be_visible(address, camera_direction, horizon).then_some(address)
                })
            })
        })
        .collect()
}

fn tile_may_be_visible(address: TileAddress, camera_direction: Vec3, horizon: f32) -> bool {
    let center = tile_direction(address, TILE_QUADS / 2, TILE_QUADS / 2);
    let extent = [
        tile_direction(address, 0, 0),
        tile_direction(address, TILE_QUADS, 0),
        tile_direction(address, 0, TILE_QUADS),
        tile_direction(address, TILE_QUADS, TILE_QUADS),
    ]
    .into_iter()
    .map(|corner| center.distance(corner))
    .fold(0.0, f32::max);
    center.dot(camera_direction) + extent >= horizon
}

fn tile_direction(address: TileAddress, x: u32, y: u32) -> Vec3 {
    let ProcgenVec3 { x, y, z } = address.grid_vertex(x, y).unwrap().direction();
    Vec3::new(x, y, z)
}

fn encode_address(address: TileAddress) -> UVec4 {
    UVec4::new(
        address.face().index() as u32,
        u32::from(address.level()),
        address.x(),
        address.y(),
    )
}

fn tile_grid_mesh() -> Mesh {
    let positions = (0..TILE_VERTICES)
        .flat_map(|y| (0..TILE_VERTICES).map(move |x| [x as f32, y as f32, 0.0]))
        .collect::<Vec<_>>();
    let normals = vec![[0.0, 0.0, 1.0]; positions.len()];
    let colors = vec![[1.0, 1.0, 1.0, 1.0]; positions.len()];
    let mut indices = Vec::with_capacity((TILE_QUADS * TILE_QUADS * 6) as usize);
    for y in 0..TILE_QUADS {
        for x in 0..TILE_QUADS {
            let lower_left = y * TILE_VERTICES + x;
            let lower_right = lower_left + 1;
            let upper_left = lower_left + TILE_VERTICES;
            let upper_right = upper_left + 1;
            indices.extend([
                lower_left,
                lower_right,
                upper_right,
                lower_left,
                upper_right,
                upper_left,
            ]);
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}

#[derive(Resource)]
struct TerrainComputePipeline {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
}

fn initialize_compute_pipeline(mut commands: Commands, pipeline_cache: Res<PipelineCache>) {
    let entries = vec![
        storage_layout_entry(0, true),
        storage_layout_entry(1, true),
        uniform_layout_entry(2, GpuTerrainParameters::min_size()),
        storage_layout_entry(3, true),
        storage_layout_entry(4, false),
        uniform_layout_entry(5, TerrainTileDispatch::min_size()),
    ];
    let layout = BindGroupLayoutDescriptor::new("terrain tile compute layout", &entries);
    let pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("terrain tile compute pipeline".into()),
        layout: vec![layout.clone()],
        shader: TERRAIN_COMPUTE_SHADER,
        ..default()
    });
    commands.insert_resource(TerrainComputePipeline { layout, pipeline });
}

fn storage_layout_entry(binding: u32, read_only: bool) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::COMPUTE,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_layout_entry(binding: u32, size: BufferSize) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::COMPUTE,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: Some(size),
        },
        count: None,
    }
}

#[derive(Resource)]
struct TerrainComputeBindGroup {
    bind_group: BindGroup,
    _parameters: UniformBuffer<GpuTerrainParameters>,
    _dispatch: UniformBuffer<TerrainTileDispatch>,
}

#[derive(SystemParam)]
struct ComputeBindGroupResources<'w> {
    pipeline: Res<'w, TerrainComputePipeline>,
    pipeline_cache: Res<'w, PipelineCache>,
    resources: Res<'w, TerrainGpuResources>,
    dispatch: Res<'w, TerrainTileDispatch>,
    buffers: Res<'w, RenderAssets<GpuShaderStorageBuffer>>,
    render_device: Res<'w, RenderDevice>,
    render_queue: Res<'w, RenderQueue>,
}

fn prepare_compute_bind_group(mut commands: Commands, resources: ComputeBindGroupResources) {
    let Some(controls) = resources.buffers.get(&resources.resources.controls) else {
        return;
    };
    let Some(stamps) = resources.buffers.get(&resources.resources.stamps) else {
        return;
    };
    let Some(addresses) = resources.buffers.get(&resources.resources.addresses) else {
        return;
    };
    let Some(samples) = resources.buffers.get(&resources.resources.samples) else {
        return;
    };
    let mut parameters = UniformBuffer::from(resources.resources.parameters);
    parameters.write_buffer(&resources.render_device, &resources.render_queue);
    let mut dispatch_buffer = UniformBuffer::from(resources.dispatch.into_inner().clone());
    dispatch_buffer.write_buffer(&resources.render_device, &resources.render_queue);
    let bind_group = resources.render_device.create_bind_group(
        Some("terrain tile compute bind group"),
        &resources
            .pipeline_cache
            .get_bind_group_layout(&resources.pipeline.layout),
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(controls.buffer.as_entire_buffer_binding()),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Buffer(stamps.buffer.as_entire_buffer_binding()),
            },
            BindGroupEntry {
                binding: 2,
                resource: parameters.binding().unwrap(),
            },
            BindGroupEntry {
                binding: 3,
                resource: BindingResource::Buffer(addresses.buffer.as_entire_buffer_binding()),
            },
            BindGroupEntry {
                binding: 4,
                resource: BindingResource::Buffer(samples.buffer.as_entire_buffer_binding()),
            },
            BindGroupEntry {
                binding: 5,
                resource: dispatch_buffer.binding().unwrap(),
            },
        ],
    );
    commands.insert_resource(TerrainComputeBindGroup {
        bind_group,
        _parameters: parameters,
        _dispatch: dispatch_buffer,
    });
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct TerrainComputeLabel;

#[derive(Default)]
struct TerrainComputeNode {
    completed_generation: AtomicU64,
}

impl render_graph::Node for TerrainComputeNode {
    fn run(
        &self,
        _graph: &mut render_graph::RenderGraphContext,
        render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), render_graph::NodeRunError> {
        let dispatch = world.resource::<TerrainTileDispatch>();
        let generation = u64::from(dispatch.generation);
        if dispatch.tile_count == 0
            || self.completed_generation.load(Ordering::Relaxed) == generation
        {
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
        let invocation_count = dispatch.tile_count as usize * TERRAIN_TILE_SAMPLE_COUNT;
        let mut pass =
            render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor {
                    label: Some("terrain tile generation"),
                    ..default()
                });
        pass.set_pipeline(compute_pipeline);
        pass.set_bind_group(0, &bind_group.bind_group, &[]);
        pass.dispatch_workgroups(invocation_count.div_ceil(64) as u32, 1, 1);
        self.completed_generation
            .store(generation, Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    #[test]
    fn tile_grid_triangles_face_outward_on_every_cube_face() {
        let mesh = tile_grid_mesh();
        let VertexAttributeValues::Float32x3(positions) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
        else {
            panic!("terrain tile positions must be float3");
        };
        let Indices::U32(indices) = mesh.indices().unwrap() else {
            panic!("terrain tile indices must be u32");
        };

        for face in CubeFace::ALL {
            let address = TileAddress::new(face, 0, 0, 0).unwrap();
            for triangle in indices.chunks_exact(3) {
                let directions = [triangle[0], triangle[1], triangle[2]].map(|index| {
                    let [x, y, _] = positions[index as usize];
                    tile_direction(address, x as u32, y as u32)
                });
                let geometric_normal =
                    (directions[1] - directions[0]).cross(directions[2] - directions[0]);
                let center = (directions[0] + directions[1] + directions[2]).normalize();
                assert!(geometric_normal.dot(center) > 0.0);
            }
        }
    }

    #[test]
    fn mode_only_replaces_final_adjusted_elevation_below_threshold() {
        assert!(terrain_tiles_active(
            TERRAIN_TILE_ZOOM_THRESHOLD,
            Some(DiagnosticLayer::IsostaticElevation)
        ));
        assert!(!terrain_tiles_active(
            TERRAIN_TILE_ZOOM_THRESHOLD + f32::EPSILON,
            Some(DiagnosticLayer::IsostaticElevation)
        ));
        assert!(!terrain_tiles_active(
            1.25,
            Some(DiagnosticLayer::GeologicalElevation)
        ));
        assert!(!terrain_tiles_active(1.25, None));
    }

    #[test]
    fn fixed_level_coverage_contains_every_geometrically_visible_tile() {
        for camera in [Vec3::Z * 1.25, Vec3::new(1.0, 0.4, -0.7).normalize() * 1.75] {
            let selected = visible_fixed_level_tiles(camera);
            assert!(selected.len() <= MAX_VISIBLE_TILES);
            let horizon = SURFACE_RADIUS / camera.length();
            let camera_direction = camera.normalize();
            for face in CubeFace::ALL {
                for y in 0..1_u32 << TERRAIN_TILE_LEVEL {
                    for x in 0..1_u32 << TERRAIN_TILE_LEVEL {
                        let address = TileAddress::new(face, TERRAIN_TILE_LEVEL, x, y).unwrap();
                        let has_visible_vertex = (0..=TILE_QUADS)
                            .step_by(TILE_QUADS as usize / 4)
                            .any(|local_y| {
                                (0..=TILE_QUADS)
                                    .step_by(TILE_QUADS as usize / 4)
                                    .any(|local_x| {
                                        tile_direction(address, local_x, local_y)
                                            .dot(camera_direction)
                                            >= horizon
                                    })
                            });
                        assert!(!has_visible_vertex || selected.contains(&address));
                    }
                }
            }
        }
    }

    #[test]
    fn world_replacement_invalidates_every_world_owned_gpu_resource() {
        use crate::test_support::fixture;
        use bevy::asset::{AssetApp, AssetPlugin};

        let mut app = App::new();
        app.add_plugins(AssetPlugin::default())
            .init_asset::<ShaderStorageBuffer>()
            .init_asset::<TerrainTileMaterial>()
            .insert_resource(ReliefSettings::default())
            .insert_resource(TerrainTileEntities::default())
            .insert_resource(fixture(64, 70))
            .add_systems(
                Update,
                initialize_gpu_world.run_if(resource_changed::<GeneratedWorld>),
            );
        app.update();
        let first = app.world().resource::<TerrainGpuResources>();
        let first_ids = (
            first.controls.id(),
            first.stamps.id(),
            first.addresses.id(),
            first.samples.id(),
            first.world_revision,
        );

        app.insert_resource(fixture(64, 71));
        app.update();
        let second = app.world().resource::<TerrainGpuResources>();
        assert_ne!(
            first_ids,
            (
                second.controls.id(),
                second.stamps.id(),
                second.addresses.id(),
                second.samples.id(),
                second.world_revision,
            )
        );
        assert_eq!(second.world_revision, first_ids.4 + 1);
    }
}
