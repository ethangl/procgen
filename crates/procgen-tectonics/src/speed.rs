//! What decides how fast a plate moves.
//!
//! The fit in [`crate::motion`] sets each plate's axis; this sets the length
//! of its rotation vector. Speed is the plate's hashed base speed scaled by
//! two things about the plate itself: the continental crust it has to drag,
//! and the slab hanging off its trenches.
//!
//! Slab pull is the one that spreads plate speeds. Forsyth and Uyeda (1975)
//! found no correlation between a plate's area and its speed — the Pacific is
//! the largest plate and among the fastest — a negative one with continental
//! area, and a strong positive one with the share of the plate's perimeter
//! that is subducting slab: Nazca and Cocos, half trench, move at 8 to 10
//! cm/yr where Eurasia, Antarctica, and Africa, with no slab to speak of,
//! move at 1 to 2.
//!
//! The rule is therefore also a feedback, because the trench share is a
//! property of the world rather than of the plate's draw. Evolution recomputes
//! it every step: a plate that starts subducting speeds up, which is the
//! Farallon story, and a plate that loses its trench slows down.
//!
//! Multiply, add, divide, and `min` alone, so no libm call sits on the path
//! that decides an integer.

use crate::{PlateKinematics, PlateKinematicsConfig};

/// The speed multiplier a plate's crust earns it: the oceanic factor at no
/// continental area, the continental factor at nothing but, and a linear
/// interpolation in between, because a plate that is half continent is half
/// as slowed by it.
pub(crate) fn crust_speed_factor(continental_fraction: f64, config: PlateKinematicsConfig) -> f32 {
    let oceanic = f64::from(config.oceanic_speed_factor);
    let continental = f64::from(config.continental_speed_factor);
    (oceanic + (continental - oceanic) * continental_fraction) as f32
}

/// The speed multiplier a plate's trenches earn it: the trenchless factor
/// where none of its boundary is subducting slab, the whole of its base at or
/// past [`PlateKinematicsConfig::slab_saturation_fraction`], and a linear ramp
/// between. Saturating rather than running on keeps a plate that is nothing
/// but trench from outrunning the ceiling by a fraction nothing measured.
fn slab_speed_factor(subducting_fraction: f64, config: PlateKinematicsConfig) -> f32 {
    let trenchless = f64::from(config.trenchless_speed_factor);
    let reach = (subducting_fraction / f64::from(config.slab_saturation_fraction)).min(1.0);
    (trenchless + (1.0 - trenchless) * reach) as f32
}

/// The speed the fit alone can give a plate: its hashed base scaled by the
/// crust it carries, clamped to the configured ceiling.
///
/// It is what [`generate_plate_kinematics`] returns and the only speed the
/// pipeline has before the ocean floor has ages. [`plate_speed`] is the speed
/// a plate turns at; at or past saturation the two agree, because a plate
/// whose whole perimeter is trench keeps the whole of its base.
pub(crate) fn crust_scaled_speed(
    base: f32,
    continental_fraction: f64,
    config: PlateKinematicsConfig,
) -> f32 {
    (base * crust_speed_factor(continental_fraction, config)).min(config.maximum_angular_speed)
}

/// The speed one plate turns at: its hashed base speed scaled by the crust it
/// carries and by the slab hanging off its trenches, clamped to the configured
/// ceiling.
///
/// There is no floor. A plate with a continent on it and no slab is slow, and
/// that spread is the whole point: [`PlateKinematicsConfig::minimum_angular_speed`]
/// bounds the hashed draw and nothing else.
///
/// `continental_fraction` is the plate's continental share of its own area, as
/// [`CrustClassification::plate_continental_fraction`] gives it, and
/// `subducting_fraction` its share of boundary edges that are subducting slab,
/// as [`crate::subducting_fractions`] gives it. Multiply, add, divide, and
/// `min` alone, so no libm call sits on the path that decides an integer.
pub fn plate_speed(
    base: f32,
    continental_fraction: f64,
    subducting_fraction: f64,
    config: PlateKinematicsConfig,
) -> f32 {
    (base
        * crust_speed_factor(continental_fraction, config)
        * slab_speed_factor(subducting_fraction, config))
    .min(config.maximum_angular_speed)
}

/// Spread of the plate speeds a world holds, for the consumers that report
/// how far the fastest plate outruns the slowest.
///
/// Deliberately not [`crate::FieldSummary`], which the per-cell fields use.
/// The middle number here is a median rather than a mean because the thing
/// being reported is the spread, and a handful of plates at the ceiling pull a
/// mean of a hundred speeds well off where most of them sit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlateSpeedSummary {
    pub minimum: f32,
    pub median: f32,
    pub maximum: f32,
}

impl PlateSpeedSummary {
    /// How far the fastest plate outruns the slowest. Infinite for a world
    /// holding a plate at rest.
    pub fn ratio(&self) -> f32 {
        self.maximum / self.minimum
    }
}

impl PlateKinematics {
    /// The smallest, middle, and largest speed a plate turns at. An even plate
    /// count takes the mean of the middle pair, and an empty world is all
    /// zeroes.
    pub fn speed_summary(&self) -> PlateSpeedSummary {
        let mut speeds: Vec<f32> = self
            .angular_velocities
            .iter()
            .map(|rotation| rotation.length())
            .collect();
        speeds.sort_by(f32::total_cmp);
        let Some((&minimum, _)) = speeds.split_first() else {
            return PlateSpeedSummary::default();
        };
        let half = speeds.len() / 2;
        PlateSpeedSummary {
            minimum,
            median: match speeds.len() % 2 {
                0 => (speeds[half - 1] + speeds[half]) / 2.0,
                _ => speeds[half],
            },
            maximum: speeds[speeds.len() - 1],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_core::Vec3;

    /// A base speed inside the default hashed range that the crust and slab
    /// factors cannot carry past the default ceiling, so the unit tests of the
    /// rule read the ramp rather than the clamp.
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
    fn the_slab_factor_ramps_from_trenchless_to_saturation() {
        // All continent, whose factor is one by default, so the slab factor is
        // the only thing between the base speed and the answer.
        let config = PlateKinematicsConfig::new(7);
        let speed = |fraction: f64| plate_speed(BASE, 1.0, fraction, config);
        let saturation = f64::from(config.slab_saturation_fraction);
        let trenchless = BASE * config.trenchless_speed_factor;

        // A plate with no trench keeps the trenchless share alone, and one at
        // or past saturation keeps the whole of its base. The defaults are
        // exact in binary, so these are equalities.
        assert_eq!(speed(0.0), trenchless);
        assert_eq!(speed(saturation), BASE);
        assert_eq!(speed(1.0), BASE);
        // Linear between: half the saturation fraction is half the way up.
        assert_eq!(speed(saturation / 2.0), (trenchless + BASE) / 2.0);
    }

    #[test]
    fn the_speed_rule_is_bounded_above_and_not_below() {
        let config = PlateKinematicsConfig::new(7);
        // A base at the top of the hashed range, all ocean, and a plate that
        // is nothing but trench: the crust factor alone would carry it half
        // again past the ceiling.
        assert_eq!(
            plate_speed(config.maximum_angular_speed, 0.0, 1.0, config),
            config.maximum_angular_speed
        );
        // The same plate with a continent on it and no trench is far below
        // the minimum the hashed base is drawn from, which bounds the draw
        // and nothing else.
        let slow = plate_speed(config.minimum_angular_speed, 1.0, 0.0, config);
        assert!(slow < config.minimum_angular_speed, "{slow}");
        assert_eq!(
            slow,
            config.minimum_angular_speed
                * config.continental_speed_factor
                * config.trenchless_speed_factor
        );
    }

    /// A plate whose whole perimeter is trench keeps the whole of its base, so
    /// the speed the stage fits and the speed the plate turns at agree there
    /// and nowhere above it.
    #[test]
    fn the_crust_scaled_speed_is_the_rule_at_saturation() {
        let config = PlateKinematicsConfig::new(7);
        for continental in [0.0, 0.4, 1.0] {
            assert_eq!(
                crust_scaled_speed(BASE, continental, config),
                plate_speed(BASE, continental, 1.0, config)
            );
            assert!(
                crust_scaled_speed(BASE, continental, config)
                    > plate_speed(BASE, continental, 0.0, config)
            );
        }
    }

    #[test]
    fn the_speed_summary_reports_the_spread_of_a_world() {
        let speeds = |values: &[f32]| {
            PlateKinematics {
                angular_velocities: values
                    .iter()
                    .map(|&speed| Vec3::new(speed, 0.0, 0.0))
                    .collect(),
                base_speeds: values.to_vec(),
            }
            .speed_summary()
        };

        // An odd count takes the middle value and an even one the mean of the
        // middle pair, over the speeds sorted rather than as given.
        let odd = speeds(&[0.5, 2.0, 1.0]);
        assert_eq!(odd.minimum, 0.5);
        assert_eq!(odd.median, 1.0);
        assert_eq!(odd.maximum, 2.0);
        assert_eq!(odd.ratio(), 4.0);
        assert_eq!(speeds(&[1.0, 4.0, 2.0, 3.0]).median, 2.5);
        // A world of no plates has no spread to report.
        assert_eq!(speeds(&[]), PlateSpeedSummary::default());
    }
}
