use procgen_core::{ScalarFieldSample3, Vec3};
use procgen_noise::{GRADIENT_NOISE_VALUE_BOUND, gradient_noise_3d};

// Fixed moments of the normalized basis, measured over a spatial reference
// distribution (see the relief report). Never fit these to a generated world.
pub(crate) const BASIS_STD_DEV: f32 = 0.135;
pub(crate) const ABS_MEAN: f32 = 0.110;
pub(crate) const ABS_STD_DEV: f32 = 0.078;
pub(crate) const SQUARE_MEAN: f32 = 0.0182;
pub(crate) const SQUARE_STD_DEV: f32 = 0.0226;

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
}

pub(crate) fn normalized_noise(key: u32, p: Vec3) -> ScalarFieldSample3 {
    gradient_noise_3d(key, p) * GRADIENT_NOISE_VALUE_BOUND.recip()
}

/// Exact derivative of the sharpness transform only; abs has derivative zero at its cusp.
pub(crate) fn shape(sample: ScalarFieldSample3, sharpness: f32) -> ScalarFieldSample3 {
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
    use crate::test_support::{positions, reference_noise_positions};
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
}
