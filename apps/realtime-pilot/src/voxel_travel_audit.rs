//! Headless consumer of the same residency owner intended for the physical viewer.
use crate::{
    PlanetDesignField, VoxelChunkAddress, VoxelPosition, VoxelResidency, VoxelResidencyConfig,
    VoxelResidencyError, VoxelResidencyStats, sample_voxel_chunk,
};
use procgen_core::Vec3;
use serde::Serialize;
use std::{
    error::Error,
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub enum VoxelTravelError {
    Residency(VoxelResidencyError),
    Timeout,
    CanonicalDensity,
    Revisit,
}
impl From<VoxelResidencyError> for VoxelTravelError {
    fn from(value: VoxelResidencyError) -> Self {
        Self::Residency(value)
    }
}
impl fmt::Display for VoxelTravelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Residency(e) => e.fmt(f),
            Self::Timeout => f.write_str("voxel travel audit did not settle within five minutes"),
            Self::CanonicalDensity => {
                f.write_str("resident density differs from the canonical CPU chunk")
            }
            Self::Revisit => f.write_str("voxel leaf addresses changed on revisit"),
        }
    }
}
impl Error for VoxelTravelError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Residency(e) => Some(e),
            _ => None,
        }
    }
}
#[derive(Serialize)]
pub struct VoxelTravelStop {
    pub camera_m: VoxelPosition,
    pub finest_spacing_m: Option<i32>,
    pub coarsest_spacing_m: Option<i32>,
    pub leaf_fingerprint: u64,
    pub stats: VoxelResidencyStats,
}
#[derive(Serialize)]
pub struct VoxelTravelAudit {
    pub config: VoxelResidencyConfig,
    pub density_budget_bytes: usize,
    pub stops: Vec<VoxelTravelStop>,
    pub canonical_samples_checked: usize,
    pub max_canonical_difference_m: f32,
    pub elapsed_seconds: f32,
    pub max_update_ms: f32,
}
fn address_fingerprint(leaves: &[VoxelChunkAddress]) -> u64 {
    procgen_core::fingerprint(leaves.iter().flat_map(|a| {
        let index = a.index();
        [
            a.lod() as u64,
            index.x as u64,
            index.y as u64,
            index.z as u64,
        ]
    }))
}
fn surface_probe(field: &PlanetDesignField, direction: Vec3, altitude_m: f32) -> VoxelPosition {
    let p = direction * (field.config().radius_m + field.height(direction, 0.0) + altitude_m);
    VoxelPosition {
        x_m: p.x.round() as i32,
        y_m: p.y.round() as i32,
        z_m: p.z.round() as i32,
    }
}

/// Orbit -> surface -> rapid travel -> opposite hemisphere -> revisit -> orbit.
/// Checks every settled volume against a fresh canonical sample, one extra
/// volume at a time. That audit scratch is separate from the residency budget.
pub fn audit_voxel_travel(
    field: Arc<PlanetDesignField>,
    config: VoxelResidencyConfig,
) -> Result<VoxelTravelAudit, VoxelTravelError> {
    let start = Instant::now();
    let mut world = VoxelResidency::new(Arc::clone(&field), config)?;
    let ground = surface_probe(&field, Vec3::X, 2.0);
    let orbit = surface_probe(&field, Vec3::X, 200_000.0);
    let far = surface_probe(&field, -Vec3::X, 2.0);
    let mut stops = Vec::new();
    let mut original_leaves = Vec::new();
    let mut canonical_samples_checked = 0;
    let mut max_canonical_difference_m = 0.0_f32;
    let mut max_update_ms = 0.0_f32;
    for (index, camera) in [orbit, ground, far, ground, orbit].into_iter().enumerate() {
        if index == 2 {
            // Move again while each new location still has pending work.
            // Use integer travel positions: no time-step or completion-order input.
            for step in 1..=8 {
                let update_start = Instant::now();
                world.update(VoxelPosition {
                    y_m: ground.y_m + step * 16_384,
                    ..ground
                })?;
                max_update_ms = max_update_ms.max(update_start.elapsed().as_secs_f32() * 1000.0);
            }
        }
        loop {
            let update_start = Instant::now();
            world.update(camera)?;
            max_update_ms = max_update_ms.max(update_start.elapsed().as_secs_f32() * 1000.0);
            if world.settled() {
                break;
            }
            if start.elapsed() > Duration::from_secs(300) {
                return Err(VoxelTravelError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        if index == 1 {
            original_leaves = world.leaves().to_vec();
        }
        if index == 3 && world.leaves() != original_leaves {
            return Err(VoxelTravelError::Revisit);
        }
        for volume in world.residents() {
            let reference =
                sample_voxel_chunk(&field, volume.address()).map_err(VoxelResidencyError::from)?;
            for ((_, actual), (_, expected)) in volume.samples().zip(reference.samples()) {
                max_canonical_difference_m =
                    max_canonical_difference_m.max((actual - expected).abs());
                canonical_samples_checked += 1;
            }
        }
        stops.push(VoxelTravelStop {
            camera_m: camera,
            finest_spacing_m: world.residents().map(|v| v.address().spacing_m()).min(),
            coarsest_spacing_m: world.leaves().iter().map(|a| a.spacing_m()).max(),
            leaf_fingerprint: address_fingerprint(world.leaves()),
            stats: world.stats(),
        });
    }
    if max_canonical_difference_m != 0.0 {
        return Err(VoxelTravelError::CanonicalDensity);
    }
    Ok(VoxelTravelAudit {
        config,
        density_budget_bytes: config.density_budget_bytes(),
        stops,
        canonical_samples_checked,
        max_canonical_difference_m,
        elapsed_seconds: start.elapsed().as_secs_f32(),
        max_update_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlanetDesignConfig;

    #[test]
    fn travel_audit_checks_regenerated_chunks_and_returns_to_zero_density() {
        let mut design = PlanetDesignConfig::starter(0);
        design.radius_m = 100_000.0;
        design.octaves.truncate(1);
        let report = audit_voxel_travel(
            Arc::new(design.validate().unwrap()),
            VoxelResidencyConfig {
                density_reach_m: 32.0,
                ..VoxelResidencyConfig::default()
            },
        )
        .unwrap();
        assert!(report.canonical_samples_checked > 0);
        assert_eq!(report.max_canonical_difference_m, 0.0);
        assert_eq!(report.stops[1].finest_spacing_m, Some(1));
        assert_eq!(
            report.stops[1].leaf_fingerprint,
            report.stops[3].leaf_fingerprint
        );
        let end = report.stops.last().unwrap().stats;
        assert_eq!(end.density_bytes, 0);
        assert!(end.evictions > 0);
        assert!(end.peak_density_bytes <= report.density_budget_bytes);
    }
}
