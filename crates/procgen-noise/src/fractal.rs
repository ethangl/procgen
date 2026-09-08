use std::{error::Error, fmt};

use procgen_core::{ScalarFieldSample3, Vec3};

use crate::gradient::gradient_noise_3d;

/// Maximum supported octave count for CPU fractal accumulation.
pub const MAX_OCTAVES: u32 = 32;

/// Octave count and frequency progression for fractal noise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OctaveConfig {
    /// Number of octaves to accumulate. Zero produces a zero sample.
    pub octaves: u32,
    /// Frequency sampled by octave zero.
    pub frequency: f32,
    /// Frequency multiplier applied after each octave.
    pub lacunarity: f32,
}

impl OctaveConfig {
    /// Validates the configuration once before repeated sampling.
    pub fn validate(self) -> Result<Validated<Self>, FractalParameterError> {
        if self.octaves > MAX_OCTAVES {
            return Err(FractalParameterError::InvalidParameter {
                name: "octaves",
                requirement: "must not exceed MAX_OCTAVES",
            });
        }
        validate_positive("frequency", self.frequency)?;
        if !self.lacunarity.is_finite() || self.lacunarity < 1.0 {
            return Err(FractalParameterError::InvalidParameter {
                name: "lacunarity",
                requirement: "must be finite and at least 1",
            });
        }
        if self
            .octaves(OctaveGain(1.0))
            .any(|octave| !octave.frequency.is_finite())
        {
            return Err(FractalParameterError::NumericalRange {
                name: "octave frequency progression",
            });
        }
        Ok(Validated(self))
    }

    fn octaves(self, gain: OctaveGain) -> Octaves {
        Octaves {
            remaining: self.octaves,
            frequency: self.frequency,
            lacunarity: self.lacunarity,
            amplitude: 1.0,
            gain: gain.0,
        }
    }
}

/// A configuration validated for repeated sampling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Validated<T>(T);

/// A validated per-sample amplitude gain for successive octaves.
///
/// Keeping gain separate from [`OctaveConfig`] lets a roughness field vary it
/// per position without repeating structural octave validation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OctaveGain(f32);

impl OctaveGain {
    pub fn new(value: f32) -> Result<Self, FractalParameterError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(FractalParameterError::InvalidParameter {
                name: "gain",
                requirement: "must be finite and in [0, 1]",
            })
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
    pub fn validate(self) -> Result<Validated<Self>, FractalParameterError> {
        self.octaves.validate()?;
        validate_positive("ridge_offset", self.ridge_offset)?;
        validate_non_negative("ridge_gain", self.ridge_gain)?;
        Ok(Validated(self))
    }
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
    pub fn validate(self) -> Result<Validated<Self>, FractalParameterError> {
        self.octaves.validate()?;
        validate_non_negative("damping", self.damping)?;
        Ok(Validated(self))
    }
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
    fn sample(self, key: u32, position: Vec3) -> ScalarFieldSample3 {
        let mut sample = gradient_noise_3d(key, position * self.frequency);
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

/// Sum of the unit starting amplitude and its gain-scaled octave amplitudes.
///
/// Fractal outputs are deliberately not normalized. This factor lets callers
/// normalize explicitly or apply the documented conservative bounds. The
/// explicit zero fold is part of the canonical CPU arithmetic.
pub fn amplitude_sum(config: Validated<OctaveConfig>, gain: OctaveGain) -> f32 {
    config
        .0
        .octaves(gain)
        .fold(0.0, |sum, octave| sum + octave.amplitude)
}

/// Accumulates plain fractional Brownian motion from [`gradient_noise_3d`].
///
/// Values are an unnormalized amplitude-weighted sum. Cubic gradient noise is
/// conservatively bounded by plus or minus [`crate::GRADIENT_NOISE_VALUE_BOUND`], so
/// this result is bounded by that value times [`amplitude_sum`]. Each octave
/// sample includes the chain-rule frequency factor in its spatial derivative
/// before accumulation.
/// Octave zero has unit amplitude; callers apply a spatially varying overall
/// amplitude by multiplying the returned sample, scaling value and derivative
/// together.
pub fn fbm_3d(
    key: u32,
    position: Vec3,
    config: Validated<OctaveConfig>,
    gain: OctaveGain,
) -> ScalarFieldSample3 {
    config
        .0
        .octaves(gain)
        .fold(ScalarFieldSample3::default(), |sum, octave| {
            sum + octave.sample(key, position) * octave.amplitude
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
/// of `abs` is defined as zero at exactly zero. Callers apply overall amplitude
/// to the returned sample.
pub fn ridged_multifractal_3d(
    key: u32,
    position: Vec3,
    config: Validated<RidgedMultifractalConfig>,
    gain: OctaveGain,
) -> ScalarFieldSample3 {
    let config = config.0;
    let mut result = ScalarFieldSample3::default();
    let mut weight = ScalarFieldSample3 {
        value: 1.0,
        derivative: Vec3::ZERO,
    };

    for octave in config.octaves.octaves(gain) {
        let sample = octave.sample(key, position);
        let absolute_derivative = if sample.value > 0.0 {
            sample.derivative
        } else if sample.value < 0.0 {
            -sample.derivative
        } else {
            Vec3::ZERO
        };
        let ridge = config.ridge_offset - sample.value.abs();
        let ridge_derivative = -absolute_derivative;
        let signal = ScalarFieldSample3 {
            value: ridge * ridge,
            derivative: ridge_derivative * (2.0 * ridge),
        };
        let weighted = ScalarFieldSample3 {
            value: signal.value * weight.value,
            derivative: signal.derivative * weight.value + weight.derivative * signal.value,
        };
        result += weighted * octave.amplitude;

        let next_weight = weighted * config.ridge_gain;
        weight = if next_weight.value > 0.0 && next_weight.value < 1.0 {
            next_weight
        } else {
            ScalarFieldSample3 {
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
/// Callers apply overall amplitude to the returned sample.
pub fn derivative_damped_fbm_3d(
    key: u32,
    position: Vec3,
    config: Validated<DerivativeDampedConfig>,
    gain: OctaveGain,
) -> ScalarFieldSample3 {
    let config = config.0;
    let mut result = ScalarFieldSample3::default();
    for octave in config.octaves.octaves(gain) {
        let attenuation = (1.0 + config.damping * result.derivative.length_squared()).recip();
        result += octave.sample(key, position) * (octave.amplitude * attenuation);
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
    use crate::{fold_seed_u64_to_u32, gradient_noise_3d};

    const SEED: u64 = 0x0123_4567_89AB_CDEF;
    const KEY: u32 = fold_seed_u64_to_u32(SEED);
    const POSITION: Vec3 = Vec3::new(0.371, -1.117, 2.053);

    fn octave_config(octaves: u32) -> OctaveConfig {
        OctaveConfig {
            octaves,
            frequency: 1.25,
            lacunarity: 2.0,
        }
    }

    fn gain() -> OctaveGain {
        OctaveGain::new(0.5).unwrap()
    }

    fn valid_octaves(octaves: u32) -> Validated<OctaveConfig> {
        octave_config(octaves).validate().unwrap()
    }

    fn valid_ridged(octaves: u32, ridge_gain: f32) -> Validated<RidgedMultifractalConfig> {
        RidgedMultifractalConfig {
            octaves: octave_config(octaves),
            ridge_offset: 1.0,
            ridge_gain,
        }
        .validate()
        .unwrap()
    }

    fn valid_damped(octaves: u32, damping: f32) -> Validated<DerivativeDampedConfig> {
        DerivativeDampedConfig {
            octaves: octave_config(octaves),
            damping,
        }
        .validate()
        .unwrap()
    }

    #[test]
    fn stable_vectors() {
        let fbm = fbm_3d(KEY, POSITION, valid_octaves(5), gain());
        let ridged = ridged_multifractal_3d(KEY, POSITION, valid_ridged(5, 2.0), gain());
        let damped = derivative_damped_fbm_3d(KEY, POSITION, valid_damped(5, 0.75), gain());

        assert_eq!(
            sample_bits(fbm),
            [0x3EA5_3A8E, 0x3F36_A3A8, 0x400F_C560, 0xC024_6DF6]
        );
        assert_eq!(
            sample_bits(ridged),
            [0x3FA8_3406, 0xBF9E_A4D2, 0xC0AD_86F0, 0x3F06_5FAE]
        );
        assert_eq!(
            sample_bits(damped),
            [0x3E7C_0C6E, 0xBF3D_220F, 0x3FC2_824C, 0xBFBB_CC53]
        );
    }

    #[test]
    fn zero_and_one_octave_cutoffs_are_exact() {
        let zero = valid_octaves(0);
        assert_eq!(amplitude_sum(zero, gain()).to_bits(), 0.0_f32.to_bits());
        assert_eq!(
            fbm_3d(KEY, POSITION, zero, gain()),
            ScalarFieldSample3::default()
        );
        assert_eq!(
            derivative_damped_fbm_3d(KEY, POSITION, valid_damped(0, 100.0), gain()),
            ScalarFieldSample3::default()
        );
        assert_eq!(
            ridged_multifractal_3d(KEY, POSITION, valid_ridged(0, 2.0), gain()),
            ScalarFieldSample3::default()
        );

        let config = octave_config(1);
        let basis = gradient_noise_3d(KEY, POSITION * config.frequency);
        let expected_fbm = ScalarFieldSample3 {
            value: basis.value,
            derivative: basis.derivative * config.frequency,
        };
        assert_eq!(
            fbm_3d(KEY, POSITION, config.validate().unwrap(), gain()),
            expected_fbm
        );
        assert_eq!(
            derivative_damped_fbm_3d(KEY, POSITION, valid_damped(1, 100.0), gain()),
            expected_fbm
        );
    }

    #[test]
    fn gain_can_vary_per_sample_without_revalidating_octave_structure() {
        let config = valid_octaves(5);
        let smooth = OctaveGain::new(0.25).unwrap();
        let rough = OctaveGain::new(0.75).unwrap();

        assert_ne!(
            fbm_3d(KEY, POSITION, config, smooth),
            fbm_3d(KEY, POSITION, config, rough)
        );
        assert_eq!(
            fbm_3d(KEY, POSITION, valid_octaves(1), smooth),
            fbm_3d(KEY, POSITION, valid_octaves(1), rough)
        );
    }

    #[test]
    fn increasing_fbm_cutoff_adds_exactly_one_octave() {
        let config = octave_config(5);
        let four_sample = fbm_3d(KEY, POSITION, valid_octaves(4), gain());
        let five_sample = fbm_3d(KEY, POSITION, config.validate().unwrap(), gain());
        let fifth_frequency = config.frequency * config.lacunarity.powi(4);
        let fifth_amplitude = 0.5_f32.powi(4);
        let fifth_basis = gradient_noise_3d(KEY, POSITION * fifth_frequency);

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

        assert_derivative_matches(STEP, TOLERANCE, |position| {
            fbm_3d(KEY, position, octaves, gain())
        });
        assert_derivative_matches(STEP, TOLERANCE, |position| {
            ridged_multifractal_3d(KEY, position, ridged, gain())
        });
    }

    #[test]
    fn zero_damping_matches_fbm_and_positive_damping_suppresses_later_octaves() {
        let plain = fbm_3d(KEY, POSITION, valid_octaves(5), gain());
        let undamped = derivative_damped_fbm_3d(KEY, POSITION, valid_damped(5, 0.0), gain());
        assert_eq!(undamped, plain);

        let first = fbm_3d(KEY, POSITION, valid_octaves(1), gain());
        let damped = derivative_damped_fbm_3d(KEY, POSITION, valid_damped(5, 4.0), gain());
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
        for invalid in [-0.1, 1.1, f32::INFINITY, f32::NAN] {
            assert!(OctaveGain::new(invalid).is_err());
        }

        let mut config = octave_config(MAX_OCTAVES);
        config.frequency = f32::MAX;
        assert!(matches!(
            config.validate(),
            Err(FractalParameterError::NumericalRange {
                name: "octave frequency progression"
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
        };
        let gain = OctaveGain::new(0.5).unwrap();
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
            fbm_3d(KEY, POSITION, octaves, gain),
            fbm_3d(fold_seed_u64_to_u32(SEED + 1), POSITION, octaves, gain)
        );

        for position in positions {
            let samples = [
                fbm_3d(KEY, position, octaves, gain),
                ridged_multifractal_3d(KEY, position, ridged, gain),
                derivative_damped_fbm_3d(KEY, position, damped, gain),
            ];
            assert_eq!(samples[0], fbm_3d(KEY, position, octaves, gain));
            assert_eq!(
                samples[1],
                ridged_multifractal_3d(KEY, position, ridged, gain)
            );
            assert_eq!(
                samples[2],
                derivative_damped_fbm_3d(KEY, position, damped, gain)
            );

            for sample in samples {
                assert!(sample.value.is_finite());
                assert!(sample.derivative.x.is_finite());
                assert!(sample.derivative.y.is_finite());
                assert!(sample.derivative.z.is_finite());
            }
            let amplitude_sum = amplitude_sum(octaves, gain);
            assert!(samples[0].value.abs() <= 2.0 * amplitude_sum);
            assert!(samples[2].value.abs() <= 2.0 * amplitude_sum);
            assert!((0.0..=amplitude_sum).contains(&samples[1].value));
        }
    }

    fn assert_derivative_matches(
        step: f32,
        tolerance: f32,
        sample: impl Copy + Fn(Vec3) -> ScalarFieldSample3,
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
