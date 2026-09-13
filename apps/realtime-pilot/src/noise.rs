use procgen_core::{ScalarFieldSample3, Vec3};
use procgen_noise::{GRADIENT_NOISE_VALUE_BOUND, gradient_noise_3d};

use crate::field::{FieldError, validate_range};

/// Fixed work per broad-surface query.
pub const OCTAVES: usize = 6;
const LACUNARITY: f32 = 2.0;

// Fixed moments of the normalized basis, measured over a spatial reference
// distribution (see the relief report). Never fit these to a generated world.
const BASIS_STD_DEV: f32 = 0.135;
const ABS_MEAN: f32 = 0.110;
const ABS_STD_DEV: f32 = 0.078;
const SQUARE_MEAN: f32 = 0.0182;
const SQUARE_STD_DEV: f32 = 0.0226;

/// Dimensionless shape controls, except wavelength (a model length).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoiseConfig {
    pub wavelength: f32,
    pub sharpness: f32,
    pub perturbation: f32,
    pub slope_erosion: f32,
    pub altitude_erosion: f32,
    pub ridge_erosion: f32,
    /// Resolved `lfGain + lfAmplifyFeatures`; one fact, stored once.
    pub gain: f32,
}

impl NoiseConfig {
    pub const WAVELENGTH_RANGE: std::ops::RangeInclusive<f32> = 0.125..=4.0;
    pub const SHARPNESS_RANGE: std::ops::RangeInclusive<f32> = -1.0..=1.0;
    pub const GAIN_RANGE: std::ops::RangeInclusive<f32> = 0.0..=0.9;
    pub const SHAPING_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.0;
    pub(crate) fn validate(self) -> Result<(), FieldError> {
        validate_range("wavelength", self.wavelength, Self::WAVELENGTH_RANGE)?;
        validate_range("sharpness", self.sharpness, Self::SHARPNESS_RANGE)?;
        validate_range("gain", self.gain, Self::GAIN_RANGE)?;
        for (name, value) in [
            ("perturbation", self.perturbation),
            ("slope erosion", self.slope_erosion),
            ("altitude erosion", self.altitude_erosion),
            ("ridge erosion", self.ridge_erosion),
        ] {
            validate_range(name, value, Self::SHAPING_RANGE)?;
        }
        Ok(())
    }
}

/// Pilot completion of the slide fragments, not a reconstruction of NMS code.
/// The derivative accumulators are shaping signals, not a final gradient.
pub(crate) fn uber_noise(key: u32, p: Vec3, config: NoiseConfig) -> f32 {
    let mut frequency = config.wavelength.recip();
    let mut amplitude = 1.0;
    let mut damped_amplitude = 1.0;
    let mut envelope_amplitude = 1.0;
    let mut envelope = 0.0;
    let mut sum = 0.0;
    let mut slope_sum = Vec3::ZERO;
    let mut ridge_sum = Vec3::ZERO;
    let mut perturb_sum = Vec3::ZERO;

    for _ in 0..OCTAVES {
        let sample = normalized_noise(key, p * frequency + perturb_sum);
        // Deliberately the unshaped basis derivative in octave coordinates.
        let d = sample.derivative;
        let feature = shape(sample, config.sharpness).value;
        slope_sum = slope_sum + d * config.slope_erosion;
        ridge_sum = ridge_sum + d * config.ridge_erosion;
        sum += damped_amplitude * feature / (1.0 + slope_sum.length_squared());
        let altitude = sum.clamp(0.0, 1.0);
        let smooth = altitude * altitude * (3.0 - 2.0 * altitude);
        amplitude *= config.gain * (1.0 + (smooth - 1.0) * config.altitude_erosion);
        damped_amplitude =
            amplitude * (1.0 - config.ridge_erosion / (1.0 + ridge_sum.length_squared()));
        perturb_sum = perturb_sum + d * config.perturbation;
        frequency *= LACUNARITY;
        // A position-independent bound preserves damping instead of undoing it.
        envelope += envelope_amplitude;
        envelope_amplitude *= config.gain;
    }
    // Standardized features have comparable contrast, not a [-1, 1] bound.
    // A smooth transfer bounds elevation without clipping peaks or canceling
    // the position-dependent erosion weights with a local normalization.
    let relief = sum / envelope;
    relief / (1.0 + relief * relief).sqrt()
}

pub(crate) fn normalized_noise(key: u32, p: Vec3) -> ScalarFieldSample3 {
    gradient_noise_3d(key, p) * GRADIENT_NOISE_VALUE_BOUND.recip()
}

/// Exact derivative of the sharpness transform only; abs has derivative zero at its cusp.
fn shape(sample: ScalarFieldSample3, sharpness: f32) -> ScalarFieldSample3 {
    let n = sample.value;
    let base = n / BASIS_STD_DEV;
    let base_derivative = BASIS_STD_DEV.recip();
    let (target, derivative) = if sharpness >= 0.0 {
        (
            (n * n - SQUARE_MEAN) / SQUARE_STD_DEV,
            2.0 * n / SQUARE_STD_DEV,
        )
    } else {
        let sign = if n == 0.0 { 0.0 } else { n.signum() };
        ((ABS_MEAN - n.abs()) / ABS_STD_DEV, -sign / ABS_STD_DEV)
    };
    let weight = sharpness.abs();
    ScalarFieldSample3 {
        value: base + (target - base) * weight,
        derivative: sample.derivative * (base_derivative + (derivative - base_derivative) * weight),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PRESETS,
        test_support::{positions, reference_noise_positions},
    };
    use procgen_noise::fold_seed_u64_to_u32;

    #[test]
    fn calibrated_shapes_have_centered_comparable_spread() {
        // Different spatial samples and keys from the calibration measurement.
        for key in [7, 19, u32::MAX] {
            for sharpness in [-1.0, 0.0, 1.0] {
                let mut sum = 0.0_f64;
                let mut squares = 0.0_f64;
                let mut count = 0;
                for p in reference_noise_positions() {
                    let value = shape(normalized_noise(key, p), sharpness).value as f64;
                    sum += value;
                    squares += value * value;
                    count += 1;
                }
                let mean = sum / count as f64;
                let deviation = (squares / count as f64 - mean * mean).sqrt();
                // Statistical margins across independent finite spatial samples,
                // not floating-point agreement tolerances or exact moments.
                assert!(
                    mean.abs() < 0.05,
                    "key={key} sharpness={sharpness} mean={mean}"
                );
                assert!(
                    (0.9..1.1).contains(&deviation),
                    "key={key} sharpness={sharpness} deviation={deviation}"
                );
            }
        }
    }

    #[test]
    fn shaped_basis_gradient_matches_independent_central_difference() {
        let key = fold_seed_u64_to_u32(917);
        let step = 0.001;
        // f32 central differences at this step lose about 1e-4 in cancellation.
        let tolerance = 0.002;
        for p in positions().take(24) {
            for sharpness in [-0.85, 0.0, 0.7] {
                let sample = shape(normalized_noise(key, p), sharpness);
                for (axis, analytic) in [
                    (Vec3::X, sample.derivative.x),
                    (Vec3::Y, sample.derivative.y),
                    (Vec3::Z, sample.derivative.z),
                ] {
                    let hi = shape(normalized_noise(key, p + axis * step), sharpness).value;
                    let lo = shape(normalized_noise(key, p - axis * step), sharpness).value;
                    assert!((analytic - (hi - lo) / (2.0 * step)).abs() < tolerance);
                }
            }
        }
    }

    #[test]
    fn extreme_controls_remain_finite_and_bounded() {
        for sharpness in [-1.0, 0.0, 1.0] {
            for strength in [0.0, 1.0] {
                let config = NoiseConfig {
                    sharpness,
                    perturbation: strength,
                    slope_erosion: strength,
                    altitude_erosion: strength,
                    ridge_erosion: strength,
                    gain: 0.9,
                    wavelength: 0.125,
                };
                for p in positions() {
                    let value = uber_noise(19, p, config);
                    assert!(value.is_finite() && (-1.0..=1.0).contains(&value));
                }
            }
        }
    }

    #[test]
    fn each_control_changes_the_sampled_field() {
        let base = PRESETS[0].config.noise;
        let variants = [
            NoiseConfig {
                sharpness: -1.0,
                ..base
            },
            NoiseConfig {
                perturbation: 0.9,
                ..base
            },
            NoiseConfig {
                slope_erosion: 0.9,
                ..base
            },
            NoiseConfig {
                altitude_erosion: 0.9,
                ..base
            },
            NoiseConfig {
                ridge_erosion: 0.9,
                ..base
            },
            NoiseConfig { gain: 0.85, ..base },
            NoiseConfig {
                wavelength: 0.3,
                ..base
            },
        ];
        for config in variants {
            let difference: f32 = positions()
                .map(|p| (uber_noise(7, p, base) - uber_noise(7, p, config)).abs())
                .sum();
            assert!(
                difference > 0.01,
                "control had no useful effect: {config:?}"
            );
        }
    }
}
