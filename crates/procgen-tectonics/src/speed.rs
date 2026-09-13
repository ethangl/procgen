//! What decides how fast a plate moves.
//!
//! The fit in [`crate::motion`] sets each plate's axis; this sets the length
//! of its rotation vector. Speed is the plate's hashed base speed scaled by
//! the one thing about the plate itself that bears on it: the continental
//! crust it has to drag. Forsyth and Uyeda (1975) found no correlation
//! between a plate's area and its speed — the Pacific is the largest plate
//! and among the fastest — and a negative one with continental area, which is
//! the correlation this keeps.
//!
//! Multiply, add, and `min` alone, so no libm call sits on the path that
//! decides an integer.

use crate::PlateKinematicsConfig;

/// The speed multiplier a plate's crust earns it: the oceanic factor at no
/// continental area, the continental factor at nothing but, and a linear
/// interpolation in between, because a plate that is half continent is half
/// as slowed by it.
fn crust_speed_factor(continental_fraction: f64, config: PlateKinematicsConfig) -> f32 {
    let oceanic = f64::from(config.oceanic_speed_factor);
    let continental = f64::from(config.continental_speed_factor);
    (oceanic + (continental - oceanic) * continental_fraction) as f32
}

/// The speed a plate turns at: its hashed base scaled by the crust it
/// carries, clamped to the configured ceiling.
///
/// There is no floor. A plate carrying a continent is slow, and that spread
/// is the point: [`PlateKinematicsConfig::minimum_angular_speed`] bounds the
/// hashed draw and nothing else.
///
/// `continental_fraction` is the plate's continental share of its own area,
/// as [`crate::CrustClassification::plate_continental_fraction`] gives it.
pub(crate) fn crust_scaled_speed(
    base: f32,
    continental_fraction: f64,
    config: PlateKinematicsConfig,
) -> f32 {
    (base * crust_speed_factor(continental_fraction, config)).min(config.maximum_angular_speed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A base speed inside the default hashed range that the crust factor
    /// cannot carry past the default ceiling, so the unit tests of the rule
    /// read the ramp rather than the clamp.
    const BASE: f32 = 0.8;

    #[test]
    fn the_crust_factor_interpolates_by_continental_area() {
        let config = PlateKinematicsConfig::new(7);
        // The defaults are 1.5 and 1.0, whose mean is exact in binary, so the
        // midpoint is an equality rather than a tolerance.
        assert_eq!(crust_speed_factor(0.0, config), config.oceanic_speed_factor);
        assert_eq!(
            crust_speed_factor(1.0, config),
            config.continental_speed_factor
        );
        assert_eq!(
            crust_speed_factor(0.5, config),
            (config.oceanic_speed_factor + config.continental_speed_factor) / 2.0
        );
    }

    #[test]
    fn the_speed_rule_is_bounded_above_and_not_below() {
        let config = PlateKinematicsConfig::new(7);
        // A base at the top of the hashed range and all ocean: the crust
        // factor alone would carry it half again past the ceiling.
        assert_eq!(
            crust_scaled_speed(config.maximum_angular_speed, 0.0, config),
            config.maximum_angular_speed
        );
        // The same plate with a continent on it is below the minimum the
        // hashed base is drawn from, which bounds the draw and nothing else.
        let slow = crust_scaled_speed(config.minimum_angular_speed, 1.0, config);
        assert_eq!(
            slow,
            config.minimum_angular_speed * config.continental_speed_factor
        );
        assert!(slow < config.oceanic_speed_factor * config.minimum_angular_speed);
    }

    /// The crust factor is the only thing between the base and the answer, so
    /// an all-ocean plate keeps more of its base than a continental one.
    #[test]
    fn continental_crust_slows_a_plate_below_an_oceanic_one() {
        let config = PlateKinematicsConfig::new(7);
        assert!(
            crust_scaled_speed(BASE, 1.0, config) < crust_scaled_speed(BASE, 0.0, config),
            "continent must not be the faster crust"
        );
    }
}
