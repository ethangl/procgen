use std::{error::Error, fmt};

use procgen_core::Vec3;

use crate::{NoiseSample3, gradient_noise_3d};

/// Maximum supported octave count for CPU fractal accumulation.
pub const MAX_OCTAVES: u32 = 32;

/// Frequency and amplitude progression for fractal noise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OctaveConfig {
    /// Number of octaves to accumulate. Zero produces a zero sample.
    pub octaves: u32,
    /// Frequency sampled by octave zero.
    pub frequency: f32,
    /// Frequency multiplier applied after each octave.
    pub lacunarity: f32,
    /// Amplitude applied to octave zero.
    pub amplitude: f32,
    /// Amplitude multiplier applied after each octave.
    pub gain: f32,
}

impl OctaveConfig {
    /// Validates the configuration once before repeated sampling.
    pub fn validate(self) -> Result<ValidatedOctaveConfig, FractalParameterError> {
        if self.octaves > MAX_OCTAVES {
            return Err(FractalParameterError::InvalidParameter {
                name: "octaves",
                requirement: "must not exceed 32",
            });
        }
        validate_positive("frequency", self.frequency)?;
        if !self.lacunarity.is_finite() || self.lacunarity < 1.0 {
            return Err(FractalParameterError::InvalidParameter {
                name: "lacunarity",
                requirement: "must be finite and at least 1",
            });
        }
        validate_non_negative("amplitude", self.amplitude)?;
        if !self.gain.is_finite() || !(0.0..=1.0).contains(&self.gain) {
            return Err(FractalParameterError::InvalidParameter {
                name: "gain",
                requirement: "must be finite and in [0, 1]",
            });
        }

        let validated = ValidatedOctaveConfig(self);
        let mut amplitude_sum = 0.0;
        for octave in validated.octaves() {
            if !octave.frequency.is_finite() {
                return Err(FractalParameterError::NumericalRange {
                    name: "octave frequency progression",
                });
            }
            amplitude_sum += octave.amplitude;
            if !amplitude_sum.is_finite() {
                return Err(FractalParameterError::NumericalRange {
                    name: "octave amplitude sum",
                });
            }
        }
        Ok(validated)
    }
}

/// An octave configuration validated for repeated sampling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValidatedOctaveConfig(OctaveConfig);

impl ValidatedOctaveConfig {
    /// Sum of the configured octave amplitudes.
    ///
    /// Fractal outputs are deliberately not normalized. This value lets a
    /// caller normalize explicitly or apply the documented conservative bounds.
    pub fn amplitude_sum(self) -> f32 {
        self.octaves().map(|octave| octave.amplitude).sum()
    }

    fn octaves(self) -> Octaves {
        Octaves {
            remaining: self.0.octaves,
            frequency: self.0.frequency,
            lacunarity: self.0.lacunarity,
            amplitude: self.0.amplitude,
            gain: self.0.gain,
        }
    }
}

/// Controls for ridged multifractal accumulation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RidgedMultifractalConfig {
    pub octaves: OctaveConfig,
    /// Offset applied before squaring the absolute gradient-noise value.
    pub ridge_offset: f32,
    /// Multiplier used to feed one octave's ridge signal into the next.
    pub ridge_gain: f32,
}

impl RidgedMultifractalConfig {
    /// Validates the configuration once before repeated sampling.
    pub fn validate(self) -> Result<ValidatedRidgedMultifractalConfig, FractalParameterError> {
        let octaves = self.octaves.validate()?;
        validate_positive("ridge_offset", self.ridge_offset)?;
        validate_non_negative("ridge_gain", self.ridge_gain)?;
        Ok(ValidatedRidgedMultifractalConfig {
            octaves,
            ridge_offset: self.ridge_offset,
            ridge_gain: self.ridge_gain,
        })
    }
}

/// A ridged multifractal configuration validated for repeated sampling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValidatedRidgedMultifractalConfig {
    octaves: ValidatedOctaveConfig,
    ridge_offset: f32,
    ridge_gain: f32,
}

/// Controls for derivative-damped fbm accumulation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DerivativeDampedConfig {
    pub octaves: OctaveConfig,
    /// Strength of attenuation from accumulated squared slope.
    pub damping: f32,
}

impl DerivativeDampedConfig {
    /// Validates the configuration once before repeated sampling.
    pub fn validate(self) -> Result<ValidatedDerivativeDampedConfig, FractalParameterError> {
        let octaves = self.octaves.validate()?;
        validate_non_negative("damping", self.damping)?;
        Ok(ValidatedDerivativeDampedConfig {
            octaves,
            damping: self.damping,
        })
    }
}

/// A derivative-damped configuration validated for repeated sampling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValidatedDerivativeDampedConfig {
    octaves: ValidatedOctaveConfig,
    damping: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FractalParameterError {
    InvalidParameter {
        name: &'static str,
        requirement: &'static str,
    },
    NumericalRange {
        name: &'static str,
    },
}

impl fmt::Display for FractalParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParameter { name, requirement } => {
                write!(formatter, "{name} {requirement}")
            }
            Self::NumericalRange { name } => {
                write!(formatter, "{name} must remain in the finite f32 range")
            }
        }
    }
}

impl Error for FractalParameterError {}

#[derive(Clone, Copy)]
struct Octave {
    frequency: f32,
    amplitude: f32,
}

impl Octave {
    fn sample(self, seed: u64, position: Vec3) -> NoiseSample3 {
        let mut sample = gradient_noise_3d(seed, position * self.frequency);
        sample.derivative = sample.derivative * self.frequency;
        sample
    }
}

struct Octaves {
    remaining: u32,
    frequency: f32,
    lacunarity: f32,
    amplitude: f32,
    gain: f32,
}

impl Iterator for Octaves {
    type Item = Octave;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        let octave = Octave {
            frequency: self.frequency,
            amplitude: self.amplitude,
        };
        self.remaining -= 1;
        self.frequency *= self.lacunarity;
        self.amplitude *= self.gain;
        Some(octave)
    }
}

/// Accumulates plain fractional Brownian motion from [`gradient_noise_3d`].
///
/// Values are an unnormalized amplitude-weighted sum. Cubic gradient noise is
/// conservatively bounded by `[-2, 2]`, so this result is bounded by twice
/// [`ValidatedOctaveConfig::amplitude_sum`]. Each octave sample includes the
/// chain-rule frequency factor in its spatial derivative before accumulation.
pub fn fbm_3d(seed: u64, position: Vec3, config: ValidatedOctaveConfig) -> NoiseSample3 {
    config
        .octaves()
        .fold(NoiseSample3::default(), |sum, octave| {
            sum + octave.sample(seed, position) * octave.amplitude
        })
}

/// Accumulates a nonnegative ridged multifractal from [`gradient_noise_3d`].
///
/// Each octave uses `(ridge_offset - abs(noise))^2`. Its contribution is
/// weighted by the previous octave's ridge signal times `ridge_gain`, clamped
/// to `[0, 1]`. The result is not normalized. Given the basis's conservative
/// `[-2, 2]` bound, the value is at most the amplitude sum multiplied by
/// `max(ridge_offset, abs(ridge_offset - 2))^2`. Derivatives propagate through
/// the absolute value, square, feedback weight, and amplitude. The derivative
/// of `abs` is defined as zero at exactly zero.
pub fn ridged_multifractal_3d(
    seed: u64,
    position: Vec3,
    config: ValidatedRidgedMultifractalConfig,
) -> NoiseSample3 {
    let mut result = NoiseSample3::default();
    let mut weight = NoiseSample3 {
        value: 1.0,
        derivative: Vec3::ZERO,
    };

    for octave in config.octaves.octaves() {
        let sample = octave.sample(seed, position);
        let absolute_derivative = if sample.value > 0.0 {
            sample.derivative
        } else if sample.value < 0.0 {
            -sample.derivative
        } else {
            Vec3::ZERO
        };
        let ridge = config.ridge_offset - sample.value.abs();
        let ridge_derivative = -absolute_derivative;
        let signal = NoiseSample3 {
            value: ridge * ridge,
            derivative: ridge_derivative * (2.0 * ridge),
        };
        let weighted = NoiseSample3 {
            value: signal.value * weight.value,
            derivative: signal.derivative * weight.value + weight.derivative * signal.value,
        };
        result += weighted * octave.amplitude;

        let next_weight = weighted * config.ridge_gain;
        weight = if next_weight.value > 0.0 && next_weight.value < 1.0 {
            next_weight
        } else {
            NoiseSample3 {
                value: next_weight.value.clamp(0.0, 1.0),
                derivative: Vec3::ZERO,
            }
        };
    }

    result
}

/// Accumulates fbm while damping higher octaves as accumulated slope grows.
///
/// Before each octave, its amplitude is divided by
/// `1 + damping * accumulated_slope_squared`. The first octave is therefore
/// unchanged. Values remain an unnormalized weighted sum, and damping never
/// increases the amplitude of an individual octave relative to plain fbm.
/// The conservative value bound is twice the octave amplitude sum. The
/// returned derivative is the accumulated slope used by the damping recurrence;
/// as is conventional for derivative-damped fbm, the spatial derivative of the
/// adaptive damping weight itself is not included.
pub fn derivative_damped_fbm_3d(
    seed: u64,
    position: Vec3,
    config: ValidatedDerivativeDampedConfig,
) -> NoiseSample3 {
    let mut result = NoiseSample3::default();
    for octave in config.octaves.octaves() {
        let attenuation = (1.0 + config.damping * result.derivative.length_squared()).recip();
        result += octave.sample(seed, position) * (octave.amplitude * attenuation);
    }
    result
}

fn validate_positive(name: &'static str, value: f32) -> Result<(), FractalParameterError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(FractalParameterError::InvalidParameter {
            name,
            requirement: "must be finite and greater than zero",
        })
    }
}

fn validate_non_negative(name: &'static str, value: f32) -> Result<(), FractalParameterError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(FractalParameterError::InvalidParameter {
            name,
            requirement: "must be finite and non-negative",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{central_difference, sample_bits};

    const SEED: u64 = 0x0123_4567_89AB_CDEF;
    const POSITION: Vec3 = Vec3::new(0.371, -1.117, 2.053);

    fn octave_config(octaves: u32) -> OctaveConfig {
        OctaveConfig {
            octaves,
            frequency: 1.25,
            lacunarity: 2.0,
            amplitude: 0.8,
            gain: 0.5,
        }
    }

    fn valid_octaves(octaves: u32) -> ValidatedOctaveConfig {
        octave_config(octaves).validate().unwrap()
    }

    fn valid_ridged(octaves: u32, ridge_gain: f32) -> ValidatedRidgedMultifractalConfig {
        RidgedMultifractalConfig {
            octaves: octave_config(octaves),
            ridge_offset: 1.0,
            ridge_gain,
        }
        .validate()
        .unwrap()
    }

    fn valid_damped(octaves: u32, damping: f32) -> ValidatedDerivativeDampedConfig {
        DerivativeDampedConfig {
            octaves: octave_config(octaves),
            damping,
        }
        .validate()
        .unwrap()
    }

    #[test]
    fn stable_vectors() {
        let fbm = fbm_3d(SEED, POSITION, valid_octaves(5));
        let ridged = ridged_multifractal_3d(SEED, POSITION, valid_ridged(5, 2.0));
        let damped = derivative_damped_fbm_3d(SEED, POSITION, valid_damped(5, 0.75));

        assert_eq!(
            sample_bits(fbm),
            [0x3E84_2ED8, 0x3F12_1C87, 0x3FE6_089A, 0xC003_8B2B]
        );
        assert_eq!(
            sample_bits(ridged),
            [0x3F86_9004, 0xBF7D_D485, 0xC08A_D25A, 0x3ED6_FF7C]
        );
        assert_eq!(
            sample_bits(damped),
            [0x3E4F_8209, 0xBF02_7974, 0x3FA8_44CE, 0xBFA7_BED1]
        );
    }

    #[test]
    fn zero_and_one_octave_cutoffs_are_exact() {
        let zero = valid_octaves(0);
        assert_eq!(fbm_3d(SEED, POSITION, zero), NoiseSample3::default());
        assert_eq!(
            derivative_damped_fbm_3d(SEED, POSITION, valid_damped(0, 100.0)),
            NoiseSample3::default()
        );
        assert_eq!(
            ridged_multifractal_3d(SEED, POSITION, valid_ridged(0, 2.0)),
            NoiseSample3::default()
        );

        let config = octave_config(1);
        let basis = gradient_noise_3d(SEED, POSITION * config.frequency);
        let expected_fbm = NoiseSample3 {
            value: basis.value * config.amplitude,
            derivative: basis.derivative * config.frequency * config.amplitude,
        };
        assert_eq!(
            fbm_3d(SEED, POSITION, config.validate().unwrap()),
            expected_fbm
        );
        assert_eq!(
            derivative_damped_fbm_3d(SEED, POSITION, valid_damped(1, 100.0)),
            expected_fbm
        );
    }

    #[test]
    fn increasing_fbm_cutoff_adds_exactly_one_octave() {
        let config = octave_config(5);
        let four_sample = fbm_3d(SEED, POSITION, valid_octaves(4));
        let five_sample = fbm_3d(SEED, POSITION, config.validate().unwrap());
        let fifth_frequency = config.frequency * config.lacunarity.powi(4);
        let fifth_amplitude = config.amplitude * config.gain.powi(4);
        let fifth_basis = gradient_noise_3d(SEED, POSITION * fifth_frequency);

        assert_eq!(
            five_sample.value.to_bits(),
            (four_sample.value + fifth_basis.value * fifth_amplitude).to_bits()
        );
        assert_eq!(
            five_sample.derivative,
            four_sample.derivative + fifth_basis.derivative * fifth_frequency * fifth_amplitude
        );
    }

    #[test]
    fn fbm_and_ridged_derivatives_match_central_difference() {
        const STEP: f32 = 5.0e-4;
        const TOLERANCE: f32 = 8.0e-3;
        let octaves = valid_octaves(4);
        let ridged = valid_ridged(4, 1.4);

        assert_derivative_matches(STEP, TOLERANCE, |position| fbm_3d(SEED, position, octaves));
        assert_derivative_matches(STEP, TOLERANCE, |position| {
            ridged_multifractal_3d(SEED, position, ridged)
        });
    }

    #[test]
    fn zero_damping_matches_fbm_and_positive_damping_suppresses_later_octaves() {
        let plain = fbm_3d(SEED, POSITION, valid_octaves(5));
        let undamped = derivative_damped_fbm_3d(SEED, POSITION, valid_damped(5, 0.0));
        assert_eq!(undamped, plain);

        let first = fbm_3d(SEED, POSITION, valid_octaves(1));
        let damped = derivative_damped_fbm_3d(SEED, POSITION, valid_damped(5, 4.0));
        assert!((damped.value - first.value).abs() < (plain.value - first.value).abs());
        assert!(
            (damped.derivative - first.derivative).length()
                < (plain.derivative - first.derivative).length()
        );
    }

    #[test]
    fn configs_reject_invalid_parameters() {
        let mut config = octave_config(4);
        config.octaves = MAX_OCTAVES + 1;
        assert!(matches!(
            config.validate(),
            Err(FractalParameterError::InvalidParameter {
                name: "octaves",
                ..
            })
        ));

        for invalid in [0.0, -1.0, f32::INFINITY, f32::NAN] {
            let mut config = octave_config(4);
            config.frequency = invalid;
            assert!(config.validate().is_err());
        }
        for invalid in [0.5, -1.0, f32::INFINITY, f32::NAN] {
            let mut config = octave_config(4);
            config.lacunarity = invalid;
            assert!(config.validate().is_err());
        }
        for invalid in [-1.0, f32::INFINITY, f32::NAN] {
            let mut config = octave_config(4);
            config.amplitude = invalid;
            assert!(config.validate().is_err());
        }
        for invalid in [-0.1, 1.1, f32::INFINITY, f32::NAN] {
            let mut config = octave_config(4);
            config.gain = invalid;
            assert!(config.validate().is_err());
        }

        let mut config = octave_config(MAX_OCTAVES);
        config.frequency = f32::MAX;
        assert!(matches!(
            config.validate(),
            Err(FractalParameterError::NumericalRange {
                name: "octave frequency progression"
            })
        ));
        config = octave_config(MAX_OCTAVES);
        config.amplitude = f32::MAX;
        config.gain = 1.0;
        assert!(matches!(
            config.validate(),
            Err(FractalParameterError::NumericalRange {
                name: "octave amplitude sum"
            })
        ));

        for invalid in [0.0, -1.0, f32::INFINITY, f32::NAN] {
            assert!(
                RidgedMultifractalConfig {
                    octaves: octave_config(4),
                    ridge_offset: invalid,
                    ridge_gain: 2.0,
                }
                .validate()
                .is_err()
            );
        }
        for invalid in [-1.0, f32::INFINITY, f32::NAN] {
            assert!(
                RidgedMultifractalConfig {
                    octaves: octave_config(4),
                    ridge_offset: 1.0,
                    ridge_gain: invalid,
                }
                .validate()
                .is_err()
            );
            assert!(
                DerivativeDampedConfig {
                    octaves: octave_config(4),
                    damping: invalid,
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn representative_results_are_deterministic_finite_and_bounded() {
        let octave_config = OctaveConfig {
            octaves: 11,
            frequency: 0.75,
            lacunarity: 2.0,
            amplitude: 1.0,
            gain: 0.5,
        };
        let octaves = octave_config.validate().unwrap();
        let ridged = RidgedMultifractalConfig {
            octaves: octave_config,
            ridge_offset: 1.0,
            ridge_gain: 2.0,
        }
        .validate()
        .unwrap();
        let damped = DerivativeDampedConfig {
            octaves: octave_config,
            damping: 1.0,
        }
        .validate()
        .unwrap();
        let positions = [
            Vec3::new(-2.75, -0.125, 1.5),
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.577_350_26, 0.577_350_26, 0.577_350_26),
            Vec3::new(8.25, -13.5, 21.75),
        ];
        assert_ne!(
            fbm_3d(SEED, POSITION, octaves),
            fbm_3d(SEED + 1, POSITION, octaves)
        );

        for position in positions {
            let samples = [
                fbm_3d(SEED, position, octaves),
                ridged_multifractal_3d(SEED, position, ridged),
                derivative_damped_fbm_3d(SEED, position, damped),
            ];
            assert_eq!(samples[0], fbm_3d(SEED, position, octaves));
            assert_eq!(samples[1], ridged_multifractal_3d(SEED, position, ridged));
            assert_eq!(samples[2], derivative_damped_fbm_3d(SEED, position, damped));

            for sample in samples {
                assert!(sample.value.is_finite());
                assert!(sample.derivative.x.is_finite());
                assert!(sample.derivative.y.is_finite());
                assert!(sample.derivative.z.is_finite());
            }
            assert!(samples[0].value.abs() <= 2.0 * octaves.amplitude_sum());
            assert!(samples[2].value.abs() <= 2.0 * octaves.amplitude_sum());
            assert!((0.0..=octaves.amplitude_sum()).contains(&samples[1].value));
        }
    }

    fn assert_derivative_matches(
        step: f32,
        tolerance: f32,
        sample: impl Copy + Fn(Vec3) -> NoiseSample3,
    ) {
        let analytic = sample(POSITION).derivative;
        let finite_difference = Vec3::new(
            central_difference(POSITION, Vec3::new(step, 0.0, 0.0), sample),
            central_difference(POSITION, Vec3::new(0.0, step, 0.0), sample),
            central_difference(POSITION, Vec3::new(0.0, 0.0, step), sample),
        );
        assert!((analytic - finite_difference).length() < tolerance);
    }
}
