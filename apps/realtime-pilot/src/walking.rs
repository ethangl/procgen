//! Fixed fine-surface, radial-gravity kinematic sphere controller.
use crate::{CONTACT_SKIN, CollisionPatch, ContactError, TerrainQueries};
use procgen_core::Vec3;

pub const WALK_RADIUS: f32 = 0.035;
pub const EYE_HEIGHT: f32 = 0.12;
pub const WALK_SPEED: f32 = 0.18;
pub const WALKABLE_COSINE: f32 = 0.65;
const GRAVITY: f32 = 1.5;
const STEP_SECONDS: f32 = 1.0 / 120.0;
pub const MAX_WALK_SECONDS: f32 = 0.1;

pub struct Walker {
    center: Vec3,
    velocity: Vec3,
    patch: CollisionPatch,
    grounded: bool,
}
impl Walker {
    pub fn land(terrain: &TerrainQueries, direction: Vec3) -> Result<Self, ContactError> {
        if !direction.is_finite() || direction.length_squared() < 1e-12 {
            return Err(ContactError::InvalidMotion);
        }
        let hit = terrain.exterior(direction).ok_or(ContactError::NoLanding)?;
        let up = hit.position.normalized();
        if hit.normal.dot(up) < WALKABLE_COSINE {
            return Err(ContactError::NoLanding);
        }
        let center = hit.position + hit.normal * (WALK_RADIUS + 0.005);
        let patch = terrain.patch(center)?;
        if patch
            .clearance(terrain, center)
            .is_some_and(|(d, _)| d < WALK_RADIUS)
        {
            return Err(ContactError::NoLanding);
        }
        Ok(Self {
            center,
            velocity: Vec3::ZERO,
            patch,
            grounded: false,
        })
    }
    pub fn center(&self) -> Vec3 {
        self.center
    }
    pub fn eye(&self) -> Vec3 {
        self.center + self.up() * (EYE_HEIGHT - WALK_RADIUS)
    }
    pub fn up(&self) -> Vec3 {
        self.center.normalized()
    }
    pub fn grounded(&self) -> bool {
        self.grounded
    }
    pub fn clearance(&self, terrain: &TerrainQueries) -> Option<f32> {
        self.patch.clearance(terrain, self.center).map(|(d, _)| d)
    }
    pub fn triangle_count(&self) -> usize {
        self.patch.triangle_count()
    }
    /// Bounded simulation time. Missing or over-capacity coverage stops motion.
    pub fn advance(
        &mut self,
        terrain: &TerrainQueries,
        input: Vec3,
        seconds: f32,
    ) -> Result<(), ContactError> {
        if !input.is_finite()
            || !seconds.is_finite()
            || !(0.0..=MAX_WALK_SECONDS).contains(&seconds)
        {
            return Err(ContactError::InvalidMotion);
        }
        let steps = (seconds / STEP_SECONDS).ceil() as usize;
        if steps == 0 {
            return Ok(());
        }
        let dt = seconds / steps as f32;
        for _ in 0..steps {
            if !self.patch.covers(self.center, 0.3) {
                self.patch = terrain.patch(self.center)?;
            }
            let up = self.up();
            let tangent = input - up * input.dot(up);
            let tangent = if tangent.length_squared() > 1e-12 {
                tangent.normalized() * input.length().min(1.0)
            } else {
                Vec3::ZERO
            };
            if self.grounded {
                self.velocity = Vec3::ZERO;
            } else {
                self.velocity = self.velocity - up * (GRAVITY * dt);
            }
            let mut motion = (tangent * WALK_SPEED + self.velocity) * dt;
            self.grounded = false;
            for _ in 0..4 {
                let before = self.center;
                let sweep = self.patch.sweep(terrain, before, motion, WALK_RADIUS)?;
                self.center = sweep.position;
                let Some(normal) = sweep.normal else {
                    break;
                };
                if normal.dot(up) >= WALKABLE_COSINE {
                    self.grounded = true;
                }
                let into = self.velocity.dot(normal);
                if into < 0.0 {
                    self.velocity = self.velocity - normal * into;
                }
                motion = motion - (self.center - before);
                let into = motion.dot(normal);
                if into < 0.0 {
                    motion = motion - normal * into;
                }
                if motion.length_squared() < 1e-12 {
                    break;
                }
                // Release the contact skin before sliding. Reject the release if
                // another triangle blocks it, as at a narrow corner.
                let release = self.center + normal * (CONTACT_SKIN * 4.0);
                if self
                    .patch
                    .clearance(terrain, release)
                    .is_some_and(|(d, _)| d < WALK_RADIUS + CONTACT_SKIN)
                {
                    break;
                }
                self.center = release;
            }
            // A short downward support query keeps contact across sloped triangle
            // joins. Walkable support has static friction; gravity does not slide
            // an idle character down it.
            let support = self
                .patch
                .sweep(terrain, self.center, -up * 0.005, WALK_RADIUS)?;
            if support.normal.is_some_and(|n| n.dot(up) >= WALKABLE_COSINE) {
                self.center = support.position;
                self.velocity = Vec3::ZERO;
                self.grounded = true;
            }
        }
        Ok(())
    }
}
