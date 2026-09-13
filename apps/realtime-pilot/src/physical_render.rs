//! Renderer-owned packing, bounded upload, and GPU readiness/retirement receipts.
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems, mesh::RenderMesh, render_asset::RenderAssets,
        renderer::RenderQueue,
    },
};
use procgen_realtime_pilot::{DesignPreview, MeterPosition, VoxelPosition, VoxelSurface};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
};

pub const UPLOAD_LIMIT: usize = 512 * 1024;
const PIECE_TRIANGLES: usize = 3584;
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Coloring {
    #[default]
    Neutral,
    Lod,
    Normals,
}
pub struct Piece {
    pub origin: VoxelPosition,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}
impl Piece {
    pub fn bytes(&self) -> usize {
        self.positions.len() * 40 + self.indices.len() * 4
    }
    pub fn mesh(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}
pub struct PackedSurface {
    pub pieces: VecDeque<Piece>,
    pub triangles: usize,
    pub bytes: usize,
}
impl PackedSurface {
    pub fn voxel(surface: &VoxelSurface, coloring: Coloring) -> Self {
        pack(
            surface.origin_m(),
            surface.positions(),
            &surface
                .triangles()
                .iter()
                .map(|t| (t.vertices, t.chunk.lod()))
                .collect::<Vec<_>>(),
            coloring,
        )
    }
    pub fn overview(preview: &DesignPreview) -> Self {
        pack(
            VoxelPosition {
                x_m: 0,
                y_m: 0,
                z_m: 0,
            },
            &preview
                .positions_m()
                .iter()
                .map(|p| {
                    MeterPosition::new(
                        VoxelPosition {
                            x_m: 0,
                            y_m: 0,
                            z_m: 0,
                        },
                        *p,
                    )
                })
                .collect::<Vec<_>>(),
            &preview
                .triangles()
                .iter()
                .map(|t| (*t, 19))
                .collect::<Vec<_>>(),
            Coloring::Neutral,
        )
    }
}
fn pack(
    origin: VoxelPosition,
    positions: &[MeterPosition],
    triangles: &[([u32; 3], u8)],
    coloring: Coloring,
) -> PackedSurface {
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for &(ids, _) in triangles {
        let origin = positions[ids[0] as usize].anchor();
        let [a, b, c] = ids.map(|i| vector(positions[i as usize].relative_to(origin)));
        let n = (b - a).cross(c - a);
        for i in ids {
            normals[i as usize] += n;
        }
    }
    // Unused source vertices have no incident normal and never enter a piece.
    for n in &mut normals {
        *n = n.normalize_or_zero();
    }
    let pieces: VecDeque<_> = triangles
        .chunks(PIECE_TRIANGLES)
        .map(|triangles| {
            let mut piece = Piece {
                origin,
                positions: Vec::new(),
                normals: Vec::new(),
                colors: Vec::new(),
                indices: Vec::with_capacity(triangles.len() * 3),
            };
            let mut lookup = HashMap::new();
            for &(ids, lod) in triangles {
                for id in ids {
                    let vertex = *lookup.entry((id, lod)).or_insert_with(|| {
                        let index = piece.positions.len() as u32;
                        let p = positions[id as usize].relative_to(origin);
                        let n = normals[id as usize];
                        piece.positions.push([p.x, p.y, p.z]);
                        piece.normals.push(n.to_array());
                        piece.colors.push(match coloring {
                            Coloring::Neutral => [0.52, 0.52, 0.52, 1.0],
                            Coloring::Normals => {
                                [(n.x + 1.0) * 0.5, (n.y + 1.0) * 0.5, (n.z + 1.0) * 0.5, 1.0]
                            }
                            Coloring::Lod => {
                                let color =
                                    Color::hsl((lod as f32 * 47.0) % 360.0, 0.65, 0.5).to_linear();
                                [color.red, color.green, color.blue, 1.0]
                            }
                        });
                        index
                    });
                    piece.indices.push(vertex);
                }
            }
            assert!(piece.bytes() <= UPLOAD_LIMIT);
            piece
        })
        .collect();
    let bytes = pieces.iter().map(Piece::bytes).sum();
    PackedSurface {
        pieces,
        triangles: triangles.len(),
        bytes,
    }
}
pub fn vector(p: procgen_core::Vec3) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}
pub fn core(p: Vec3) -> procgen_core::Vec3 {
    procgen_core::Vec3::new(p.x, p.y, p.z)
}

#[derive(Default)]
pub struct Receipts {
    pub pending: Vec<AssetId<Mesh>>,
    pub ready: HashSet<AssetId<Mesh>>,
    pub retiring: Option<Vec<AssetId<Mesh>>>,
    pub retired: bool,
}
#[derive(Resource, Clone, Default)]
pub struct PhysicalUploads(pub Arc<Mutex<Receipts>>);
impl Plugin for PhysicalUploads {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.clone());
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render
                .insert_resource(self.clone())
                .add_systems(Render, poll.after(RenderSystems::PrepareAssets));
        }
    }
}
fn poll(
    bridge: Res<PhysicalUploads>,
    meshes: Res<RenderAssets<RenderMesh>>,
    queue: Res<RenderQueue>,
) {
    let mut receipts = bridge.0.lock().unwrap();
    for id in std::mem::take(&mut receipts.pending) {
        if meshes.get(id).is_some() {
            receipts.ready.insert(id);
        } else {
            receipts.pending.push(id);
        }
    }
    if receipts
        .retiring
        .as_ref()
        .is_some_and(|ids| ids.iter().all(|id| meshes.get(*id).is_none()))
    {
        receipts.retiring = None;
        drop(receipts);
        let shared = Arc::clone(&bridge.0);
        queue.on_submitted_work_done(move || shared.lock().unwrap().retired = true);
    }
}
