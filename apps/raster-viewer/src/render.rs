//! Six cube-face grids that sample the resident partition buffers.
//!
//! Grid density is a display setting: the grids only carry geometry, and every
//! per-cell lookup happens in the fragment shader against the growth-label
//! buffer, so the rendered boundaries stay at texel resolution however coarse
//! the grid is.

use crate::partition::ResidentPartition;
use bevy::{
    asset::{RenderAssetUsages, uuid_handle},
    camera::visibility::NoFrustumCulling,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::{
        render_resource::{AsBindGroup, Buffer, ShaderType},
        storage::ShaderStorageBuffer,
    },
    shader::{Shader, ShaderRef},
};
use procgen_cubesphere::{CubeFace, FaceCoordinates, MAPPING_WGSL_SOURCE, face_to_direction};
use procgen_raster_tectonics::{MAX_PLATE_COUNT, UNCLAIMED_PLATE};
use procgen_viewer_support::id_color;

/// Radius the face grids are drawn at.
const SURFACE_RADIUS: f32 = 1.0;
/// Colour of a cell no plate reached, which only a starved head start leaves.
const UNCLAIMED_COLOR: Color = Color::srgb(0.08, 0.08, 0.1);
/// One colour per addressable plate, plus the reserved unclaimed id.
const PALETTE_LEN: usize = MAX_PLATE_COUNT as usize + 1;

pub const GRID_QUAD_RANGE: std::ops::RangeInclusive<u32> = 16..=256;
const DEFAULT_GRID_QUADS: u32 = 128;

const FACE_FRAGMENT_SHADER: Handle<Shader> = uuid_handle!("2f36f2f5-0a1e-4a0e-9a54-2a8e0f4d0f11");

/// How finely the face grids are tessellated, independent of texel resolution.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplaySettings {
    pub grid_quads: u32,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            grid_quads: DEFAULT_GRID_QUADS,
        }
    }
}

#[derive(Clone, Copy, Debug, ShaderType)]
struct PlateFaceDisplay {
    resolution: u32,
}

#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
struct PlateFaceMaterial {
    #[storage(0, read_only, buffer)]
    growth_labels: Buffer,
    #[uniform(1)]
    display: PlateFaceDisplay,
    #[storage(2, read_only)]
    palette: Handle<ShaderStorageBuffer>,
}

impl Material for PlateFaceMaterial {
    fn fragment_shader() -> ShaderRef {
        FACE_FRAGMENT_SHADER.into()
    }
}

#[derive(Resource)]
struct FaceGrids([Handle<Mesh>; CubeFace::ALL.len()]);

#[derive(Resource)]
struct FaceMaterial(Handle<PlateFaceMaterial>);

/// The identity ramp, resolved to linear colour on the host so the shader is a
/// lookup rather than a second implementation of Bevy's colour conversions.
#[derive(Resource)]
struct PlatePalette(Handle<ShaderStorageBuffer>);

#[derive(Component)]
struct FaceGrid;

pub struct FaceGridRenderPlugin;

impl Plugin for FaceGridRenderPlugin {
    fn build(&self, app: &mut App) {
        register_shader(app);
        app.add_plugins(MaterialPlugin::<PlateFaceMaterial>::default())
            .init_resource::<DisplaySettings>()
            .add_systems(Startup, initialize_palette)
            .add_systems(
                Update,
                (
                    sync_grid_meshes.run_if(resource_changed::<DisplaySettings>),
                    sync_material.run_if(resource_exists_and_changed::<ResidentPartition>),
                    sync_faces.run_if(faces_are_stale),
                )
                    .chain(),
            );
    }
}

fn register_shader(app: &mut App) {
    let constants = format!("const RASTER_PLATE_LABEL_MASK: u32 = {UNCLAIMED_PLATE}u;");
    let source = [MAPPING_WGSL_SOURCE, &constants, include_str!("faces.wgsl")].join("\n");
    app.world_mut()
        .resource_mut::<Assets<Shader>>()
        .insert(
            FACE_FRAGMENT_SHADER.id(),
            Shader::from_wgsl(source, "procgen-raster-faces.wgsl"),
        )
        .expect("the face shader handle must be free");
}

fn initialize_palette(mut commands: Commands, mut buffers: ResMut<Assets<ShaderStorageBuffer>>) {
    let palette: Vec<[f32; 4]> = (0..PALETTE_LEN)
        .map(|plate| {
            let color = if plate == UNCLAIMED_PLATE as usize {
                UNCLAIMED_COLOR
            } else {
                id_color(plate)
            };
            color.to_linear().to_f32_array()
        })
        .collect();
    commands.insert_resource(PlatePalette(buffers.add(ShaderStorageBuffer::new(
        bytemuck::cast_slice(&palette),
        RenderAssetUsages::default(),
    ))));
}

fn sync_grid_meshes(
    mut commands: Commands,
    display: Res<DisplaySettings>,
    mut meshes: ResMut<Assets<Mesh>>,
    grids: Option<ResMut<FaceGrids>>,
) {
    let built = CubeFace::ALL.map(|face| meshes.add(face_grid_mesh(face, display.grid_quads)));
    match grids {
        Some(mut grids) => grids.0 = built,
        None => commands.insert_resource(FaceGrids(built)),
    }
}

fn sync_material(
    mut commands: Commands,
    partition: Res<ResidentPartition>,
    palette: Res<PlatePalette>,
    mut materials: ResMut<Assets<PlateFaceMaterial>>,
    existing: Option<Res<FaceMaterial>>,
) {
    let pipeline = partition.pipeline();
    let material = PlateFaceMaterial {
        growth_labels: Buffer::from(pipeline.growth_label_buffer().clone()),
        display: PlateFaceDisplay {
            resolution: pipeline.resolution(),
        },
        palette: palette.0.clone(),
    };
    match existing {
        Some(existing) => {
            *materials
                .get_mut(&existing.0)
                .expect("the face material outlives the resident partition") = material;
        }
        None => commands.insert_resource(FaceMaterial(materials.add(material))),
    }
}

fn faces_are_stale(grids: Option<Res<FaceGrids>>, material: Option<Res<FaceMaterial>>) -> bool {
    let (Some(grids), Some(material)) = (grids, material) else {
        return false;
    };
    grids.is_changed() || material.is_changed()
}

fn sync_faces(
    mut commands: Commands,
    grids: Res<FaceGrids>,
    material: Res<FaceMaterial>,
    faces: Query<Entity, With<FaceGrid>>,
) {
    for face in faces {
        commands.entity(face).despawn();
    }
    for mesh in &grids.0 {
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.0.clone()),
            NoFrustumCulling,
            FaceGrid,
        ));
    }
}

/// Builds one cube face as a sphere-projected grid. The grid carries geometry
/// only; the shader resolves which cell each fragment falls in.
fn face_grid_mesh(face: CubeFace, quads: u32) -> Mesh {
    let vertices = quads + 1;
    let directions: Vec<Vec3> = (0..vertices)
        .flat_map(|y| (0..vertices).map(move |x| (x, y)))
        .map(|(x, y)| {
            let direction = face_to_direction(FaceCoordinates {
                face,
                u: -1.0 + 2.0 * x as f32 / quads as f32,
                v: -1.0 + 2.0 * y as f32 / quads as f32,
            })
            .expect("grid coordinates stay inside the face");
            Vec3::new(direction.x, direction.y, direction.z)
        })
        .collect();
    let mut indices = Vec::with_capacity((quads * quads * 6) as usize);
    for y in 0..quads {
        for x in 0..quads {
            let lower_left = y * vertices + x;
            let lower_right = lower_left + 1;
            let upper_left = lower_left + vertices;
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
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        directions
            .iter()
            .map(|&direction| direction * SURFACE_RADIUS)
            .collect::<Vec<_>>(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, directions)
    .with_inserted_indices(Indices::U32(indices))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    #[test]
    fn face_grid_triangles_face_outward_on_every_cube_face() {
        for face in CubeFace::ALL {
            let mesh = face_grid_mesh(face, 4);
            let VertexAttributeValues::Float32x3(positions) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
            else {
                panic!("face grid positions must be float3");
            };
            let Indices::U32(indices) = mesh.indices().unwrap() else {
                panic!("face grid indices must be u32");
            };
            assert_eq!(positions.len(), 25);
            assert_eq!(indices.len(), 4 * 4 * 6);
            for triangle in indices.chunks_exact(3) {
                let corners = [triangle[0], triangle[1], triangle[2]]
                    .map(|index| Vec3::from(positions[index as usize]));
                let normal = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
                let center = (corners[0] + corners[1] + corners[2]).normalize();
                assert!(normal.dot(center) > 0.0, "{face:?} winds inward");
            }
        }
    }

    #[test]
    fn grid_vertices_lie_on_the_display_sphere() {
        let mesh = face_grid_mesh(CubeFace::NegativeY, 8);
        let VertexAttributeValues::Float32x3(positions) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
        else {
            panic!("face grid positions must be float3");
        };
        for position in positions {
            assert!((Vec3::from(*position).length() - SURFACE_RADIUS).abs() <= 1.0e-6);
        }
    }
}
