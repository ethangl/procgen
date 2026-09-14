//! Physical broad terrain for the planet-scale pilot. No density or LOD storage.
use std::{error::Error, fmt};

use procgen_core::{Vec3, hash_u32};
use procgen_noise::fold_seed_u64_to_u32;
use serde::{Deserialize, Serialize};

use crate::{
    FieldError, HeightDistribution, NoiseConfig,
    field::validate_range,
    height_distribution::sample_heights,
    noise::{normalized_noise, shape},
};

pub const MAX_DESIGN_OCTAVES: usize = 24;

/// Amplitude is in meters before feedback damping and the final height bound.
/// Zero amplitude or disabled means no contribution, including feedback.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OctaveConfig {
    pub enabled: bool,
    pub wavelength_m: f32,
    pub amplitude_m: f32,
    pub sharpness: f32,
    pub perturbation: f32,
    pub slope_erosion: f32,
    pub altitude_erosion: f32,
    pub ridge_erosion: f32,
}
impl OctaveConfig {
    pub const WAVELENGTH_RANGE: std::ops::RangeInclusive<f32> = 8.0..=16_000_000.0;
    pub const AMPLITUDE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=20_000.0;
}

/// Serialized generation contract. The reference sphere is the zero-elevation
/// datum. Preview/camera preferences are not generation parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetDesignConfig {
    pub seed: u64,
    pub radius_m: f32,
    pub height_limit_m: f32,
    /// Strictly descending wavelengths; octave zero is the broadest band.
    pub octaves: Vec<OctaveConfig>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DesignError {
    Field(FieldError),
    OctaveCount,
    WavelengthOrder,
    Direction,
    Spacing,
}
impl From<FieldError> for DesignError {
    fn from(value: FieldError) -> Self {
        Self::Field(value)
    }
}
impl fmt::Display for DesignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field(e) => e.fmt(f),
            Self::OctaveCount => write!(f, "use 1 to {MAX_DESIGN_OCTAVES} octaves"),
            Self::WavelengthOrder => write!(f, "octave wavelengths must strictly decrease"),
            Self::Direction => write!(f, "direction must be finite and nonzero"),
            Self::Spacing => write!(f, "sample spacing must be finite and nonnegative"),
        }
    }
}
impl Error for DesignError {}

impl PlanetDesignConfig {
    pub const RADIUS_RANGE: std::ops::RangeInclusive<f32> = 100_000.0..=8_000_000.0;
    pub const HEIGHT_LIMIT_RANGE: std::ops::RangeInclusive<f32> = 1.0..=20_000.0;

    /// Provisional editing preset, not a reconstruction of the source game's world.
    pub fn starter(seed: u64) -> Self {
        Self {
            seed,
            radius_m: 2_000_000.0,
            height_limit_m: 12_000.0,
            octaves: (0..18)
                .map(|i| OctaveConfig {
                    enabled: true,
                    wavelength_m: 1_048_576.0 / (1_u32 << i) as f32,
                    amplitude_m: 4_000.0 * 0.55_f32.powi(i),
                    sharpness: if i < 2 { 0.0 } else { -0.5 },
                    perturbation: 0.15,
                    slope_erosion: 0.15,
                    altitude_erosion: 0.0,
                    ridge_erosion: 0.1,
                })
                .collect(),
        }
    }

    pub fn validate(&self) -> Result<PlanetDesignField, DesignError> {
        validate_range("radius (m)", self.radius_m, Self::RADIUS_RANGE)?;
        validate_range(
            "height limit (m)",
            self.height_limit_m,
            Self::HEIGHT_LIMIT_RANGE,
        )?;
        if self.octaves.is_empty() || self.octaves.len() > MAX_DESIGN_OCTAVES {
            return Err(DesignError::OctaveCount);
        }
        for o in &self.octaves {
            validate_range(
                "wavelength (m)",
                o.wavelength_m,
                OctaveConfig::WAVELENGTH_RANGE,
            )?;
            validate_range(
                "amplitude (m)",
                o.amplitude_m,
                OctaveConfig::AMPLITUDE_RANGE,
            )?;
            validate_range("sharpness", o.sharpness, NoiseConfig::SHARPNESS_RANGE)?;
            for (name, value) in [
                ("warp", o.perturbation),
                ("slope damping", o.slope_erosion),
                ("altitude damping", o.altitude_erosion),
                ("ridge damping", o.ridge_erosion),
            ] {
                validate_range(name, value, NoiseConfig::SHAPING_RANGE)?;
            }
        }
        if self
            .octaves
            .windows(2)
            .any(|p| p[0].wavelength_m <= p[1].wavelength_m)
        {
            return Err(DesignError::WavelengthOrder);
        }
        Ok(PlanetDesignField {
            config: self.clone(),
            key: hash_u32(fold_seed_u64_to_u32(self.seed), 0x5355_5246, 0, 0),
        })
    }
}

#[derive(Clone, Debug)]
pub struct PlanetDesignField {
    config: PlanetDesignConfig,
    key: u32,
}
impl PlanetDesignField {
    pub(crate) fn noise_key(&self) -> u32 {
        self.key
    }

    pub fn config(&self) -> &PlanetDesignConfig {
        &self.config
    }

    /// Zero spacing evaluates every enabled octave. Nonzero spacing smoothly
    /// removes wavelengths from four down to two samples per wavelength.
    pub fn elevation_m(&self, direction: Vec3, spacing_m: f32) -> Result<f32, DesignError> {
        if !direction.is_finite()
            || !direction.length_squared().is_finite()
            || direction.length_squared() == 0.0
        {
            return Err(DesignError::Direction);
        }
        if !spacing_m.is_finite() || spacing_m < 0.0 {
            return Err(DesignError::Spacing);
        }
        Ok(self.height(direction.normalized(), spacing_m))
    }

    pub fn height_distribution(&self) -> HeightDistribution {
        sample_heights(|direction| self.height(direction, 0.0))
    }

    pub(crate) fn height(&self, direction: Vec3, spacing_m: f32) -> f32 {
        let mut sum = 0.0;
        let mut slope = Vec3::ZERO;
        let mut ridge = Vec3::ZERO;
        let mut warp = Vec3::ZERO;
        for o in &self.config.octaves {
            if !o.enabled || o.amplitude_m == 0.0 {
                continue;
            }
            let weight = octave_weight(o.wavelength_m, spacing_m);
            // Filter feedback as well as height. Unresolved octaves cannot alter
            // later bands or re-enter through domain warp.
            if weight == 0.0 {
                continue;
            }
            let p = direction * (self.config.radius_m / o.wavelength_m) + warp;
            let sample = normalized_noise(self.key, p);
            let d = sample.derivative * weight;
            slope = slope + d * o.slope_erosion;
            ridge = ridge + d * o.ridge_erosion;
            let altitude = (sum / self.config.height_limit_m).clamp(0.0, 1.0);
            let smooth = altitude * altitude * (3.0 - 2.0 * altitude);
            let damping = (1.0 + (smooth - 1.0) * o.altitude_erosion)
                * (1.0 - o.ridge_erosion / (1.0 + ridge.length_squared()))
                / (1.0 + slope.length_squared());
            sum += weight * o.amplitude_m * shape(sample, o.sharpness).value * damping;
            warp = warp + d * o.perturbation;
        }
        let relative = sum / self.config.height_limit_m;
        sum / (1.0 + relative * relative).sqrt()
    }
}

pub(crate) fn octave_weight(wavelength_m: f32, spacing_m: f32) -> f32 {
    if spacing_m == 0.0 {
        return 1.0;
    }
    let t = ((wavelength_m / spacing_m - 2.0) * 0.5).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::positions;

    #[test]
    fn physical_scale_and_serialization_preserve_the_field() {
        let config = PlanetDesignConfig::starter(4_294_967_338);
        let encoded = serde_json::to_string(&config).unwrap();
        let restored: PlanetDesignConfig = serde_json::from_str(&encoded).unwrap();
        assert_eq!(config, restored);
        let field = config.validate().unwrap();
        let mut doubled = restored;
        doubled.radius_m *= 2.0;
        doubled.height_limit_m *= 1.5;
        for o in &mut doubled.octaves {
            o.wavelength_m *= 2.0;
            o.amplitude_m *= 1.5;
        }
        let larger = doubled.validate().unwrap();
        for p in positions().take(100) {
            let h = field.elevation_m(p, 100.0).unwrap();
            // Centimeter margin for f32 summation at kilometer elevation scales.
            assert!((larger.elevation_m(p, 200.0).unwrap() - h * 1.5).abs() < 0.01);
        }
    }

    #[test]
    fn filtered_and_disabled_bands_leave_no_feedback() {
        let mut config = PlanetDesignConfig::starter(42);
        let field = config.validate().unwrap();
        config.octaves[1..]
            .iter_mut()
            .for_each(|o| o.enabled = false);
        let coarse = config.validate().unwrap();
        // First band has four samples per wavelength, second has two.
        let spacing = config.octaves[0].wavelength_m / 4.0;
        for p in positions().take(100) {
            assert_eq!(
                field.elevation_m(p, spacing).unwrap(),
                coarse.elevation_m(p, 0.0).unwrap()
            );
        }
        config.octaves[0].enabled = false;
        assert_eq!(
            config
                .validate()
                .unwrap()
                .elevation_m(Vec3::X, 0.0)
                .unwrap(),
            0.0
        );
    }

    #[test]
    fn validation_rejects_bad_order_lengths_and_nonfinite_queries() {
        let mut config = PlanetDesignConfig::starter(42);
        config.octaves.swap(0, 1);
        assert!(matches!(
            config.validate(),
            Err(DesignError::WavelengthOrder)
        ));
        config.octaves.swap(0, 1);
        config.radius_m = f32::NAN;
        assert!(config.validate().is_err());
        let field = PlanetDesignConfig::starter(42).validate().unwrap();
        assert!(field.elevation_m(Vec3::ZERO, 0.0).is_err());
        assert!(field.elevation_m(Vec3::X, -1.0).is_err());
    }

    #[test]
    fn per_band_controls_change_the_field_and_extremes_remain_bounded() {
        let config = PlanetDesignConfig::starter(42);
        let base = config.validate().unwrap();
        let mut variants = Vec::new();
        for control in 0..6 {
            let mut variant = config.clone();
            let band = &mut variant.octaves[2];
            match control {
                0 => band.amplitude_m *= 2.0,
                1 => band.sharpness = 1.0,
                2 => band.perturbation = 1.0,
                3 => band.slope_erosion = 1.0,
                4 => band.altitude_erosion = 1.0,
                5 => band.ridge_erosion = 1.0,
                _ => unreachable!(),
            }
            variants.push(variant.validate().unwrap());
        }
        for variant in variants {
            let difference: f32 = positions()
                .take(100)
                .map(|p| {
                    (base.elevation_m(p, 0.0).unwrap() - variant.elevation_m(p, 0.0).unwrap()).abs()
                })
                .sum();
            assert!(difference > 1.0);
        }
        let mut extreme = config;
        extreme.radius_m = *PlanetDesignConfig::RADIUS_RANGE.end();
        for band in &mut extreme.octaves {
            band.amplitude_m = *OctaveConfig::AMPLITUDE_RANGE.end();
            band.perturbation = 1.0;
            band.slope_erosion = 1.0;
            band.altitude_erosion = 1.0;
            band.ridge_erosion = 1.0;
            band.sharpness = 1.0;
        }
        let field = extreme.validate().unwrap();
        for p in positions().take(100) {
            let height = field.elevation_m(p, 0.0).unwrap();
            assert!(height.is_finite() && height.abs() < extreme.height_limit_m);
        }
    }

    #[test]
    fn bounded_seeded_and_schedule_independent() {
        let config = PlanetDesignConfig::starter(42);
        let a = config.validate().unwrap();
        let b = PlanetDesignConfig {
            seed: 4_294_967_338,
            ..config.clone()
        }
        .validate()
        .unwrap();
        let mut difference = 0.0;
        for p in positions().take(100) {
            let h = a.elevation_m(p, 0.0).unwrap();
            assert!(h.is_finite() && h.abs() < config.height_limit_m);
            difference += (h - b.elevation_m(p, 0.0).unwrap()).abs();
        }
        assert!(difference > 100.0);
        let run = |n| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .unwrap()
                .install(|| a.height_distribution())
        };
        assert_eq!(run(1), run(4));
    }
}
