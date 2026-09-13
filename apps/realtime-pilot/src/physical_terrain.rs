//! Complete planetary replacements assembled off the presentation thread.
use crate::{
    PlanetDesignField, VOXEL_DENSITY_BYTES, VOXEL_WORLD_HALF_EXTENT_M, VoxelPosition,
    VoxelResidency, VoxelResidencyConfig, VoxelResidencyError, VoxelSurface, VoxelSurfaceError,
};
use std::{
    error::Error,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub struct PhysicalTerrain {
    residency: VoxelResidency,
}
pub struct PhysicalTerrainFrame {
    pub surface: VoxelSurface,
    pub source_bytes: usize,
    pub generation_seconds: f32,
}
#[derive(Debug)]
pub enum PhysicalTerrainError {
    Residency(VoxelResidencyError),
    Surface(VoxelSurfaceError),
}
impl From<VoxelResidencyError> for PhysicalTerrainError {
    fn from(e: VoxelResidencyError) -> Self {
        Self::Residency(e)
    }
}
impl From<VoxelSurfaceError> for PhysicalTerrainError {
    fn from(e: VoxelSurfaceError) -> Self {
        Self::Surface(e)
    }
}
impl fmt::Display for PhysicalTerrainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Residency(e) => e.fmt(f),
            Self::Surface(e) => e.fmt(f),
        }
    }
}
impl Error for PhysicalTerrainError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(match self {
            Self::Residency(e) => e,
            Self::Surface(e) => e,
        })
    }
}
impl PhysicalTerrain {
    pub fn new(field: Arc<PlanetDesignField>) -> Result<Self, PhysicalTerrainError> {
        Ok(Self {
            residency: VoxelResidency::new(field, VoxelResidencyConfig::default())?,
        })
    }
    /// Blocking CPU operation; the viewer runs one at a time on its terrain worker.
    pub fn build(
        &mut self,
        camera: VoxelPosition,
        cancel: &AtomicBool,
    ) -> Result<PhysicalTerrainFrame, PhysicalTerrainError> {
        let started = Instant::now();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(VoxelSurfaceError::Cancelled.into());
            }
            self.residency.update(camera)?;
            if self.residency.settled() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let inside = [camera.x_m, camera.y_m, camera.z_m]
            .into_iter()
            .all(|v| (-VOXEL_WORLD_HALF_EXTENT_M..VOXEL_WORLD_HALF_EXTENT_M).contains(&v));
        let origin = if inside && self.residency.stats().resident_chunks != 0 {
            camera
        } else {
            VoxelPosition {
                x_m: 0,
                y_m: 0,
                z_m: 0,
            }
        };
        let surface = self.residency.build_complete_surface(origin, cancel)?;
        Ok(PhysicalTerrainFrame {
            surface,
            source_bytes: self.residency.leaves().len() * VOXEL_DENSITY_BYTES,
            generation_seconds: started.elapsed().as_secs_f32(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlanetDesignConfig;
    #[test]
    fn complete_replacement_covers_distant_and_near_leaves_without_open_edges() {
        let mut config = PlanetDesignConfig::starter(42);
        config.radius_m = 100_000.0;
        config.octaves.iter_mut().for_each(|o| o.enabled = false);
        let mut terrain = PhysicalTerrain::new(Arc::new(config.validate().unwrap())).unwrap();
        let frame = terrain
            .build(
                VoxelPosition {
                    x_m: 100_002,
                    y_m: 0,
                    z_m: 0,
                },
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(frame.surface.topology().boundary_edges, 0);
        assert!(frame.surface.triangles().iter().any(|t| t.chunk.lod() == 0));
        assert!(frame.source_bytes <= VoxelResidencyConfig::default().density_budget_bytes());
        assert!(
            frame
                .surface
                .positions()
                .iter()
                .any(|p| p.relative_to(frame.surface.origin_m()).x < -199_000.0)
        );
    }
}
