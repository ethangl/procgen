//! Build the actual resident working set and compare it with fine collision.
use crate::voxel_collision::surface_ray;
use crate::{
    PlanetDesignField, VoxelCollision, VoxelCollisionError, VoxelPosition, VoxelResidency,
    VoxelResidencyConfig, VoxelResidencyError, VoxelSurfaceError, VoxelSurfaceTopology,
    build_voxel_surface,
};
use procgen_core::Vec3;
use serde::Serialize;
use std::{
    error::Error,
    fmt,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

#[derive(Debug)]
pub enum VoxelSurfaceAuditError {
    Residency(VoxelResidencyError),
    Surface(VoxelSurfaceError),
    Collision(VoxelCollisionError),
    Timeout,
    Agreement,
}
impl From<VoxelResidencyError> for VoxelSurfaceAuditError {
    fn from(e: VoxelResidencyError) -> Self {
        Self::Residency(e)
    }
}
impl From<VoxelSurfaceError> for VoxelSurfaceAuditError {
    fn from(e: VoxelSurfaceError) -> Self {
        Self::Surface(e)
    }
}
impl From<VoxelCollisionError> for VoxelSurfaceAuditError {
    fn from(e: VoxelCollisionError) -> Self {
        Self::Collision(e)
    }
}
impl fmt::Display for VoxelSurfaceAuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Residency(e) => e.fmt(f),
            Self::Surface(e) => e.fmt(f),
            Self::Collision(e) => e.fmt(f),
            Self::Timeout => {
                f.write_str("surface audit residency did not settle within five minutes")
            }
            Self::Agreement => {
                f.write_str("surface order, nearby collision, or sweep agreement failed")
            }
        }
    }
}
impl Error for VoxelSurfaceAuditError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Residency(e) => Some(e),
            Self::Surface(e) => Some(e),
            Self::Collision(e) => Some(e),
            _ => None,
        }
    }
}
#[derive(Serialize)]
pub struct VoxelSurfaceAudit {
    pub origin_m: VoxelPosition,
    pub source_chunks: usize,
    pub source_density_bytes: usize,
    pub finest_spacing_m: i32,
    pub coarsest_spacing_m: i32,
    pub vertices: usize,
    pub triangles: usize,
    pub mesh_payload_bytes: usize,
    pub topology: VoxelSurfaceTopology,
    pub order_agreement: bool,
    pub revisit_evictions: usize,
    pub collision_triangles: usize,
    pub collision_mesh_payload_bytes: usize,
    pub collision_difference_m: f32,
    pub sphere_radius_m: f32,
    pub stopped_center_m: [f32; 3],
    pub mesh_seconds: f32,
    pub collision_seconds: f32,
}
pub fn audit_voxel_surfaces(
    field: Arc<PlanetDesignField>,
) -> Result<VoxelSurfaceAudit, VoxelSurfaceAuditError> {
    let origin_m = VoxelPosition {
        x_m: (field.config().radius_m + field.height(Vec3::X, 0.0) + 2.0).round() as i32,
        y_m: 0,
        z_m: 0,
    };
    let mut residency = VoxelResidency::new(Arc::clone(&field), VoxelResidencyConfig::default())?;
    settle(&mut residency, origin_m)?;
    let cancel = AtomicBool::new(false);
    let volumes: Vec<_> = residency.residents().collect();
    let start = Instant::now();
    let surface = build_voxel_surface(&volumes, origin_m, &cancel)?;
    let mesh_seconds = start.elapsed().as_secs_f32();
    let source_chunks = volumes.len();
    let source_density_bytes = residency.stats().density_bytes;
    let finest_spacing_m = volumes
        .iter()
        .map(|v| v.address().spacing_m())
        .min()
        .unwrap();
    let coarsest_spacing_m = volumes
        .iter()
        .map(|v| v.address().spacing_m())
        .max()
        .unwrap();
    drop(volumes);
    settle(
        &mut residency,
        VoxelPosition {
            x_m: origin_m.x_m + 200_000,
            ..origin_m
        },
    )?;
    if residency.stats().resident_chunks != 0 {
        return Err(VoxelSurfaceAuditError::Agreement);
    }
    let revisit_evictions = residency.stats().evictions;
    settle(&mut residency, origin_m)?;
    let mut volumes: Vec<_> = residency.residents().collect();
    volumes.reverse();
    let replay = build_voxel_surface(&volumes, origin_m, &cancel)?;
    let order_agreement =
        surface.positions() == replay.positions() && surface.triangles() == replay.triangles();
    if !order_agreement {
        return Err(VoxelSurfaceAuditError::Agreement);
    }
    drop(replay);
    let start = Instant::now();
    let collision = VoxelCollision::build(&field, origin_m, &cancel)?;
    let collision_seconds = start.elapsed().as_secs_f32();
    let ray_start = Vec3::X * 4.0;
    let ray_end = -Vec3::X * 8.0;
    let render =
        surface_ray(&surface, ray_start, ray_end).ok_or(VoxelSurfaceAuditError::Agreement)?;
    let contact = collision
        .ray(ray_start, ray_end)?
        .ok_or(VoxelSurfaceAuditError::Agreement)?;
    let collision_difference_m = (render.position_m - contact.position_m).length();
    let sphere_radius_m = 0.4;
    let sweep = collision.sweep(Vec3::ZERO, -Vec3::X * 8.0, sphere_radius_m)?;
    if collision_difference_m > 0.001 || sweep.normal.is_none() {
        return Err(VoxelSurfaceAuditError::Agreement);
    }
    Ok(VoxelSurfaceAudit {
        origin_m: collision.origin_m(),
        source_chunks,
        source_density_bytes,
        finest_spacing_m,
        coarsest_spacing_m,
        vertices: surface.positions().len(),
        triangles: surface.triangles().len(),
        mesh_payload_bytes: surface.payload_bytes(),
        topology: surface.topology(),
        order_agreement,
        revisit_evictions,
        collision_triangles: collision.surface().triangles().len(),
        collision_mesh_payload_bytes: collision.surface().payload_bytes(),
        collision_difference_m,
        sphere_radius_m,
        stopped_center_m: [sweep.position_m.x, sweep.position_m.y, sweep.position_m.z],
        mesh_seconds,
        collision_seconds,
    })
}

fn settle(world: &mut VoxelResidency, point: VoxelPosition) -> Result<(), VoxelSurfaceAuditError> {
    let start = Instant::now();
    loop {
        world.update(point)?;
        if world.settled() {
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(300) {
            return Err(VoxelSurfaceAuditError::Timeout);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
