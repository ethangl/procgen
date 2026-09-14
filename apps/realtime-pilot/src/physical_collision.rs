//! Retained collision coverage and bounded lookahead for asynchronous replacement.
use crate::{MeterPosition, VOXEL_CHUNK_CELLS, VoxelCollision, VoxelPosition};
use procgen_core::Vec3;

const REFRESH_MARGIN_M: f32 = 20.0;
const INSTALL_MARGIN_M: f32 = 1.0;

#[derive(Default)]
pub struct PhysicalCollision {
    patch: Option<VoxelCollision>,
}
impl PhysicalCollision {
    pub fn patch(&self) -> Option<&VoxelCollision> {
        self.patch.as_ref()
    }
    /// Forecast coverage starts a build early; its center stays close enough to
    /// the current position that the requested box also covers the player.
    pub fn request(
        &self,
        position: MeterPosition,
        forecast: MeterPosition,
    ) -> Option<VoxelPosition> {
        if self.patch.as_ref().is_some_and(|patch| {
            patch.covers(position.relative_to(patch.origin_m()), REFRESH_MARGIN_M)
                && patch.covers(forecast.relative_to(patch.origin_m()), REFRESH_MARGIN_M)
        }) {
            return None;
        }
        let delta =
            forecast.relative_to(position.anchor()) - position.relative_to(position.anchor());
        let reach = VOXEL_CHUNK_CELLS as f32 * 0.5;
        let ahead = Vec3::new(
            delta.x.clamp(-reach, reach),
            delta.y.clamp(-reach, reach),
            delta.z.clamp(-reach, reach),
        );
        Some(position.translated(ahead).anchor())
    }
    /// Stale work cannot remove the coverage still supporting movement.
    pub fn install(&mut self, patch: VoxelCollision, position: MeterPosition) -> bool {
        if !patch.covers(position.relative_to(patch.origin_m()), INSTALL_MARGIN_M) {
            return false;
        }
        self.patch = Some(patch);
        true
    }
    pub fn covers(&self, position: MeterPosition) -> bool {
        self.patch
            .as_ref()
            .is_some_and(|p| p.covers(position.relative_to(p.origin_m()), crate::PLAYER_RADIUS_M))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContactError, PhysicalWalker, PlanetDesignConfig, VoxelCollisionError};
    use std::sync::atomic::AtomicBool;

    fn flat_field() -> crate::PlanetDesignField {
        let mut config = PlanetDesignConfig::starter(42);
        config.radius_m = 8_000_000.0;
        config.octaves.iter_mut().for_each(|o| o.enabled = false);
        config.validate().unwrap()
    }
    #[test]
    fn stale_completion_preserves_retained_support_and_forecast_requests_early() {
        let field = flat_field();
        let position = MeterPosition::new(
            VoxelPosition {
                x_m: 8_000_001,
                y_m: 0,
                z_m: 0,
            },
            Vec3::ZERO,
        );
        let cancel = AtomicBool::new(false);
        let mut coverage = PhysicalCollision::default();
        assert!(coverage.install(
            VoxelCollision::build(&field, position.anchor(), &cancel).unwrap(),
            position
        ));
        assert!(coverage.request(position, position).is_none());
        let forecast = position.translated(Vec3::Y * 50.0);
        let request = coverage
            .request(position, forecast)
            .expect("prefetch before current support is missing");
        assert_eq!(
            request.y_m, 16,
            "bound the lookahead center to half a chunk"
        );
        let distant = position.translated(Vec3::Y * 200.0);
        let stale = VoxelCollision::build(&field, distant.anchor(), &cancel).unwrap();
        assert!(!coverage.install(stale, position));
        assert!(coverage.covers(position));
        assert_eq!(coverage.patch().unwrap().origin_m(), position.anchor());
    }

    fn walk_route(
        field: &crate::PlanetDesignField,
        delay_steps: usize,
        steps: usize,
    ) -> (usize, usize, f32) {
        let origin = VoxelPosition {
            x_m: (field.config().radius_m + field.elevation_m(Vec3::X, 0.0).unwrap()).round()
                as i32
                + 5,
            y_m: 0,
            z_m: 0,
        };
        let cancel = AtomicBool::new(false);
        let initial = VoxelCollision::build(field, origin, &cancel).unwrap();
        // Like the recorded route, move forward in flight until a walkable
        // triangle is available; the small preset starts above a steep face.
        let mut walker = (0..120)
            .find_map(|i| {
                match PhysicalWalker::land(
                    &initial,
                    MeterPosition::new(origin, Vec3::Y * (i as f32 * 0.16)),
                ) {
                    Ok(walker) => Some(walker),
                    Err(VoxelCollisionError::Contact(ContactError::NoLanding)) => None,
                    Err(error) => panic!("landing query failed: {error}"),
                }
            })
            .expect("walkable landing within the starting patch");
        let start = walker.center();
        let mut coverage = PhysicalCollision::default();
        assert!(coverage.install(initial, walker.center()));
        let mut pending = None;
        let mut replacements = 0;
        let mut pauses = 0;
        let mut resumed = false;
        for step in 0..steps {
            if pending.as_ref().is_some_and(|(ready, _)| step >= *ready) {
                let (_, patch) = pending.take().unwrap();
                if coverage.install(patch, walker.center()) {
                    replacements += 1;
                }
            }
            if pending.is_none()
                && let Some(request) =
                    coverage.request(walker.center(), walker.collision_forecast(Vec3::Y))
            {
                pending = Some((
                    step + delay_steps,
                    VoxelCollision::build(field, request, &cancel).unwrap(),
                ));
            }
            let previous = walker.center();
            match walker.advance(coverage.patch().unwrap(), Vec3::Y, 0.02) {
                Ok(()) => {
                    resumed |= pauses > 0 && walker.center() != previous;
                }
                Err(VoxelCollisionError::Contact(ContactError::MissingCoverage)) => {
                    assert_eq!(
                        walker.center(),
                        previous,
                        "missing coverage cannot move the player"
                    );
                    pauses += 1;
                }
                Err(e) => panic!("step {step}, delay {delay_steps}: {e}"),
            }
            assert!(
                coverage.covers(walker.center()),
                "retained patch covers every accepted position"
            );
        }
        if pauses > 0 {
            assert!(resumed, "late coverage must resume movement");
        }
        (
            replacements,
            pauses,
            (walker.center().relative_to(origin) - start.relative_to(origin)).y,
        )
    }
    #[test]
    fn delayed_replacements_cross_multiple_chunks_and_resume_after_a_long_stall() {
        let field = flat_field();
        for delay in [40, 1000] {
            let (replacements, pauses, distance) = walk_route(&field, delay, 3000);
            assert!(replacements >= 2);
            if delay == 40 {
                assert_eq!(pauses, 0);
                assert!((distance - 240.0).abs() < 0.02);
            } else {
                assert!(
                    pauses > 0,
                    "twenty-second stall exercises safe boundary stop"
                );
                assert!(
                    distance > 96.0,
                    "movement resumes beyond the initial coverage"
                );
            }
        }
    }
    #[test]
    fn saved_terrain_walks_keep_coverage_during_delayed_replacement() {
        for source in [
            include_str!("../../../planet-design.json"),
            include_str!("../../../planet-design-300km.json"),
        ] {
            let value: serde_json::Value = serde_json::from_str(source).unwrap();
            let config: PlanetDesignConfig =
                serde_json::from_value(value["design"].clone()).unwrap();
            let field = config.validate().unwrap();
            let (replacements, pauses, distance) = walk_route(&field, 75, 750);
            eprintln!(
                "radius {}: {replacements} replacements, {pauses} coverage pauses, {distance:.2} m tangent travel",
                config.radius_m
            );
            assert!(replacements > 0);
            assert_eq!(pauses, 0, "one-and-a-half-second worker latency");
            assert!(distance > 40.0);
        }
    }
}
