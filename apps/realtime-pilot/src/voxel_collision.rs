//! Meter-scale, two-sided collision from canonical one-meter source chunks.
use crate::triangle_query::{closest, inside};
use crate::{
    ChunkIndex, ContactError, PlanetDesignField, VoxelAddressError, VoxelChunkAddress,
    VoxelPosition, VoxelSurface, VoxelSurfaceError, build_voxel_surface, sample_voxel_chunk,
};
use procgen_core::Vec3;
use std::{error::Error, fmt, sync::atomic::AtomicBool};

pub const VOXEL_CONTACT_SKIN_M: f32 = 0.001;
#[derive(Debug)]
pub enum VoxelCollisionError {
    Address(VoxelAddressError),
    Surface(VoxelSurfaceError),
    Contact(ContactError),
    Overlap,
}
impl From<VoxelAddressError> for VoxelCollisionError {
    fn from(e: VoxelAddressError) -> Self {
        Self::Address(e)
    }
}
impl From<VoxelSurfaceError> for VoxelCollisionError {
    fn from(e: VoxelSurfaceError) -> Self {
        Self::Surface(e)
    }
}
impl From<ContactError> for VoxelCollisionError {
    fn from(e: ContactError) -> Self {
        Self::Contact(e)
    }
}
impl fmt::Display for VoxelCollisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address(e) => e.fmt(f),
            Self::Surface(e) => e.fmt(f),
            Self::Contact(e) => e.fmt(f),
            Self::Overlap => f.write_str("sphere intersects the collision surface"),
        }
    }
}
impl Error for VoxelCollisionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(match self {
            Self::Address(e) => e,
            Self::Surface(e) => e,
            Self::Contact(e) => e,
            Self::Overlap => return None,
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct VoxelContact {
    pub position_m: Vec3,
    pub normal: Vec3,
}
#[derive(Clone, Copy, Debug)]
pub struct VoxelSweep {
    pub position_m: Vec3,
    pub normal: Option<Vec3>,
}

/// Coordinates in all queries are relative to origin_m. The complete 3x3x3
/// support box stays owned here, independent of render residency or replacement.
pub struct VoxelCollision {
    surface: VoxelSurface,
    min: Vec3,
    max: Vec3,
}
impl VoxelCollision {
    pub fn build(
        field: &PlanetDesignField,
        origin_m: VoxelPosition,
        cancel: &AtomicBool,
    ) -> Result<Self, VoxelCollisionError> {
        let center = VoxelChunkAddress::containing(origin_m, 0)?;
        let index = center.index();
        let mut volumes = Vec::new();
        for z in -1..=1 {
            for y in -1..=1 {
                for x in -1..=1 {
                    crate::voxel_surface_grid::check_cancel(cancel)?;
                    let coordinate =
                        |v: u32, d: i32| v.checked_add_signed(d).ok_or(VoxelAddressError::Index);
                    let a = VoxelChunkAddress::new(
                        0,
                        ChunkIndex {
                            x: coordinate(index.x, x)?,
                            y: coordinate(index.y, y)?,
                            z: coordinate(index.z, z)?,
                        },
                    )?;
                    // All 27 source volumes remain live through extraction, then drop.
                    volumes.push(sample_voxel_chunk(field, a).map_err(VoxelSurfaceError::from)?);
                }
            }
        }
        let surface = build_voxel_surface(&volumes.iter().collect::<Vec<_>>(), origin_m, cancel)?;
        let start = center.origin().relative_to(origin_m).as_vec3();
        Ok(Self {
            surface,
            min: start - Vec3::new(1.0, 1.0, 1.0) * crate::VOXEL_CHUNK_CELLS as f32,
            max: start + Vec3::new(2.0, 2.0, 2.0) * crate::VOXEL_CHUNK_CELLS as f32,
        })
    }
    pub fn origin_m(&self) -> VoxelPosition {
        self.surface.origin_m()
    }
    pub fn surface(&self) -> &VoxelSurface {
        &self.surface
    }
    fn triangle(&self, id: usize) -> [Vec3; 3] {
        self.surface.triangles()[id]
            .vertices
            .map(|i| self.surface.positions()[i as usize])
    }
    fn covers(&self, p: Vec3, r: f32) -> bool {
        [p.x - r, p.y - r, p.z - r]
            .into_iter()
            .zip([self.min.x, self.min.y, self.min.z])
            .all(|(p, m)| p > m)
            && [p.x + r, p.y + r, p.z + r]
                .into_iter()
                .zip([self.max.x, self.max.y, self.max.z])
                .all(|(p, m)| p < m)
    }
    /// A miss is distinct from missing source coverage.
    pub fn ray(&self, start: Vec3, end: Vec3) -> Result<Option<VoxelContact>, VoxelCollisionError> {
        if !start.is_finite() || !end.is_finite() {
            return Err(ContactError::InvalidMotion.into());
        }
        if !self.covers(start, 0.0) || !self.covers(end, 0.0) {
            return Err(ContactError::MissingCoverage.into());
        }
        Ok(surface_ray(&self.surface, start, end))
    }

    /// Conservative advancement uses triangle distance, never density as an SDF.
    /// Missing coverage or exhausted iteration budget fails before the caller
    /// applies movement. A contact returns the last safe sphere center.
    pub fn sweep(
        &self,
        start: Vec3,
        delta: Vec3,
        radius_m: f32,
    ) -> Result<VoxelSweep, VoxelCollisionError> {
        if !start.is_finite() || !delta.is_finite() || !radius_m.is_finite() || radius_m <= 0.0 {
            return Err(ContactError::InvalidMotion.into());
        }
        let end = start + delta;
        if !self.covers(start, radius_m) || !self.covers(end, radius_m) {
            return Err(ContactError::MissingCoverage.into());
        }
        let lower = Vec3::new(
            start.x.min(end.x) - radius_m,
            start.y.min(end.y) - radius_m,
            start.z.min(end.z) - radius_m,
        );
        let upper = Vec3::new(
            start.x.max(end.x) + radius_m,
            start.y.max(end.y) + radius_m,
            start.z.max(end.z) + radius_m,
        );
        let candidates: Vec<_> = (0..self.surface.triangles().len())
            .filter(|&id| {
                let t = self.triangle(id);
                (0..3).all(|axis| {
                    let value = |p: Vec3| [p.x, p.y, p.z][axis];
                    t.iter().any(|&p| value(p) >= value(lower))
                        && t.iter().any(|&p| value(p) <= value(upper))
                })
            })
            .collect();
        let length = delta.length();
        let mut travelled = 0.0;
        let mut p = start;
        for _ in 0..64 {
            let nearest = candidates
                .iter()
                .map(|&id| {
                    let q = closest(p, self.triangle(id));
                    (p.distance_squared(q), q, id)
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
            let Some((squared, q, _)) = nearest else {
                return Ok(VoxelSweep {
                    position_m: end,
                    normal: None,
                });
            };
            let distance = squared.sqrt();
            if distance < radius_m {
                return Err(VoxelCollisionError::Overlap);
            }
            if distance <= radius_m + VOXEL_CONTACT_SKIN_M {
                let normal = (p - q) * (1.0 / distance);
                return Ok(VoxelSweep {
                    position_m: p,
                    normal: Some(normal),
                });
            }
            let safe = distance - radius_m;
            if travelled + safe >= length {
                return Ok(VoxelSweep {
                    position_m: end,
                    normal: None,
                });
            }
            travelled += safe * 0.9;
            p = start + delta * (travelled / length);
        }
        Err(ContactError::SweepLimit.into())
    }
}

// Shared by the physical query owner and the mixed-LOD agreement audit.
pub(crate) fn surface_ray(surface: &VoxelSurface, start: Vec3, end: Vec3) -> Option<VoxelContact> {
    let delta = end - start;
    let mut best: Option<(f32, VoxelContact)> = None;
    for triangle in surface.triangles() {
        let [a, b, c] = triangle.vertices.map(|i| surface.positions()[i as usize]);
        let n = (b - a).cross(c - a);
        let denominator = n.dot(delta);
        if denominator == 0.0 {
            continue;
        }
        let t = n.dot(a - start) / denominator;
        if !(0.0..=1.0).contains(&t) {
            continue;
        }
        let p = start + delta * t;
        if inside(p, [a, b, c], n) && best.is_none_or(|(old, _)| t < old) {
            best = Some((
                t,
                VoxelContact {
                    position_m: p,
                    normal: n.normalized(),
                },
            ));
        }
    }
    best.map(|(_, hit)| hit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PlanetDesignConfig, VOXEL_CONTACT_SKIN_M};
    #[test]
    fn fine_collision_stops_sweeps_on_both_sides_and_rejects_missing_coverage() {
        let mut design = PlanetDesignConfig::starter(42);
        design.radius_m = 8_000_000.0;
        design.octaves.iter_mut().for_each(|o| o.enabled = false);
        let field = design.validate().unwrap();
        let origin = VoxelPosition {
            x_m: 8_000_000,
            y_m: 0,
            z_m: 0,
        };
        let collision = VoxelCollision::build(&field, origin, &AtomicBool::new(false)).unwrap();
        for sign in [-1.0, 1.0] {
            let hit = collision
                .ray(Vec3::X * 4.0 * sign, -Vec3::X * 4.0 * sign)
                .unwrap()
                .unwrap();
            assert!(hit.position_m.length() < 0.001);
            assert!(hit.normal.x > 0.999);
            let sweep = collision
                .sweep(Vec3::X * 4.0 * sign, -Vec3::X * 8.0 * sign, 0.4)
                .unwrap();
            assert!((sweep.position_m.x.abs() - 0.4).abs() < VOXEL_CONTACT_SKIN_M * 2.0);
            assert!(sweep.normal.unwrap().x * sign > 0.999);
        }
        assert!(matches!(
            collision.sweep(Vec3::ZERO, Vec3::X, 0.4),
            Err(VoxelCollisionError::Overlap)
        ));
        assert!(matches!(
            collision.sweep(Vec3::ZERO, Vec3::X * 100.0, 0.4),
            Err(VoxelCollisionError::Contact(ContactError::MissingCoverage))
        ));
        assert!(
            collision
                .ray(Vec3::new(f32::NAN, 0.0, 0.0), Vec3::ZERO)
                .is_err()
        );
        assert!(
            collision
                .ray(Vec3::X * 2.0, Vec3::X * 4.0)
                .unwrap()
                .is_none()
        );
        assert!(VoxelCollision::build(&field, origin, &AtomicBool::new(true)).is_err());
    }
}
