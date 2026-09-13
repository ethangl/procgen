//! Host camera coordinates and a meter-scale kinematic sphere controller.
use crate::{
    ContactError, MeterPosition, VOXEL_CONTACT_SKIN_M, VoxelCollision, VoxelCollisionError,
};
use procgen_core::Vec3;

pub const PLAYER_RADIUS_M: f32 = 0.4;
pub const PLAYER_EYE_M: f32 = 1.7;
pub const PLAYER_SPEED_MPS: f32 = 4.0;
const WALKABLE_COSINE: f32 = 0.65;

#[derive(Clone)]
pub struct PhysicalWalker {
    center: MeterPosition,
    velocity: Vec3,
    grounded: bool,
}
impl PhysicalWalker {
    pub fn land(
        collision: &VoxelCollision,
        probe: MeterPosition,
    ) -> Result<Self, VoxelCollisionError> {
        let up = probe.direction();
        let p = probe.relative_to(collision.origin_m());
        let hit = collision
            .ray(p + up * 20.0, p - up * 20.0)?
            .ok_or(ContactError::NoLanding)?;
        if hit.normal.dot(up) < WALKABLE_COSINE {
            return Err(ContactError::NoLanding.into());
        }
        let center = hit.position_m + hit.normal * (PLAYER_RADIUS_M + 0.01);
        collision.sweep(center, Vec3::ZERO, PLAYER_RADIUS_M)?;
        Ok(Self {
            center: MeterPosition::new(collision.origin_m(), center),
            velocity: Vec3::ZERO,
            grounded: true,
        })
    }
    pub fn center(&self) -> MeterPosition {
        self.center
    }
    pub fn eye(&self) -> MeterPosition {
        self.center
            .translated(self.center.direction() * (PLAYER_EYE_M - PLAYER_RADIUS_M))
    }
    pub fn grounded(&self) -> bool {
        self.grounded
    }
    /// Missing support leaves the complete previous movement state unchanged.
    pub fn advance(
        &mut self,
        collision: &VoxelCollision,
        input: Vec3,
        seconds: f32,
    ) -> Result<(), VoxelCollisionError> {
        if !input.is_finite() || !seconds.is_finite() || !(0.0..=0.05).contains(&seconds) {
            return Err(ContactError::InvalidMotion.into());
        }
        let mut next = self.clone();
        let steps = (seconds / (1.0 / 120.0)).ceil() as usize;
        if steps == 0 {
            return Ok(());
        }
        for _ in 0..steps {
            next.step(collision, input, seconds / steps as f32)?;
        }
        *self = next;
        Ok(())
    }
    fn step(
        &mut self,
        collision: &VoxelCollision,
        input: Vec3,
        dt: f32,
    ) -> Result<(), VoxelCollisionError> {
        let up = self.center.direction();
        let tangent = input - up * input.dot(up);
        let tangent = if tangent.length_squared() > 1e-12 {
            tangent.normalized() * input.length().min(1.0)
        } else {
            Vec3::ZERO
        };
        if self.grounded {
            self.velocity = Vec3::ZERO;
        } else {
            self.velocity = self.velocity - up * (9.81 * dt);
        }
        let mut motion = (tangent * PLAYER_SPEED_MPS + self.velocity) * dt;
        let mut p = self.center.relative_to(collision.origin_m());
        self.grounded = false;
        for _ in 0..4 {
            let sweep = collision.sweep(p, motion, PLAYER_RADIUS_M)?;
            motion = motion - (sweep.position_m - p);
            p = sweep.position_m;
            let Some(n) = sweep.normal else {
                break;
            };
            self.grounded |= n.dot(up) >= WALKABLE_COSINE;
            self.velocity = self.velocity - n * self.velocity.dot(n).min(0.0);
            motion = motion - n * motion.dot(n).min(0.0);
            if motion.length_squared() < 1e-12 {
                break;
            }
            let release = p + n * (VOXEL_CONTACT_SKIN_M * 4.0);
            collision.sweep(release, Vec3::ZERO, PLAYER_RADIUS_M)?;
            p = release;
        }
        let support = collision.sweep(p, -up * 0.08, PLAYER_RADIUS_M)?;
        if support.normal.is_some_and(|n| n.dot(up) >= WALKABLE_COSINE) {
            p = support.position_m;
            self.grounded = true;
            self.velocity = Vec3::ZERO;
        }
        self.center = MeterPosition::new(collision.origin_m(), p);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PlanetDesignConfig, VoxelPosition};
    use std::sync::atomic::AtomicBool;
    #[test]
    fn walker_crosses_triangle_and_chunk_boundaries_and_stops_without_support() {
        let mut config = PlanetDesignConfig::starter(42);
        config.radius_m = 8_000_000.0;
        config.octaves.iter_mut().for_each(|o| o.enabled = false);
        let field = config.validate().unwrap();
        let origin = VoxelPosition {
            x_m: 8_000_002,
            y_m: 0,
            z_m: 0,
        };
        let patch = VoxelCollision::build(&field, origin, &AtomicBool::new(false)).unwrap();
        let mut walker =
            PhysicalWalker::land(&patch, MeterPosition::new(origin, Vec3::ZERO)).unwrap();
        // Looking down must not reduce the configured horizontal walking speed.
        for _ in 0..500 {
            walker
                .advance(&patch, Vec3::new(-0.6, 0.8, 0.0), 0.02)
                .unwrap();
        }
        assert!((walker.center().relative_to(origin).y - 40.0).abs() < 0.02);
        assert!(walker.grounded());
        assert!((walker.eye().altitude_m(config.radius_m) - PLAYER_EYE_M as f64).abs() < 0.02);
        let before = walker.center().relative_to(origin);
        for _ in 0..200 {
            let previous = walker.center().relative_to(origin);
            if walker.advance(&patch, Vec3::Y, 0.05).is_err() {
                assert_eq!(walker.center().relative_to(origin), previous);
                assert!(previous.y > before.y);
                return;
            }
        }
        panic!("walking must stop at the patch boundary");
    }
}
