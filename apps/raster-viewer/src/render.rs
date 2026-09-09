//! Six cube-face grids that sample the resident tectonics buffers.
//!
//! Grid density and the layer on show are display settings: the grids only
//! carry geometry, and every per-cell lookup happens in the fragment shader
//! against the pipeline's own buffers, so the rendered boundaries stay at texel
//! resolution however coarse the grid is.

use crate::tectonics::ResidentTectonics;
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
use procgen_cubesphere::{
    CubeFace, FaceCoordinates, MAPPING_WGSL_SOURCE, RASTER_WGSL_SOURCE, face_to_direction,
};
use procgen_raster_tectonics::{
    PLATE_ID_COUNT, UNCLAIMED_PLATE, boundary_class_code, crust_class_code, field_wgsl_source,
};
use procgen_tectonics::{BoundaryClass, CrustClass};
use procgen_viewer_support::id_color;

/// Radius the face grids are drawn at.
const SURFACE_RADIUS: f32 = 1.0;
/// Colour of a cell no plate reached, which only a starved head start leaves.
const UNCLAIMED_COLOR: Color = Color::srgb(0.08, 0.08, 0.1);
/// How far a plate's colour is dimmed where the boundary layer draws no border.
const INTERIOR_DIMMING: f32 = 0.3;

/// The palette holds one colour per addressable plate id, then one per crust
/// class, then one per boundary class, so every layer is one lookup.
const CRUST_PALETTE_BASE: usize = PLATE_ID_COUNT as usize;
const BOUNDARY_PALETTE_BASE: usize = CRUST_PALETTE_BASE + CrustClass::ALL.len();
const PALETTE_LEN: usize = BOUNDARY_PALETTE_BASE + BoundaryClass::ALL.len();

pub const GRID_QUAD_RANGE: std::ops::RangeInclusive<u32> = 16..=256;
const DEFAULT_GRID_QUADS: u32 = 128;

const FACE_FRAGMENT_SHADER: Handle<Shader> = uuid_handle!("2f36f2f5-0a1e-4a0e-9a54-2a8e0f4d0f11");

/// Which field the face grids colour themselves by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SurfaceLayer {
    #[default]
    Plates,
    Crust,
    Boundaries,
}

impl SurfaceLayer {
    pub const ALL: [Self; 3] = [Self::Plates, Self::Crust, Self::Boundaries];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Plates => "Plates",
            Self::Crust => "Crust",
            Self::Boundaries => "Boundaries",
        }
    }
}

/// How finely the face grids are tessellated, independent of texel resolution,
/// and which field they show.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplaySettings {
    pub grid_quads: u32,
    pub layer: SurfaceLayer,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            grid_quads: DEFAULT_GRID_QUADS,
            layer: SurfaceLayer::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, ShaderType)]
struct PlateFaceDisplay {
    resolution: u32,
    layer: u32,
}

#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
struct PlateFaceMaterial {
    #[storage(0, read_only, buffer)]
    ownership: Buffer,
    #[storage(1, read_only, buffer)]
    boundary_classes: Buffer,
    #[storage(2, read_only, buffer)]
    plates: Buffer,
    #[uniform(3)]
    display: PlateFaceDisplay,
    #[storage(4, read_only)]
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
                    sync_material.run_if(
                        resource_exists_and_changed::<ResidentTectonics>
                            .or(resource_exists::<ResidentTectonics>
                                .and(resource_changed::<DisplaySettings>)),
                    ),
                    sync_faces.run_if(faces_are_stale),
                )
                    .chain(),
            );
    }
}

fn register_shader(app: &mut App) {
    let constants = format!(
        "const RASTER_LAYER_CRUST: u32 = {}u;\n\
         const RASTER_LAYER_BOUNDARIES: u32 = {}u;\n\
         const RASTER_CRUST_PALETTE_BASE: u32 = {CRUST_PALETTE_BASE}u;\n\
         const RASTER_BOUNDARY_PALETTE_BASE: u32 = {BOUNDARY_PALETTE_BASE}u;\n\
         const RASTER_INTERIOR_DIMMING: f32 = {INTERIOR_DIMMING:?};",
        layer_code(SurfaceLayer::Crust),
        layer_code(SurfaceLayer::Boundaries),
    );
    let source = [
        MAPPING_WGSL_SOURCE,
        RASTER_WGSL_SOURCE,
        &field_wgsl_source(),
        &constants,
        include_str!("faces.wgsl"),
    ]
    .join("\n");
    app.world_mut()
        .resource_mut::<Assets<Shader>>()
        .insert(
            FACE_FRAGMENT_SHADER.id(),
            Shader::from_wgsl(source, "procgen-raster-faces.wgsl"),
        )
        .expect("the face shader handle must be free");
}

/// Colour of each crust class, in [`CrustClass::ALL`] order.
const CRUST_COLORS: [Color; CrustClass::ALL.len()] =
    [Color::srgb(0.10, 0.24, 0.45), Color::srgb(0.52, 0.46, 0.32)];

/// Colour of each boundary class, in [`BoundaryClass::ALL`] order. The interior
/// entry is never sampled; the layer dims the plate colour there instead.
const BOUNDARY_COLORS: [Color; BoundaryClass::ALL.len()] = [
    UNCLAIMED_COLOR,
    Color::srgb(0.85, 0.24, 0.18),
    Color::srgb(0.20, 0.55, 0.85),
    Color::srgb(0.92, 0.78, 0.22),
];

const fn layer_code(layer: SurfaceLayer) -> u32 {
    layer as u32
}

fn initialize_palette(mut commands: Commands, mut buffers: ResMut<Assets<ShaderStorageBuffer>>) {
    let plates = (0..PLATE_ID_COUNT as usize).map(|plate| {
        if plate == UNCLAIMED_PLATE as usize {
            UNCLAIMED_COLOR
        } else {
            id_color(plate)
        }
    });
    let crust = CrustClass::ALL.map(|class| CRUST_COLORS[crust_class_code(class) as usize]);
    let boundaries =
        BoundaryClass::ALL.map(|class| BOUNDARY_COLORS[boundary_class_code(class) as usize]);
    let palette: Vec<[f32; 4]> = plates
        .chain(crust)
        .chain(boundaries)
        .map(|color| color.to_linear().to_f32_array())
        .collect();
    debug_assert_eq!(palette.len(), PALETTE_LEN);
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
    tectonics: Res<ResidentTectonics>,
    display: Res<DisplaySettings>,
    palette: Res<PlatePalette>,
    mut materials: ResMut<Assets<PlateFaceMaterial>>,
    existing: Option<Res<FaceMaterial>>,
) {
    let pipeline = tectonics.pipeline();
    let material = PlateFaceMaterial {
        ownership: Buffer::from(pipeline.ownership_buffer().clone()),
        boundary_classes: Buffer::from(pipeline.boundary_class_buffer().clone()),
        plates: Buffer::from(pipeline.plate_buffer().clone()),
        display: PlateFaceDisplay {
            resolution: pipeline.resolution(),
            layer: layer_code(display.layer),
        },
        palette: palette.0.clone(),
    };
    match existing {
        Some(existing) => {
            *materials
                .get_mut(&existing.0)
                .expect("the face material outlives the resident pipeline") = material;
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
