use bevy::camera::visibility::VisibilityRange;
use bevy::render::renderer::RenderQueue;
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::{Render, RenderApp, RenderSystems, mesh::RenderMesh, render_asset::RenderAssets},
};
use procgen_realtime_pilot::{DetailLevel, Ticket, UploadPiece};
use std::sync::{Arc, Mutex};

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
        app.insert_resource(self.clone())
            .add_systems(PostUpdate, refresh_visibility_indices);
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render
                .insert_resource(self.clone())
                .add_systems(Render, poll.after(RenderSystems::PrepareAssets));
        }
    }
}
// Bevy 0.18 rebuilds the shared visibility-range table when any range changes,
// but GPU mesh extraction otherwise refreshes only changed entities. Refresh
// all participating mesh references so unchanged regions cannot retain old indices.
fn refresh_visibility_indices(
    ranges: Query<Ref<VisibilityRange>>,
    mut removed: RemovedComponents<VisibilityRange>,
    mut meshes: Query<&mut Mesh3d, With<VisibilityRange>>,
) {
    let removed = removed.read().count() != 0;
    if removed || ranges.iter().any(|range| range.is_changed()) {
        for mut mesh in &mut meshes {
            mesh.set_changed();
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
    let corners = piece.triangles.len() * 3;
    let mut positions = Vec::with_capacity(corners);
    let mut normals = Vec::with_capacity(corners);
    let mut density_lod = Vec::with_capacity(corners);
    let mut barycentrics = Vec::with_capacity(corners);
    let lod = match piece.ticket.detail {
        DetailLevel::Coarse => 0.0,
        DetailLevel::Medium => 1.0,
        DetailLevel::Fine => 2.0,
    };
    for triangle in &surface.triangles()[piece.triangles.clone()] {
        for (corner, id) in triangle.vertices.into_iter().enumerate() {
            let p = surface.positions()[id as usize];
            let n = piece.mesh.normals()[id as usize];
            let d = piece.mesh.density_normals()[id as usize];
            positions.push([p.x, p.y, p.z]);
            normals.push([n.x, n.y, n.z]);
            density_lod.push([d.x, d.y, d.z, lod]);
            barycentrics.push([[1.0, 0.0], [0.0, 1.0], [0.0, 0.0]][corner]);
        }
    }
    let indices = (0..positions.len() as u32).collect();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, density_lod)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, barycentrics)
    .with_inserted_indices(Indices::U32(indices))
}

// Keep the same dither shader variant warm on overview, resident, and staged meshes.
pub fn full_visibility() -> VisibilityRange {
    VisibilityRange {
        start_margin: -2.0..-1.0,
        end_margin: f32::MAX..f32::MAX,
        use_aabb: false,
    }
}
