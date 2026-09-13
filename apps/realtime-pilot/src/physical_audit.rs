//! Repeatable planet-to-ground and collision-handoff checks without a renderer.
use crate::{
    MeterPosition, PLAYER_SPEED_MPS, PhysicalTerrain, PhysicalTerrainError, PhysicalWalker,
    PlanetDesignField, VoxelCollision, VoxelCollisionError, VoxelPosition,
};
use procgen_core::Vec3;
use serde::Serialize;
use std::{
    error::Error,
    fmt,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
#[derive(Debug)]
pub enum PhysicalAuditError {
    Terrain(PhysicalTerrainError),
    Collision(VoxelCollisionError),
    OpenSurface,
    Walking,
}
impl From<PhysicalTerrainError> for PhysicalAuditError {
    fn from(e: PhysicalTerrainError) -> Self {
        Self::Terrain(e)
    }
}
impl From<VoxelCollisionError> for PhysicalAuditError {
    fn from(e: VoxelCollisionError) -> Self {
        Self::Collision(e)
    }
}
impl fmt::Display for PhysicalAuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Terrain(e) => e.fmt(f),
            Self::Collision(e) => e.fmt(f),
            Self::OpenSurface => f.write_str("complete terrain replacement has open edges"),
            Self::Walking => {
                f.write_str("walking or collision replacement failed the motion audit")
            }
        }
    }
}
impl Error for PhysicalAuditError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Terrain(e) => Some(e),
            Self::Collision(e) => Some(e),
            _ => None,
        }
    }
}
#[derive(Serialize)]
pub struct PhysicalStop {
    pub camera_m: VoxelPosition,
    pub triangles: usize,
    pub source_bytes: usize,
    pub mesh_bytes: usize,
    pub generation_seconds: f32,
    pub finest_spacing_m: i32,
}
#[derive(Serialize)]
pub struct PhysicalAudit {
    pub stops: Vec<PhysicalStop>,
    pub collision_bytes: usize,
    pub collision_seconds: f32,
    pub walking_seconds: f32,
    pub walk_update_max_ms: f32,
    pub handoff_difference_m: f32,
    pub idle_drift_m: f32,
}

pub fn audit_physical_exploration(
    field: Arc<PlanetDesignField>,
) -> Result<PhysicalAudit, PhysicalAuditError> {
    let cancel = AtomicBool::new(false);
    let ground = VoxelPosition {
        x_m: (field.config().radius_m + field.height(Vec3::X, 0.0)).round() as i32 + 2,
        y_m: 0,
        z_m: 0,
    };
    let orbit = VoxelPosition {
        x_m: (field.config().radius_m * 3.0).round() as i32,
        y_m: 0,
        z_m: 0,
    };
    let mut terrain = PhysicalTerrain::new(Arc::clone(&field))?;
    let mut stops = Vec::new();
    for camera in [orbit, ground] {
        let frame = terrain.build(camera, &cancel)?;
        if frame.surface.topology().boundary_edges != 0 {
            return Err(PhysicalAuditError::OpenSurface);
        }
        stops.push(PhysicalStop {
            camera_m: camera,
            triangles: frame.surface.triangles().len(),
            source_bytes: frame.source_bytes,
            mesh_bytes: frame.surface.payload_bytes(),
            generation_seconds: frame.generation_seconds,
            finest_spacing_m: frame
                .surface
                .triangles()
                .iter()
                .map(|t| t.chunk.spacing_m())
                .min()
                .expect("planet surface"),
        });
    }
    // Visual source and geometry can be released while walking remains valid.
    drop(terrain);
    let start = Instant::now();
    let collision = VoxelCollision::build(&field, ground, &cancel)?;
    let collision_seconds = start.elapsed().as_secs_f32();
    let collision_bytes = collision.payload_bytes();
    let mut walker = PhysicalWalker::land(&collision, MeterPosition::new(ground, Vec3::ZERO))?;
    let mut walk_update_max_ms = 0.0_f32;
    let step = |walker: &mut PhysicalWalker,
                patch: &VoxelCollision,
                input: Vec3,
                peak: &mut f32|
     -> Result<(), PhysicalAuditError> {
        let start = Instant::now();
        walker.advance(patch, input, 0.02)?;
        *peak = peak.max(start.elapsed().as_secs_f32() * 1000.0);
        Ok(())
    };
    let walking_seconds = 4.0;
    let before = walker.center().relative_to(ground);
    for _ in 0..200 {
        step(&mut walker, &collision, Vec3::Y, &mut walk_update_max_ms)?;
    }
    if walker.center().relative_to(ground).y - before.y < walking_seconds * PLAYER_SPEED_MPS * 0.8 {
        return Err(PhysicalAuditError::Walking);
    }
    let replacement = VoxelCollision::build(&field, walker.center().anchor(), &cancel)?;
    let mut retained = walker.clone();
    for _ in 0..50 {
        step(&mut walker, &replacement, Vec3::Y, &mut walk_update_max_ms)?;
        step(&mut retained, &collision, Vec3::Y, &mut walk_update_max_ms)?;
    }
    let handoff_difference_m =
        (walker.center().relative_to(ground) - retained.center().relative_to(ground)).length();
    // Five contact skins allow local-origin rounding and contact-release differences.
    if handoff_difference_m > 0.005 {
        return Err(PhysicalAuditError::Walking);
    }
    for _ in 0..10 {
        step(
            &mut walker,
            &replacement,
            Vec3::ZERO,
            &mut walk_update_max_ms,
        )?;
    }
    let before = walker.center().relative_to(ground);
    for _ in 0..100 {
        step(
            &mut walker,
            &replacement,
            Vec3::ZERO,
            &mut walk_update_max_ms,
        )?;
    }
    let idle_drift_m = (walker.center().relative_to(ground) - before).length();
    if idle_drift_m > 0.005 || !walker.grounded() {
        return Err(PhysicalAuditError::Walking);
    }
    Ok(PhysicalAudit {
        stops,
        collision_bytes,
        collision_seconds,
        walking_seconds,
        walk_update_max_ms,
        handoff_difference_m,
        idle_drift_m,
    })
}
