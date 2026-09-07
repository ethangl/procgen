use std::{error::Error, fmt};

use procgen_core::Vec3;

use crate::{NoiseSample3, gradient_noise_3d};

/// Maximum supported octave count for CPU fractal accumulation.
pub const MAX_OCTAVES: u32 = 32;

/// Validated frequency and amplitude progression shared by the fractal variants.
///
/// Octave zero samples `position * frequency` with the configured amplitude.
/// Each following octave multiplies frequency by `lacunarity` and amplitude by
/// `gain`. Zero octaves is valid and produces [`NoiseSample3::default`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OctaveSettings {
    octaves: u32,
    frequency: f32,
    lacunarity: f32,
    amplitude: f32,
    gain: f32,
}

impl OctaveSettings {
    pub fn new(
        octaves: u32,
        frequency: f32,
        lacunarity: f32,
        amplitude: f32,
        gain: f32,
    ) -> Result<Self, FractalParameterError> {
        if octaves > MAX_OCTAVES {
            return Err(FractalParameterError::TooManyOctaves {
                requested: octaves,
                maximum: MAX_OCTAVES,
            });
        }
        validate_positive("frequency", frequency)?;
        if !lacunarity.is_finite() || lacunarity < 1.0 {
            return Err(FractalParameterError::InvalidParameter {
                name: "lacunarity",
                requirement: "must be finite and at least 1",
            });
        }
        validate_non_negative("amplitude", amplitude)?;
        if !gain.is_finite() || !(0.0..=1.0).contains(&gain) {
            return Err(FractalParameterError::InvalidParameter {
                name: "gain",
                requirement: "must be finite and in [0, 1]",
            });
        }

        let settings = Self {
            octaves,
            frequency,
            lacunarity,
            amplitude,
            gain,
        };
        settings.validate_progression()?;
        Ok(settings)
    }

    pub const fn octaves(self) -> u32 {
        self.octaves
    }

    pub const fn frequency(self) -> f32 {
        self.frequency
    }

    pub const fn lacunarity(self) -> f32 {
        self.lacunarity
    }

    pub const fn amplitude(self) -> f32 {
        self.amplitude
    }

    pub const fn gain(self) -> f32 {
        self.gain
    }

    /// Sum of the configured octave amplitudes.
    ///
    /// Fractal outputs are deliberately not normalized. This value is useful
    /// when a caller wants to normalize or reason about the conservative fbm
    /// value bound of twice this sum.
    pub fn amplitude_sum(self) -> f32 {
        let mut amplitude = self.amplitude;
        let mut sum = 0.0;
        for _ in 0..self.octaves {
            sum += amplitude;
            amplitude *= self.gain;
        }
        sum
    }

    fn validate_progression(self) -> Result<(), FractalParameterError> {
        let mut frequency = self.frequency;
        let mut amplitude = self.amplitude;
        let mut amplitude_sum = 0.0;
        for _ in 0..self.octaves {
            amplitude_sum += amplitude;
            if !amplitude_sum.is_finite() {
                return Err(FractalParameterError::AmplitudeOverflow);
            }
            amplitude *= self.gain;
        }
        for _ in 1..self.octaves {
            frequency *= self.lacunarity;
            if !frequency.is_finite() {
                return Err(FractalParameterError::FrequencyOverflow);
            }
        }
        Ok(())
    }
}

/// Validated controls for ridged multifractal accumulation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RidgedMultifractalSettings {
    octaves: OctaveSettings,
    ridge_offset: f32,
    ridge_gain: f32,
}

impl RidgedMultifractalSettings {
    pub fn new(
        octaves: OctaveSettings,
        ridge_offset: f32,
        ridge_gain: f32,
    ) -> Result<Self, FractalParameterError> {
        validate_positive("ridge_offset", ridge_offset)?;
        validate_non_negative("ridge_gain", ridge_gain)?;
        let settings = Self {
            octaves,
            ridge_offset,
            ridge_gain,
        };
        if !settings.value_upper_bound().is_finite() {
            return Err(FractalParameterError::RidgedRangeOverflow);
        }
        Ok(settings)
    }

    pub const fn octaves(self) -> OctaveSettings {
        self.octaves
    }

    pub const fn ridge_offset(self) -> f32 {
        self.ridge_offset
    }

    pub const fn ridge_gain(self) -> f32 {
        self.ridge_gain
    }

    /// Conservative upper bound for the nonnegative output value.
    pub fn value_upper_bound(self) -> f32 {
        let maximum_ridge = self.ridge_offset.max((self.ridge_offset - 2.0).abs());
        maximum_ridge * maximum_ridge * self.octaves.amplitude_sum()
    }
}

/// Validated controls for derivative-damped fbm accumulation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DerivativeDampedSettings {
    octaves: OctaveSettings,
    damping: f32,
}

impl DerivativeDampedSettings {
    pub fn new(octaves: OctaveSettings, damping: f32) -> Result<Self, FractalParameterError> {
        validate_non_negative("damping", damping)?;
        Ok(Self { octaves, damping })
    }

    pub const fn octaves(self) -> OctaveSettings {
        self.octaves
    }

    pub const fn damping(self) -> f32 {
        self.damping
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FractalParameterError {
    TooManyOctaves {
        requested: u32,
        maximum: u32,
    },
    InvalidParameter {
        name: &'static str,
        requirement: &'static str,
    },
    FrequencyOverflow,
    AmplitudeOverflow,
    RidgedRangeOverflow,
}

impl fmt::Display for FractalParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyOctaves { requested, maximum } => {
                write!(
                    formatter,
                    "octave count {requested} exceeds maximum {maximum}"
                )
            }
            Self::InvalidParameter { name, requirement } => {
                write!(formatter, "{name} {requirement}")
            }
            Self::FrequencyOverflow => {
                formatter.write_str("octave frequency progression must remain finite")
            }
            Self::AmplitudeOverflow => {
                formatter.write_str("octave amplitude sum must remain finite")
            }
            Self::RidgedRangeOverflow => {
                formatter.write_str("ridged value bound must remain finite")
            }
        }
    }
}

impl Error for FractalParameterError {}

/// Accumulates plain fractional Brownian motion from [`gradient_noise_3d`].
///
/// Values are an unnormalized amplitude-weighted sum. Cubic gradient noise is
/// conservatively bounded by `[-2, 2]`, so this result is bounded by twice
/// [`OctaveSettings::amplitude_sum`]. The returned derivative includes both
/// amplitude scaling and the chain-rule frequency factor for every octave.
pub fn fbm_3d(seed: u64, position: Vec3, settings: OctaveSettings) -> NoiseSample3 {
    accumulate_octaves(seed, position, settings, |sample, _| sample)
}

/// Accumulates a nonnegative ridged multifractal from [`gradient_noise_3d`].
///
/// Each octave uses `(ridge_offset - abs(noise))^2`. Its contribution is
/// weighted by the previous octave's ridge signal times `ridge_gain`, clamped
/// to `[0, 1]`. The result is not normalized; both the configured amplitude
/// progression and ridge controls determine its range. Derivatives propagate
/// through the absolute value, square, feedback weight, frequency, and
/// amplitude. The derivative of `abs` is defined as zero at exactly zero.
pub fn ridged_multifractal_3d(
    seed: u64,
    position: Vec3,
    settings: RidgedMultifractalSettings,
) -> NoiseSample3 {
    let mut ridge_weight = 1.0;
    let mut ridge_weight_derivative = Vec3::ZERO;

    accumulate_octaves(seed, position, settings.octaves, |sample, frequency| {
        let input_derivative = sample.derivative * frequency;
        let absolute_derivative = if sample.value > 0.0 {
            input_derivative
        } else if sample.value < 0.0 {
            -input_derivative
        } else {
            Vec3::ZERO
        };
        let ridge = settings.ridge_offset - sample.value.abs();
        let ridge_derivative = -absolute_derivative;
        let signal = ridge * ridge;
        let signal_derivative = ridge_derivative * (2.0 * ridge);
        let weighted_signal = signal * ridge_weight;
        let weighted_derivative =
            signal_derivative * ridge_weight + ridge_weight_derivative * signal;

        let next_weight = weighted_signal * settings.ridge_gain;
        if next_weight > 0.0 && next_weight < 1.0 {
            ridge_weight = next_weight;
            ridge_weight_derivative = weighted_derivative * settings.ridge_gain;
        } else {
            ridge_weight = next_weight.clamp(0.0, 1.0);
            ridge_weight_derivative = Vec3::ZERO;
        }

        NoiseSample3 {
            value: weighted_signal,
            // The common accumulator applies frequency below; undo the
            // factor already used for ridge feedback here.
            derivative: weighted_derivative * frequency.recip(),
        }
    })
}

/// Accumulates fbm while damping higher octaves as accumulated slope grows.
///
/// Before each octave, its amplitude is divided by
/// `1 + damping * accumulated_slope_squared`. The first octave is therefore
/// unchanged. Values remain an unnormalized weighted sum, and damping never
/// increases the amplitude of an individual octave relative to plain fbm.
/// The conservative value bound is therefore twice
/// [`OctaveSettings::amplitude_sum`].
/// The returned derivative is the accumulated, frequency-scaled slope used by
/// the damping recurrence; as is conventional for derivative-damped fbm, the
/// spatial derivative of the adaptive damping weight itself is not included.
pub fn derivative_damped_fbm_3d(
    seed: u64,
    position: Vec3,
    settings: DerivativeDampedSettings,
) -> NoiseSample3 {
    let mut result = NoiseSample3::default();
    let mut frequency = settings.octaves.frequency;
    let mut amplitude = settings.octaves.amplitude;

    for _ in 0..settings.octaves.octaves {
        let sample = gradient_noise_3d(seed, position * frequency);
        let attenuation = (1.0 + settings.damping * result.derivative.length_squared()).recip();
        let weighted_amplitude = amplitude * attenuation;
        result.value += weighted_amplitude * sample.value;
        result.derivative =
            result.derivative + sample.derivative * (weighted_amplitude * frequency);
        frequency *= settings.octaves.lacunarity;
        amplitude *= settings.octaves.gain;
    }

    result
}

fn accumulate_octaves(
    seed: u64,
    position: Vec3,
    settings: OctaveSettings,
    mut transform: impl FnMut(NoiseSample3, f32) -> NoiseSample3,
) -> NoiseSample3 {
    let mut result = NoiseSample3::default();
    let mut frequency = settings.frequency;
    let mut amplitude = settings.amplitude;

    for _ in 0..settings.octaves {
        // The public u64 seed reaches the lattice only through
        // gradient_noise_3d, preserving the crate's single folding location.
        let sample = gradient_noise_3d(seed, position * frequency);
        let sample = transform(sample, frequency);
        result.value += amplitude * sample.value;
        result.derivative = result.derivative + sample.derivative * (amplitude * frequency);
        frequency *= settings.lacunarity;
        amplitude *= settings.gain;
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

    const SEED: u64 = 0x0123_4567_89AB_CDEF;
    const POSITION: Vec3 = Vec3::new(0.371, -1.117, 2.053);

    fn octave_settings(octaves: u32) -> OctaveSettings {
        OctaveSettings::new(octaves, 1.25, 2.0, 0.8, 0.5).unwrap()
    }

    #[test]
    fn stable_vectors() {
        let fbm = fbm_3d(SEED, POSITION, octave_settings(5));
        let ridged = ridged_multifractal_3d(
            SEED,
            POSITION,
            RidgedMultifractalSettings::new(octave_settings(5), 1.0, 2.0).unwrap(),
        );
        let damped = derivative_damped_fbm_3d(
            SEED,
            POSITION,
            DerivativeDampedSettings::new(octave_settings(5), 0.75).unwrap(),
        );

        assert_eq!(
            sample_bits(fbm),
            [0x3E84_2ED8, 0x3F12_1C87, 0x3FE6_089A, 0xC003_8B2A]
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
        let zero = octave_settings(0);
        assert_eq!(fbm_3d(SEED, POSITION, zero), NoiseSample3::default());
        assert_eq!(
            derivative_damped_fbm_3d(
                SEED,
                POSITION,
                DerivativeDampedSettings::new(zero, 100.0).unwrap(),
            ),
            NoiseSample3::default()
        );
        assert_eq!(
            ridged_multifractal_3d(
                SEED,
                POSITION,
                RidgedMultifractalSettings::new(zero, 1.0, 2.0).unwrap(),
            ),
            NoiseSample3::default()
        );

        let settings = octave_settings(1);
        let basis = gradient_noise_3d(SEED, POSITION * settings.frequency());
        let expected_fbm = NoiseSample3 {
            value: basis.value * settings.amplitude(),
            derivative: basis.derivative * (settings.frequency() * settings.amplitude()),
        };
        assert_eq!(fbm_3d(SEED, POSITION, settings), expected_fbm);
        assert_eq!(
            derivative_damped_fbm_3d(
                SEED,
                POSITION,
                DerivativeDampedSettings::new(settings, 100.0).unwrap(),
            ),
            expected_fbm
        );
    }

    #[test]
    fn increasing_fbm_cutoff_adds_exactly_one_octave() {
        let four = octave_settings(4);
        let five = octave_settings(5);
        let four_sample = fbm_3d(SEED, POSITION, four);
        let five_sample = fbm_3d(SEED, POSITION, five);
        let fifth_frequency = five.frequency() * five.lacunarity().powi(4);
        let fifth_amplitude = five.amplitude() * five.gain().powi(4);
        let fifth_basis = gradient_noise_3d(SEED, POSITION * fifth_frequency);

        assert_eq!(
            five_sample.value.to_bits(),
            (four_sample.value + fifth_basis.value * fifth_amplitude).to_bits()
        );
        assert_eq!(
            five_sample.derivative,
            four_sample.derivative + fifth_basis.derivative * (fifth_frequency * fifth_amplitude)
        );
    }

    #[test]
    fn fbm_and_ridged_derivatives_match_central_difference() {
        const STEP: f32 = 5.0e-4;
        const TOLERANCE: f32 = 8.0e-3;
        let octaves = octave_settings(4);
        let ridged = RidgedMultifractalSettings::new(octaves, 1.0, 1.4).unwrap();

        assert_derivative_matches(STEP, TOLERANCE, |position| fbm_3d(SEED, position, octaves));
        assert_derivative_matches(STEP, TOLERANCE, |position| {
            ridged_multifractal_3d(SEED, position, ridged)
        });
    }

    #[test]
    fn zero_damping_matches_fbm_and_positive_damping_suppresses_later_octaves() {
        let one_octave = octave_settings(1);
        let five_octaves = octave_settings(5);
        let plain = fbm_3d(SEED, POSITION, five_octaves);
        let undamped = derivative_damped_fbm_3d(
            SEED,
            POSITION,
            DerivativeDampedSettings::new(five_octaves, 0.0).unwrap(),
        );
        assert_eq!(undamped, plain);

        let first = fbm_3d(SEED, POSITION, one_octave);
        let damped = derivative_damped_fbm_3d(
            SEED,
            POSITION,
            DerivativeDampedSettings::new(five_octaves, 4.0).unwrap(),
        );
        assert!((damped.value - first.value).abs() < (plain.value - first.value).abs());
        assert!(
            (damped.derivative - first.derivative).length()
                < (plain.derivative - first.derivative).length()
        );
    }

    #[test]
    fn settings_reject_invalid_parameters() {
        assert!(matches!(
            OctaveSettings::new(MAX_OCTAVES + 1, 1.0, 2.0, 1.0, 0.5),
            Err(FractalParameterError::TooManyOctaves { .. })
        ));
        for invalid in [0.0, -1.0, f32::INFINITY, f32::NAN] {
            assert!(OctaveSettings::new(1, invalid, 2.0, 1.0, 0.5).is_err());
        }
        for invalid in [0.5, -1.0, f32::INFINITY, f32::NAN] {
            assert!(OctaveSettings::new(1, 1.0, invalid, 1.0, 0.5).is_err());
        }
        for invalid in [-1.0, f32::INFINITY, f32::NAN] {
            assert!(OctaveSettings::new(1, 1.0, 2.0, invalid, 0.5).is_err());
        }
        for invalid in [-0.1, 1.1, f32::INFINITY, f32::NAN] {
            assert!(OctaveSettings::new(1, 1.0, 2.0, 1.0, invalid).is_err());
        }
        assert!(matches!(
            OctaveSettings::new(MAX_OCTAVES, f32::MAX, 2.0, 1.0, 0.5),
            Err(FractalParameterError::FrequencyOverflow)
        ));
        assert!(matches!(
            OctaveSettings::new(MAX_OCTAVES, 1.0, 2.0, f32::MAX, 1.0),
            Err(FractalParameterError::AmplitudeOverflow)
        ));

        let octaves = octave_settings(4);
        for invalid in [0.0, -1.0, f32::INFINITY, f32::NAN] {
            assert!(RidgedMultifractalSettings::new(octaves, invalid, 2.0).is_err());
        }
        assert!(matches!(
            RidgedMultifractalSettings::new(octaves, f32::MAX, 2.0),
            Err(FractalParameterError::RidgedRangeOverflow)
        ));
        for invalid in [-1.0, f32::INFINITY, f32::NAN] {
            assert!(RidgedMultifractalSettings::new(octaves, 1.0, invalid).is_err());
            assert!(DerivativeDampedSettings::new(octaves, invalid).is_err());
        }
    }

    #[test]
    fn representative_results_are_deterministic_finite_and_bounded() {
        let octaves = OctaveSettings::new(11, 0.75, 2.0, 1.0, 0.5).unwrap();
        let ridged_settings = RidgedMultifractalSettings::new(octaves, 1.0, 2.0).unwrap();
        let damped_settings = DerivativeDampedSettings::new(octaves, 1.0).unwrap();
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
                ridged_multifractal_3d(SEED, position, ridged_settings),
                derivative_damped_fbm_3d(SEED, position, damped_settings),
            ];
            assert_eq!(samples[0], fbm_3d(SEED, position, octaves));
            assert_eq!(
                samples[1],
                ridged_multifractal_3d(SEED, position, ridged_settings)
            );
            assert_eq!(
                samples[2],
                derivative_damped_fbm_3d(SEED, position, damped_settings)
            );

            for sample in samples {
                assert!(sample.value.is_finite());
                assert!(sample.derivative.x.is_finite());
                assert!(sample.derivative.y.is_finite());
                assert!(sample.derivative.z.is_finite());
            }
            assert!(samples[0].value.abs() <= 2.0 * octaves.amplitude_sum());
            assert!(samples[2].value.abs() <= 2.0 * octaves.amplitude_sum());
            assert!(samples[1].value >= 0.0);
            assert!(samples[1].value <= ridged_settings.value_upper_bound());
        }
    }

    fn assert_derivative_matches(step: f32, tolerance: f32, sample: impl Fn(Vec3) -> NoiseSample3) {
        let analytic = sample(POSITION).derivative;
        let finite_difference = Vec3::new(
            central_difference(POSITION, Vec3::new(step, 0.0, 0.0), &sample),
            central_difference(POSITION, Vec3::new(0.0, step, 0.0), &sample),
            central_difference(POSITION, Vec3::new(0.0, 0.0, step), &sample),
        );
        assert!((analytic - finite_difference).length() < tolerance);
    }

    fn central_difference(
        position: Vec3,
        offset: Vec3,
        sample: &impl Fn(Vec3) -> NoiseSample3,
    ) -> f32 {
        (sample(position + offset).value - sample(position - offset).value)
            / (2.0 * offset.length())
    }

    fn sample_bits(sample: NoiseSample3) -> [u32; 4] {
        [
            sample.value.to_bits(),
            sample.derivative.x.to_bits(),
            sample.derivative.y.to_bits(),
            sample.derivative.z.to_bits(),
        ]
    }
}
