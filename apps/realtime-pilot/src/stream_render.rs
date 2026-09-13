use bevy::render::renderer::RenderQueue;
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::{Render, RenderApp, RenderSystems, mesh::RenderMesh, render_asset::RenderAssets},
};
use procgen_realtime_pilot::{DetailLevel, Ticket, UploadPiece};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub struct Readiness {
    pub pending: Vec<(Ticket, usize, AssetId<Mesh>)>,
    pub ready: Vec<(Ticket, usize)>,
    pub retiring: Vec<(Ticket, Vec<AssetId<Mesh>>)>,
    pub retired: Vec<Ticket>,
}
#[derive(Resource, Clone, Default)]
pub struct UploadBridge(pub Arc<Mutex<Readiness>>);
impl Plugin for UploadBridge {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.clone());
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render
                .insert_resource(self.clone())
                .add_systems(Render, poll.after(RenderSystems::PrepareAssets));
        }
    }
}
fn poll(bridge: Res<UploadBridge>, meshes: Res<RenderAssets<RenderMesh>>, queue: Res<RenderQueue>) {
    let shared = Arc::clone(&bridge.0);
    let mut completed = Vec::new();
    let mut bridge = bridge.0.lock().expect("upload bridge");
    let pending = std::mem::take(&mut bridge.pending);
    for (ticket, piece, id) in pending {
        if meshes.get(id).is_some() {
            bridge.ready.push((ticket, piece));
        } else {
            bridge.pending.push((ticket, piece, id));
        }
    }
    let retiring = std::mem::take(&mut bridge.retiring);
    for (ticket, ids) in retiring {
        if ids.iter().all(|&id| meshes.get(id).is_none()) {
            completed.push(ticket);
        } else {
            bridge.retiring.push((ticket, ids));
        }
    }
    drop(bridge);
    for ticket in completed {
        let shared = Arc::clone(&shared);
        queue.on_submitted_work_done(move || {
            shared.lock().expect("upload bridge").retired.push(ticket)
        });
    }
}

pub fn chunk_mesh(piece: &UploadPiece) -> Mesh {
    let surface = piece.mesh.surface();
    let mut ids = BTreeMap::new();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    let color = match piece.ticket.detail {
        DetailLevel::Coarse => [0.16, 0.32, 0.6, 1.0],
        DetailLevel::Medium => [0.2, 0.55, 0.32, 1.0],
        DetailLevel::Fine => [0.62, 0.44, 0.15, 1.0],
    };
    for triangle in &surface.triangles()[piece.triangles.clone()] {
        for id in triangle.vertices {
            let index = *ids.entry(id).or_insert_with(|| {
                let p = surface.positions()[id as usize];
                let n = piece.mesh.normals()[id as usize];
                let index = positions.len() as u32;
                positions.push([p.x, p.y, p.z]);
                normals.push([n.x, n.y, n.z]);
                colors.push(color);
                index
            });
            indices.push(index);
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}
