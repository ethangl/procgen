//! Minimal stellar, orbital, rotation, size, and atmosphere inputs for planetary
//! generation stages.
//!
//! Units are explicit SI units. Presets own real-world parameter choices;
//! downstream algorithms consume only the supplied values.

use std::{fmt, ops::RangeInclusive};

/// Supported physical range for planet radius.
pub const PLANET_RADIUS_METERS_RANGE: RangeInclusive<f64> = 1.0..=1.0e9;
/// Supported physical range for sidereal rotation period. Zero is
/// the explicit non-rotating case.
pub const SIDEREAL_ROTATION_PERIOD_SECONDS_RANGE: RangeInclusive<f64> = 0.0..=1.0e9;
/// Supported physical range for an atmosphere's specific gas constant.
pub const ATMOSPHERIC_SPECIFIC_GAS_CONSTANT_RANGE: RangeInclusive<f64> = 1.0..=1.0e5;
/// Supported physical height represented by normalized land elevation.
pub const MAXIMUM_LAND_ELEVATION_METERS_RANGE: RangeInclusive<f64> = 0.0..=100_000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Star {
    pub luminosity_watts: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orbit {
    pub semi_major_axis_meters: f64,
    pub eccentricity: f64,
    pub obliquity_radians: f64,
    /// Apparent stellar longitude at periapsis, measured from the northern
    /// vernal equinox in the direction of orbital motion.
    pub stellar_longitude_at_periapsis_radians: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Planet {
    pub star: Star,
    pub orbit: Orbit,
    pub radius_meters: f64,
    /// Sidereal rotation period in seconds. A positive value rotates about the
    /// model's Y axis; zero represents a non-rotating body.
    pub sidereal_rotation_period_seconds: f64,
    /// Specific gas constant of the bulk atmosphere in J kg^-1 K^-1.
    pub atmospheric_specific_gas_constant_joules_per_kilogram_kelvin: f64,
    /// Physical height represented by normalized elevation 1.0 above sea level.
    pub maximum_land_elevation_meters: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanetValidationError {
    Luminosity,
    SemiMajorAxis,
    Eccentricity,
    Obliquity,
    PeriapsisLongitude,
    Radius,
    RotationPeriod,
    AtmosphericGasConstant,
    MaximumLandElevation,
}

impl fmt::Display for PlanetValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Luminosity => "stellar luminosity must be finite and positive",
            Self::SemiMajorAxis => "orbital semi-major axis must be finite and positive",
            Self::Eccentricity => "orbital eccentricity must be finite and in [0, 1)",
            Self::Obliquity => "obliquity must be finite and in [0, pi / 2]",
            Self::PeriapsisLongitude => "stellar longitude at periapsis must be finite",
            Self::Radius => "planet radius is outside the supported range",
            Self::RotationPeriod => "sidereal rotation period is outside the supported range",
            Self::AtmosphericGasConstant => {
                "atmospheric specific gas constant is outside the supported range"
            }
            Self::MaximumLandElevation => "maximum land elevation is outside the supported range",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for PlanetValidationError {}

impl Planet {
    /// Present-day Earth-like parameters. These values are a convenient
    /// generation preset, not constants embedded in climate algorithms.
    pub const EARTH: Self = Self {
        star: Star {
            luminosity_watts: 3.828e26,
        },
        orbit: Orbit {
            semi_major_axis_meters: 149_597_870_700.0,
            eccentricity: 0.016_708_6,
            obliquity_radians: 23.439_281_1_f64.to_radians(),
            stellar_longitude_at_periapsis_radians: 282.937_681_93_f64.to_radians(),
        },
        radius_meters: 6_371_000.0,
        sidereal_rotation_period_seconds: 86_164.090_5,
        atmospheric_specific_gas_constant_joules_per_kilogram_kelvin: 287.05,
        maximum_land_elevation_meters: 10_000.0,
    };

    pub fn validate(self) -> Result<(), PlanetValidationError> {
        if !self.star.luminosity_watts.is_finite() || self.star.luminosity_watts <= 0.0 {
            return Err(PlanetValidationError::Luminosity);
        }
        if !self.orbit.semi_major_axis_meters.is_finite()
            || self.orbit.semi_major_axis_meters <= 0.0
        {
            return Err(PlanetValidationError::SemiMajorAxis);
        }
        if !self.orbit.eccentricity.is_finite() || !(0.0..1.0).contains(&self.orbit.eccentricity) {
            return Err(PlanetValidationError::Eccentricity);
        }
        if !self.orbit.obliquity_radians.is_finite()
            || !(0.0..=std::f64::consts::FRAC_PI_2).contains(&self.orbit.obliquity_radians)
        {
            return Err(PlanetValidationError::Obliquity);
        }
        if !self
            .orbit
            .stellar_longitude_at_periapsis_radians
            .is_finite()
        {
            return Err(PlanetValidationError::PeriapsisLongitude);
        }
        if !self.radius_meters.is_finite()
            || !PLANET_RADIUS_METERS_RANGE.contains(&self.radius_meters)
        {
            return Err(PlanetValidationError::Radius);
        }
        if !self.sidereal_rotation_period_seconds.is_finite()
            || !SIDEREAL_ROTATION_PERIOD_SECONDS_RANGE
                .contains(&self.sidereal_rotation_period_seconds)
        {
            return Err(PlanetValidationError::RotationPeriod);
        }
        let gas_constant = self.atmospheric_specific_gas_constant_joules_per_kilogram_kelvin;
        if !gas_constant.is_finite()
            || !ATMOSPHERIC_SPECIFIC_GAS_CONSTANT_RANGE.contains(&gas_constant)
        {
            return Err(PlanetValidationError::AtmosphericGasConstant);
        }
        if !MAXIMUM_LAND_ELEVATION_METERS_RANGE.contains(&self.maximum_land_elevation_meters) {
            return Err(PlanetValidationError::MaximumLandElevation);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earth_preset_is_valid() {
        assert_eq!(Planet::EARTH.validate(), Ok(()));
    }

    #[test]
    fn rejects_nonphysical_inputs() {
        let mut planet = Planet::EARTH;
        planet.orbit.eccentricity = 1.0;
        assert_eq!(planet.validate(), Err(PlanetValidationError::Eccentricity));

        planet = Planet::EARTH;
        planet.star.luminosity_watts = f64::NAN;
        assert_eq!(planet.validate(), Err(PlanetValidationError::Luminosity));

        planet = Planet::EARTH;
        planet.sidereal_rotation_period_seconds = -1.0;
        assert_eq!(
            planet.validate(),
            Err(PlanetValidationError::RotationPeriod)
        );

        planet = Planet::EARTH;
        planet.maximum_land_elevation_meters = f64::NAN;
        assert_eq!(
            planet.validate(),
            Err(PlanetValidationError::MaximumLandElevation)
        );
    }
}
