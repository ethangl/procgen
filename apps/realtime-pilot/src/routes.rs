//! Reproducible camera inputs, separate from generation and collision.
use crate::StreamView;
use procgen_core::Vec3;
pub const ROUTE_SECONDS: f32 = 36.0;
pub const FLIGHT_SPEED: f32 = 0.5;
pub const FAST_FLIGHT_SPEED: f32 = 4.0;
pub struct RouteSample {
    pub view: StreamView,
    pub phase: &'static str,
    pub phase_seconds: f32,
}
pub fn streaming_route(seconds: f32) -> RouteSample {
    let (position, forward, phase, start) = if seconds < 12.0 {
        let p = Vec3::Z * (13.0 + (4.45 - 13.0) * (seconds / 12.0));
        (p, -Vec3::Z, "descent", 0.0)
    } else if seconds < 18.0 {
        // The last two seconds deliberately outrun multi-piece uploads to
        // exercise cancellation. Camera trigonometry is not a density primitive.
        let angle = if seconds < 16.0 {
            (seconds - 12.0) * std::f32::consts::TAU / 1.5
        } else {
            4.0 * std::f32::consts::TAU / 1.5 + (seconds - 16.0) * std::f32::consts::TAU / 0.12
        };
        (
            Vec3::Z * (4.45 + 1.05 * ((seconds - 16.0) / 2.0).clamp(0.0, 1.0)),
            Vec3::new(angle.sin(), 0.0, -angle.cos()),
            "rapid-turns",
            12.0,
        )
    } else if seconds < 30.0 {
        let radius = 5.5;
        let angle = (seconds - 18.0) * FAST_FLIGHT_SPEED / radius;
        let direction = Vec3::new(angle.sin(), 0.0, angle.cos());
        (direction * radius, -direction, "fast-flight", 18.0)
    } else {
        let angle = 12.0 * FAST_FLIGHT_SPEED / 5.5;
        let direction = Vec3::new(angle.sin(), 0.0, angle.cos());
        (
            direction * (5.5 + (13.0 - 5.5) * ((seconds - 30.0) / 6.0)),
            -direction,
            "retreat",
            30.0,
        )
    };
    RouteSample {
        view: StreamView { position, forward },
        phase,
        phase_seconds: seconds - start,
    }
}

/// Shared fixed walking inputs for native runs and deterministic headless audits.
pub fn walking_input(seconds: f32) -> Vec3 {
    if seconds < 12.0 {
        Vec3::X
    } else if seconds < 18.0 {
        Vec3::ZERO
    } else if seconds < 30.0 {
        -Vec3::X
    } else {
        Vec3::ZERO
    }
}
pub fn route_walker(terrain: &crate::UsableTerrain) -> Result<crate::Walker, crate::ContactError> {
    terrain
        .population
        .placements()
        .iter()
        .find_map(|p| crate::Walker::land(&terrain.queries, p.position).ok())
        .ok_or(crate::ContactError::NoLanding)
}
