mod compute;
mod coverage;

use coverage::visible_fixed_level_tiles;

use super::{
    DiagnosticLayer, ReliefSettings, SURFACE_RADIUS, SurfaceSelection,
    palette::ELEVATION_COLOR_STOPS,
};
use crate::{camera::ViewerCamera, model::GeneratedWorld};
use bevy::{
    asset::{RenderAssetUsages, uuid_handle},
    camera::visibility::NoFrustumCulling,
    ecs::system::SystemParam,
    mesh::{Indices, MeshTag, PrimitiveTopology},
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::{
        RenderApp,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_resource::{AsBindGroup, ShaderType},
        storage::ShaderStorageBuffer,
    },
    shader::{Shader, ShaderRef},
};
use procgen_cubesphere::{MAPPING_WGSL_SOURCE, TILE_QUADS, TILE_VERTICES, TileAddress};
use procgen_terrain::{
    TERRAIN_TILE_SAMPLE_COUNT, TERRAIN_WGSL_SOURCE, TerrainGpuParameters, TerrainGpuStamp,
    TerrainNoiseKeys, fixed_level_4_height_config, pack_control_bake, pack_stamps,
};

/// Camera distance at or below which adjusted elevation uses fixed-level GPU tiles.
pub const TERRAIN_TILE_ZOOM_THRESHOLD: f32 = 1.75;
pub const TERRAIN_TILE_LEVEL: u8 = 4;

const MAX_VISIBLE_TILES: usize = 512;
const TERRAIN_SAMPLE_CAPACITY: usize = MAX_VISIBLE_TILES * TERRAIN_TILE_SAMPLE_COUNT;
const ELEVATION_PALETTE_STOP_COUNT: usize = ELEVATION_COLOR_STOPS.len();

const TERRAIN_VERTEX_SHADER: Handle<Shader> = uuid_handle!("f793fce0-ef68-49dd-8c98-7bba8432bc76");
const TERRAIN_COMPUTE_SHADER: Handle<Shader> = uuid_handle!("c7c57793-ae7c-4eb7-a1a9-9b8fb2d5a651");

#[derive(Clone, Copy, Debug, ShaderType)]
struct TerrainDisplayParameters {
    relief_exaggeration: f32,
    surface_radius: f32,
    padding: Vec2,
}

#[derive(Clone, Copy, Debug, ShaderType)]
struct TerrainElevationPalette {
    stops: [Vec4; ELEVATION_PALETTE_STOP_COUNT],
}

#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
struct TerrainTileExtension {
    #[storage(100, read_only)]
    addresses: Handle<ShaderStorageBuffer>,
    #[storage(101, read_only)]
    samples: Handle<ShaderStorageBuffer>,
    #[uniform(102)]
    display: TerrainDisplayParameters,
    #[uniform(103)]
    palette: TerrainElevationPalette,
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
    parameters: Handle<ShaderStorageBuffer>,
    addresses: Handle<ShaderStorageBuffer>,
    samples: Handle<ShaderStorageBuffer>,
}

#[derive(Resource)]
struct TerrainTileAssets {
    mesh: Handle<Mesh>,
    material: Handle<TerrainTileMaterial>,
}

#[derive(Resource, Clone, ExtractResource, Default)]
struct TerrainTileDispatch {
    tile_count: u32,
    generation: u32,
}

#[derive(Resource, Default)]
struct TerrainTileEntities(Vec<Entity>);

#[derive(Resource, Default)]
struct TerrainCoverageState {
    addresses: Vec<TileAddress>,
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum TerrainTileMode {
    #[default]
    Coarse,
    Tiles,
}

#[derive(Resource)]
struct TerrainGridMesh(Handle<Mesh>);

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
        .init_resource::<TerrainTileMode>()
        .add_systems(Startup, initialize_grid_mesh)
        .add_systems(
            Update,
            (
                initialize_gpu_world.run_if(resource_changed::<GeneratedWorld>),
                sync_mode.run_if(resource_changed::<SurfaceSelection>.or(camera_changed)),
                sync_relief.run_if(
                    resource_changed::<ReliefSettings>.and(resource_exists::<TerrainTileAssets>),
                ),
                update_tile_coverage.run_if(
                    resource_changed::<GeneratedWorld>
                        .or(resource_changed::<TerrainTileMode>)
                        .or(camera_changed),
                ),
            )
                .chain(),
        );

        compute::install(app.sub_app_mut(RenderApp));
    }
}

fn camera_changed(camera: Single<Ref<Transform>, With<ViewerCamera>>) -> bool {
    camera.is_changed()
}

pub(super) fn sync_mode(
    camera: Single<&Transform, With<ViewerCamera>>,
    selection: Res<SurfaceSelection>,
    mut mode: ResMut<TerrainTileMode>,
) {
    let next = if terrain_tiles_active(camera.translation.length(), selection.selected()) {
        TerrainTileMode::Tiles
    } else {
        TerrainTileMode::Coarse
    };
    if *mode != next {
        *mode = next;
    }
}

fn initialize_grid_mesh(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    commands.insert_resource(TerrainGridMesh(meshes.add(tile_grid_mesh())));
}

fn register_shaders(app: &mut App) {
    let vertex_source = format!(
        "{MAPPING_WGSL_SOURCE}\nconst TERRAIN_ELEVATION_PALETTE_STOP_COUNT: u32 = {ELEVATION_PALETTE_STOP_COUNT}u;\n{}",
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

#[derive(SystemParam)]
struct TerrainWorldAssets<'w, 's> {
    commands: Commands<'w, 's>,
    grid: Res<'w, TerrainGridMesh>,
    tile_entities: ResMut<'w, TerrainTileEntities>,
    coverage: ResMut<'w, TerrainCoverageState>,
    buffers: ResMut<'w, Assets<ShaderStorageBuffer>>,
    materials: ResMut<'w, Assets<TerrainTileMaterial>>,
}

fn initialize_gpu_world(
    world: Res<GeneratedWorld>,
    relief: Res<ReliefSettings>,
    mut assets: TerrainWorldAssets,
) {
    for entity in assets.tile_entities.0.drain(..) {
        assets.commands.entity(entity).despawn();
    }
    assets.coverage.addresses.clear();

    let controls = pack_control_bake(&world.terrain_control_bake);
    let mut stamps = pack_stamps(&world.terrain_controls.stamps);
    // wgpu storage bindings cannot be empty; the shader reads no stamps when stamp_count is zero.
    if stamps.is_empty() {
        stamps.push(TerrainGpuStamp {
            position: [0.0; 3],
            strength: 0.0,
            kind: 0,
            padding: [0; 3],
        });
    }
    let parameters = TerrainGpuParameters::new(
        &world.terrain_control_bake,
        &world.terrain_controls.stamps,
        TerrainNoiseKeys::new(world.config.fibonacci.seed),
        fixed_level_4_height_config(),
    );
    let controls = assets.buffers.add(ShaderStorageBuffer::new(
        bytemuck::cast_slice(&controls),
        RenderAssetUsages::default(),
    ));
    let stamps = assets.buffers.add(ShaderStorageBuffer::new(
        bytemuck::cast_slice(&stamps),
        RenderAssetUsages::default(),
    ));
    let parameters = assets.buffers.add(ShaderStorageBuffer::new(
        bytemuck::bytes_of(&parameters),
        RenderAssetUsages::default(),
    ));
    // This placeholder keeps the binding valid until the first nonempty visible coverage upload.
    let addresses = assets.buffers.add(ShaderStorageBuffer::new(
        bytemuck::bytes_of(&[0_u32; 4]),
        RenderAssetUsages::default(),
    ));
    let samples = assets.buffers.add(ShaderStorageBuffer::with_size(
        TERRAIN_SAMPLE_CAPACITY * size_of::<Vec4>(),
        RenderAssetUsages::default(),
    ));
    let material = assets.materials.add(ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            ..default()
        },
        extension: TerrainTileExtension {
            addresses: addresses.clone(),
            samples: samples.clone(),
            display: TerrainDisplayParameters {
                relief_exaggeration: relief.exaggeration,
                surface_radius: SURFACE_RADIUS,
                padding: Vec2::ZERO,
            },
            palette: elevation_palette(),
        },
    });
    assets.commands.insert_resource(TerrainGpuResources {
        controls,
        stamps,
        parameters,
        addresses,
        samples,
    });
    assets.commands.insert_resource(TerrainTileAssets {
        mesh: assets.grid.0.clone(),
        material,
    });
}

fn elevation_palette() -> TerrainElevationPalette {
    TerrainElevationPalette {
        stops: std::array::from_fn(|index| {
            let (value, color) = ELEVATION_COLOR_STOPS[index];
            Vec4::new(color.x, color.y, color.z, value)
        }),
    }
}

#[derive(SystemParam)]
struct TerrainCoverageAssets<'w, 's> {
    commands: Commands<'w, 's>,
    tile_entities: ResMut<'w, TerrainTileEntities>,
    resources: ResMut<'w, TerrainGpuResources>,
    tile_assets: Res<'w, TerrainTileAssets>,
    buffers: ResMut<'w, Assets<ShaderStorageBuffer>>,
    materials: ResMut<'w, Assets<TerrainTileMaterial>>,
}

fn sync_relief(
    relief: Res<ReliefSettings>,
    tile_assets: Res<TerrainTileAssets>,
    mut materials: ResMut<Assets<TerrainTileMaterial>>,
) {
    materials
        .get_mut(&tile_assets.material)
        .expect("terrain tile material must remain alive")
        .extension
        .display
        .relief_exaggeration = relief.exaggeration;
}

fn update_tile_coverage(
    camera: Single<&Transform, With<ViewerCamera>>,
    mode: Res<TerrainTileMode>,
    mut dispatch: ResMut<TerrainTileDispatch>,
    mut coverage: ResMut<TerrainCoverageState>,
    mut assets: TerrainCoverageAssets,
) {
    let addresses = if *mode == TerrainTileMode::Tiles {
        visible_fixed_level_tiles(camera.translation)
    } else {
        Vec::new()
    };
    if coverage.addresses == addresses {
        return;
    }
    if !addresses.is_empty() {
        let encoded = addresses
            .iter()
            .copied()
            .map(TileAddress::gpu_words)
            .collect::<Vec<_>>();
        let address_buffer = assets.buffers.add(ShaderStorageBuffer::new(
            bytemuck::cast_slice(&encoded),
            RenderAssetUsages::default(),
        ));
        // A new handle invalidates both the compute bind group's change gate and Bevy's
        // prepared material binding; mutating the old asset can leave either GPU binding stale.
        assets.resources.addresses = address_buffer.clone();
        assets
            .materials
            .get_mut(&assets.tile_assets.material)
            .expect("terrain tile material must remain alive")
            .extension
            .addresses = address_buffer;
    }

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
                ))
                .id()
        })
        .collect();
    assets.tile_entities.0 = entities;
    let tile_count = addresses.len() as u32;
    coverage.addresses = addresses;
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

fn tile_grid_mesh() -> Mesh {
    // Integer grid coordinates are decoded by the custom vertex shader; normals and colors are
    // present only to select Bevy's lit, vertex-color mesh pipeline layout.
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;
    use procgen_cubesphere::CubeFace;

    use super::coverage::tile_direction;

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
    fn mode_resource_changes_only_when_the_rendering_mode_changes() {
        let mut app = App::new();
        app.init_resource::<SurfaceSelection>()
            .init_resource::<TerrainTileMode>()
            .add_systems(Update, sync_mode);
        let camera = app
            .world_mut()
            .spawn((Transform::from_xyz(0.0, 0.0, 3.2), ViewerCamera))
            .id();

        app.update();
        assert_eq!(
            *app.world().resource::<TerrainTileMode>(),
            TerrainTileMode::Coarse
        );
        app.world_mut().clear_trackers();
        app.update();
        assert!(!app.world().resource_ref::<TerrainTileMode>().is_changed());

        app.world_mut()
            .entity_mut(camera)
            .get_mut::<Transform>()
            .unwrap()
            .translation = Vec3::Z * TERRAIN_TILE_ZOOM_THRESHOLD;
        app.update();
        assert_eq!(
            *app.world().resource::<TerrainTileMode>(),
            TerrainTileMode::Tiles
        );
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
    fn visible_coverage_stays_within_the_proven_capacity() {
        let golden_angle = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
        for distance in [1.25, TERRAIN_TILE_ZOOM_THRESHOLD] {
            for index in 0..3_000 {
                let y = 1.0 - 2.0 * (index as f32 + 0.5) / 3_000.0;
                let radius = (1.0 - y * y).sqrt();
                let longitude = index as f32 * golden_angle;
                let direction = Vec3::new(radius * longitude.cos(), y, radius * longitude.sin());
                assert!(visible_fixed_level_tiles(direction * distance).len() <= MAX_VISIBLE_TILES);
            }
        }
    }

    #[test]
    fn world_replacement_resets_coverage_and_world_owned_gpu_resources() {
        use crate::test_support::fixture;
        use bevy::asset::{AssetApp, AssetPlugin};

        let mut app = App::new();
        app.add_plugins(AssetPlugin::default())
            .init_asset::<ShaderStorageBuffer>()
            .init_asset::<TerrainTileMaterial>()
            .insert_resource(ReliefSettings::default())
            .insert_resource(TerrainTileEntities::default())
            .insert_resource(TerrainCoverageState::default())
            .insert_resource(TerrainTileDispatch::default())
            .insert_resource(TerrainGridMesh(Handle::default()))
            .insert_resource(fixture(64, 70))
            .add_systems(
                Update,
                initialize_gpu_world.run_if(resource_changed::<GeneratedWorld>),
            );
        app.update();
        let first = app.world().resource::<TerrainGpuResources>();
        let first_tile_assets = app.world().resource::<TerrainTileAssets>();
        let first_ids = (
            first.controls.id(),
            first.stamps.id(),
            first.parameters.id(),
            first.addresses.id(),
            first.samples.id(),
            first_tile_assets.material.id(),
        );

        let stale_address =
            TileAddress::new(CubeFace::PositiveZ, TERRAIN_TILE_LEVEL, 0, 0).unwrap();
        app.world_mut()
            .resource_mut::<TerrainCoverageState>()
            .addresses
            .push(stale_address);
        let stale_entity = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<TerrainTileEntities>()
            .0
            .push(stale_entity);

        app.insert_resource(fixture(64, 71));
        app.update();
        let second = app.world().resource::<TerrainGpuResources>();
        let second_tile_assets = app.world().resource::<TerrainTileAssets>();
        assert_ne!(
            first_ids,
            (
                second.controls.id(),
                second.stamps.id(),
                second.parameters.id(),
                second.addresses.id(),
                second.samples.id(),
                second_tile_assets.material.id(),
            )
        );
        assert!(
            app.world()
                .resource::<TerrainCoverageState>()
                .addresses
                .is_empty()
        );
        assert!(app.world().get_entity(stale_entity).is_err());
    }
}
