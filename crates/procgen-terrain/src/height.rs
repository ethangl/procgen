//! Canonical CPU terrain-height evaluation.

use std::{error::Error, fmt};

use procgen_core::Vec3;
use procgen_noise::{
    DerivativeDampedConfig, FractalParameterError, NoiseSample3, OctaveConfig, OctaveGain,
    RidgedMultifractalConfig, Validated, derivative_damped_fbm_3d, gradient_noise_3d,
    ridged_multifractal_3d,
};
use procgen_tectonics::SEA_LEVEL;

use crate::{TerrainCellControls, TerrainControlBake, TerrainStampInput, TerrainStampKind};

const UNIT_DIRECTION_TOLERANCE: f32 = 2.0e-5;
const WARP_COMPONENT_BOUND: f32 = 3.464_101_6; // 2 * sqrt(3)
const WARP_SEEDS: [u64; 3] = [
    0x5741_5250_5F58_0001,
    0x5741_5250_5F59_0002,
    0x5741_5250_5F5A_0003,
];
const ABYSSAL_SEED: u64 = 0x4142_5953_5341_4C01;

/// Radius and maximum normalized-height contribution for one stamp kind.
///
/// Radius is chord distance on the unit sphere and must be in `(0, 2]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainStampProfile {
    pub radius: f32,
    pub amplitude: f32,
}

/// Concrete parameters for the backend-neutral terrain-height function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainHeightConfig {
    pub detail_octaves: OctaveConfig,
    pub derivative_damping: f32,
    pub ridge_offset: f32,
    pub ridge_gain: f32,
    pub abyssal_octaves: OctaveConfig,
    pub abyssal_derivative_damping: f32,
    /// Half-width of the normalized-elevation band affected by coast behavior.
    pub coast_half_width: f32,
    /// Frequency of the three scalar fields forming the tangent-space warp.
    pub coast_warp_frequency: f32,
    /// Maximum pre-normalization tangent displacement on the unit sphere.
    pub maximum_coast_warp: f32,
    pub hotspot: TerrainStampProfile,
    pub volcanic_arc: TerrainStampProfile,
    pub oceanic_seamount: TerrainStampProfile,
    pub oceanic_abyssal_hill: TerrainStampProfile,
}

impl Default for TerrainHeightConfig {
    fn default() -> Self {
        Self {
            detail_octaves: OctaveConfig {
                octaves: 11,
                frequency: 64.0,
                lacunarity: 2.0,
            },
            derivative_damping: 0.75,
            ridge_offset: 1.0,
            ridge_gain: 2.0,
            abyssal_octaves: OctaveConfig {
                octaves: 6,
                frequency: 128.0,
                lacunarity: 2.0,
            },
            abyssal_derivative_damping: 1.0,
            coast_half_width: 0.14,
            coast_warp_frequency: 8.0,
            maximum_coast_warp: 0.01,
            hotspot: TerrainStampProfile {
                radius: 0.018,
                amplitude: 0.12,
            },
            volcanic_arc: TerrainStampProfile {
                radius: 0.0063,
                amplitude: 0.15,
            },
            oceanic_seamount: TerrainStampProfile {
                radius: 0.0314,
                amplitude: 0.08,
            },
            oceanic_abyssal_hill: TerrainStampProfile {
                radius: 0.0157,
                amplitude: 0.025,
            },
        }
    }
}

impl TerrainHeightConfig {
    /// Validates structural parameters once before repeated point evaluation.
    pub fn validate(self) -> Result<ValidatedTerrainHeightConfig, TerrainHeightError> {
        let detail = DerivativeDampedConfig {
            octaves: self.detail_octaves,
            damping: self.derivative_damping,
        }
        .validate()?;
        let ridged = RidgedMultifractalConfig {
            octaves: self.detail_octaves,
            ridge_offset: self.ridge_offset,
            ridge_gain: self.ridge_gain,
        }
        .validate()?;
        let abyssal = DerivativeDampedConfig {
            octaves: self.abyssal_octaves,
            damping: self.abyssal_derivative_damping,
        }
        .validate()?;

        if !self.coast_half_width.is_finite() || self.coast_half_width <= 0.0 {
            return Err(TerrainHeightError::InvalidParameter("coast_half_width"));
        }
        if !self.coast_warp_frequency.is_finite() || self.coast_warp_frequency <= 0.0 {
            return Err(TerrainHeightError::InvalidParameter("coast_warp_frequency"));
        }
        if !self.maximum_coast_warp.is_finite() || !(0.0..=0.25).contains(&self.maximum_coast_warp)
        {
            return Err(TerrainHeightError::InvalidParameter("maximum_coast_warp"));
        }
        for (name, profile) in [
            ("hotspot", self.hotspot),
            ("volcanic_arc", self.volcanic_arc),
            ("oceanic_seamount", self.oceanic_seamount),
            ("oceanic_abyssal_hill", self.oceanic_abyssal_hill),
        ] {
            if !profile.radius.is_finite()
                || !(0.0..=2.0).contains(&profile.radius)
                || profile.radius == 0.0
                || !profile.amplitude.is_finite()
                || !(0.0..=1.0).contains(&profile.amplitude)
            {
                return Err(TerrainHeightError::InvalidParameter(name));
            }
        }

        Ok(ValidatedTerrainHeightConfig {
            config: self,
            detail,
            ridged,
            abyssal,
        })
    }
}

/// Terrain-height configuration validated for repeated sampling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValidatedTerrainHeightConfig {
    config: TerrainHeightConfig,
    detail: Validated<DerivativeDampedConfig>,
    ridged: Validated<RidgedMultifractalConfig>,
    abyssal: Validated<DerivativeDampedConfig>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainHeightError {
    InvalidParameter(&'static str),
    Noise(FractalParameterError),
}

impl fmt::Display for TerrainHeightError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParameter(name) => write!(formatter, "terrain-height {name} is invalid"),
            Self::Noise(error) => error.fmt(formatter),
        }
    }
}

impl Error for TerrainHeightError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidParameter(_) => None,
            Self::Noise(error) => Some(error),
        }
    }
}

impl From<FractalParameterError> for TerrainHeightError {
    fn from(error: FractalParameterError) -> Self {
        Self::Noise(error)
    }
}

/// Normalized elevation and its tangent-space derivative at the input direction.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TerrainHeightSample {
    pub height: f32,
    pub derivative: Vec3,
}

/// Borrowed point-evaluation inputs. Spatial indexing remains a caller concern,
/// so `stamps` contains only relevant entries in their established order.
#[derive(Clone, Copy, Debug)]
pub struct TerrainHeightInputs<'a> {
    pub direction: Vec3,
    pub controls: &'a TerrainControlBake,
    pub stamps: &'a [TerrainStampInput],
    pub seed: u64,
}

/// Evaluates canonical CPU terrain detail at one unit direction.
///
/// `stamps` must retain the stable order from [`crate::TerrainControls`], even
/// after a spatial index narrows it to stamps relevant to this point. Stamp
/// overlap is accumulated in that order. Hotspots and abyssal hills use a
/// cubic compact-support cap `(1-r^2/R^2)^3`; volcanic arcs and seamounts use
/// the sharper quadratic cap `(1-r^2/R^2)^2`. All profiles and their first
/// derivatives are zero at their support edge.
///
/// The unwarped baked base controls coast proximity. It remains the authority
/// used to decide where warping and taper apply; this function does not return
/// or mutate a land/ocean classification.
pub fn terrain_height(
    inputs: TerrainHeightInputs<'_>,
    config: ValidatedTerrainHeightConfig,
) -> TerrainHeightSample {
    let TerrainHeightInputs {
        direction,
        controls,
        stamps,
        seed,
    } = inputs;
    assert!(direction.is_finite(), "terrain direction must be finite");
    assert!(
        (direction.length_squared() - 1.0).abs() <= UNIT_DIRECTION_TOLERANCE,
        "terrain direction must be unit length"
    );

    let original = controls
        .sample_with_derivatives(direction)
        .expect("a finite unit direction must map to a cube face");
    let (coast_taper, coast_taper_derivative) = coast_taper(
        original.values[0],
        original.derivatives[0],
        config.config.coast_half_width,
    );
    let warp = coast_warp(
        direction,
        seed,
        1.0 - coast_taper,
        -coast_taper_derivative,
        config.config.coast_warp_frequency,
        config.config.maximum_coast_warp,
    );
    let sampled = controls
        .sample_with_derivatives(warp.direction)
        .expect("a warped unit direction must map to a cube face");
    let values = TerrainCellControls::from_channels(sampled.values);
    let control_derivatives = sampled.derivatives.map(|value| warp.pullback(value));
    let gain = OctaveGain::new(values.octave_gain)
        .expect("a terrain-control bake must preserve octave gain in [0, 1]");

    let fbm = pullback_noise(
        derivative_damped_fbm_3d(seed, warp.direction, config.detail, gain),
        warp,
    );
    let ridged = pullback_noise(
        ridged_multifractal_3d(seed, warp.direction, config.ridged, gain),
        warp,
    );
    let ridge_weight = values.ridge_weight;
    let blended = NoiseSample3 {
        value: fbm.value + (ridged.value - fbm.value) * ridge_weight,
        derivative: fbm.derivative
            + (ridged.derivative - fbm.derivative) * ridge_weight
            + control_derivatives[2] * (ridged.value - fbm.value),
    };
    let detail = NoiseSample3 {
        value: values.detail_amplitude * blended.value,
        derivative: blended.derivative * values.detail_amplitude
            + control_derivatives[1] * blended.value,
    };

    let abyssal = pullback_noise(
        derivative_damped_fbm_3d(seed ^ ABYSSAL_SEED, warp.direction, config.abyssal, gain),
        warp,
    );
    let abyssal = NoiseSample3 {
        value: values.abyssal_amplitude * abyssal.value,
        derivative: abyssal.derivative * values.abyssal_amplitude
            + control_derivatives[4] * abyssal.value,
    };
    let additive = detail + abyssal;

    let mut result = TerrainHeightSample {
        height: values.base_elevation + coast_taper * additive.value,
        derivative: control_derivatives[0]
            + additive.derivative * coast_taper
            + coast_taper_derivative * additive.value,
    };
    for stamp in stamps {
        let contribution = stamp_contribution(direction, *stamp, config.config);
        result.height += contribution.value;
        result.derivative = result.derivative + contribution.derivative;
    }

    if result.height <= 0.0 || result.height >= 1.0 {
        result.height = result.height.clamp(0.0, 1.0);
        result.derivative = Vec3::ZERO;
    } else {
        result.derivative = result.derivative - direction * result.derivative.dot(direction);
    }
    result
}

#[derive(Clone, Copy)]
struct DomainWarp {
    direction: Vec3,
    source_direction: Vec3,
    raw: Vec3,
    raw_derivatives: [Vec3; 3],
    tangent: Vec3,
    scale: f32,
    scale_derivative: Vec3,
    inverse_length: f32,
}

impl DomainWarp {
    fn pullback(self, derivative: Vec3) -> Vec3 {
        let normalized =
            (derivative - self.direction * derivative.dot(self.direction)) * self.inverse_length;
        let jacobian_transpose = transpose_product(self.raw_derivatives, normalized);
        let raw_dot_source = self.raw.dot(self.source_direction);
        let projection_derivative =
            transpose_product(self.raw_derivatives, self.source_direction) + self.raw;
        normalized
            + self.scale_derivative * normalized.dot(self.tangent)
            + (jacobian_transpose
                - normalized * raw_dot_source
                - projection_derivative * normalized.dot(self.source_direction))
                * self.scale
    }
}

fn coast_warp(
    direction: Vec3,
    seed: u64,
    weight: f32,
    weight_derivative: Vec3,
    frequency: f32,
    maximum: f32,
) -> DomainWarp {
    let samples = WARP_SEEDS.map(|stream| gradient_noise_3d(seed ^ stream, direction * frequency));
    let raw = Vec3::new(samples[0].value, samples[1].value, samples[2].value);
    let raw_derivatives = samples.map(|sample| sample.derivative * frequency);
    let tangent = raw - direction * raw.dot(direction);
    let maximum_scale = maximum / WARP_COMPONENT_BOUND;
    let scale = maximum_scale * weight;
    let scale_derivative = weight_derivative * maximum_scale;
    let displaced = direction + tangent * scale;
    let inverse_length = displaced.length().recip();
    DomainWarp {
        direction: displaced * inverse_length,
        source_direction: direction,
        raw,
        raw_derivatives,
        tangent,
        scale,
        scale_derivative,
        inverse_length,
    }
}

fn transpose_product(rows: [Vec3; 3], vector: Vec3) -> Vec3 {
    rows[0] * vector.x + rows[1] * vector.y + rows[2] * vector.z
}

fn pullback_noise(sample: NoiseSample3, warp: DomainWarp) -> NoiseSample3 {
    NoiseSample3 {
        value: sample.value,
        derivative: warp.pullback(sample.derivative),
    }
}

fn coast_taper(base: f32, derivative: Vec3, half_width: f32) -> (f32, Vec3) {
    let signed = base - SEA_LEVEL;
    let distance = signed.abs();
    if distance >= half_width {
        return (1.0, Vec3::ZERO);
    }
    if distance == 0.0 {
        return (0.0, Vec3::ZERO);
    }
    let t = distance / half_width;
    let value = t * t * (3.0 - 2.0 * t);
    let sign = if signed > 0.0 { 1.0 } else { -1.0 };
    let slope = 6.0 * t * (1.0 - t) / half_width;
    (value, derivative * (sign * slope))
}

fn stamp_contribution(
    direction: Vec3,
    stamp: TerrainStampInput,
    config: TerrainHeightConfig,
) -> NoiseSample3 {
    let (profile, power) = match stamp.kind {
        TerrainStampKind::Hotspot => (config.hotspot, 3),
        TerrainStampKind::VolcanicArc => (config.volcanic_arc, 2),
        TerrainStampKind::OceanicSeamount => (config.oceanic_seamount, 2),
        TerrainStampKind::OceanicAbyssalHill => (config.oceanic_abyssal_hill, 3),
    };
    let center = stamp.position.normalized();
    let displacement = direction - center;
    let normalized_squared = displacement.length_squared() / (profile.radius * profile.radius);
    if normalized_squared >= 1.0 {
        return NoiseSample3::default();
    }
    let support = 1.0 - normalized_squared;
    let (shape, shape_slope) = if power == 2 {
        (support * support, 2.0 * support)
    } else {
        (support * support * support, 3.0 * support * support)
    };
    let amplitude = profile.amplitude * stamp.strength;
    NoiseSample3 {
        value: amplitude * shape,
        derivative: displacement
            * (-2.0 * amplitude * shape_slope / (profile.radius * profile.radius)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_cubesphere::{CubeFace, FaceCoordinates, face_to_direction};

    const SEED: u64 = 0x0123_4567_89AB_CDEF;

    fn constant_bake(controls: TerrainCellControls) -> TerrainControlBake {
        let texel = controls.to_channels();
        TerrainControlBake::from_face_texels(4, std::array::from_fn(|_| vec![texel; 16])).unwrap()
    }

    fn varying_bake() -> TerrainControlBake {
        let resolution = 16;
        let faces = CubeFace::ALL.map(|_face| {
            (0..resolution)
                .flat_map(|y| {
                    (0..resolution).map(move |x| {
                        let u = 2.0 * (x as f32 + 0.5) / resolution as f32 - 1.0;
                        let v = 2.0 * (y as f32 + 0.5) / resolution as f32 - 1.0;
                        let base = SEA_LEVEL + 0.02 + 0.01 * u - 0.015 * v;
                        TerrainCellControls {
                            base_elevation: base,
                            detail_amplitude: 0.015 + 0.004 * u,
                            ridge_weight: 0.35 + 0.1 * v,
                            octave_gain: 0.45 + 0.05 * u,
                            abyssal_amplitude: 0.008 - 0.002 * v,
                        }
                        .to_channels()
                    })
                })
                .collect()
        });
        TerrainControlBake::from_face_texels(resolution, faces).unwrap()
    }

    fn one_octave_config() -> TerrainHeightConfig {
        TerrainHeightConfig {
            detail_octaves: OctaveConfig {
                octaves: 1,
                frequency: 2.0,
                lacunarity: 2.0,
            },
            abyssal_octaves: OctaveConfig {
                octaves: 1,
                frequency: 3.0,
                lacunarity: 2.0,
            },
            maximum_coast_warp: 0.0,
            ..TerrainHeightConfig::default()
        }
    }

    fn direction() -> Vec3 {
        Vec3::new(0.371, -0.517, 0.771).normalized()
    }

    fn tangent(direction: Vec3, axis: Vec3) -> Vec3 {
        (axis - direction * axis.dot(direction)).normalized()
    }

    fn inputs<'a>(
        direction: Vec3,
        controls: &'a TerrainControlBake,
        stamps: &'a [TerrainStampInput],
    ) -> TerrainHeightInputs<'a> {
        TerrainHeightInputs {
            direction,
            controls,
            stamps,
            seed: SEED,
        }
    }

    fn assert_directional_derivative(
        bake: &TerrainControlBake,
        stamps: &[TerrainStampInput],
        config: ValidatedTerrainHeightConfig,
        direction: Vec3,
        axis: Vec3,
        tolerance: f32,
    ) {
        const STEP: f32 = 1.0e-4;
        let axis = tangent(direction, axis);
        let before = terrain_height(
            inputs((direction - axis * STEP).normalized(), bake, stamps),
            config,
        );
        let after = terrain_height(
            inputs((direction + axis * STEP).normalized(), bake, stamps),
            config,
        );
        let sample = terrain_height(inputs(direction, bake, stamps), config);
        let finite_difference = (after.height - before.height) / (2.0 * STEP);
        assert!(
            (sample.derivative.dot(axis) - finite_difference).abs() < tolerance,
            "analytic={}, finite={finite_difference}, sample={sample:?}",
            sample.derivative.dot(axis)
        );
    }

    #[test]
    fn pinned_height_vectors_are_stable_and_seeded() {
        let bake = constant_bake(TerrainCellControls {
            base_elevation: 0.63,
            detail_amplitude: 0.025,
            ridge_weight: 0.4,
            octave_gain: 0.55,
            abyssal_amplitude: 0.01,
        });
        let stamp = TerrainStampInput {
            cell: 0,
            kind: TerrainStampKind::OceanicAbyssalHill,
            source_index: 4,
            position: Vec3::new(0.39, -0.50, 0.78),
            strength: 0.7,
        };
        let stamps = [stamp];
        let sample = terrain_height(
            inputs(direction(), &bake, &stamps),
            TerrainHeightConfig::default().validate().unwrap(),
        );
        assert_eq!(
            [
                sample.height.to_bits(),
                sample.derivative.x.to_bits(),
                sample.derivative.y.to_bits(),
                sample.derivative.z.to_bits(),
            ],
            [0x3F24_AA13, 0xC121_8D33, 0x4071_C6EC, 0x40EC_89AB]
        );
        assert!(sample.derivative.dot(direction()).abs() < 2.0e-6);
        assert_eq!(
            sample,
            terrain_height(
                inputs(direction(), &bake, &stamps),
                TerrainHeightConfig::default().validate().unwrap(),
            )
        );
        assert_ne!(
            sample,
            terrain_height(
                TerrainHeightInputs {
                    seed: SEED + 1,
                    ..inputs(direction(), &bake, &stamps)
                },
                TerrainHeightConfig::default().validate().unwrap(),
            )
        );
    }

    #[test]
    fn neutral_controls_return_the_baked_base() {
        let bake = constant_bake(TerrainCellControls {
            base_elevation: 0.72,
            detail_amplitude: 0.0,
            ridge_weight: 1.0,
            octave_gain: 1.0,
            abyssal_amplitude: 0.0,
        });
        let sample = terrain_height(
            inputs(direction(), &bake, &[]),
            TerrainHeightConfig::default().validate().unwrap(),
        );
        assert_eq!(sample.height, 0.72);
        assert_eq!(sample.derivative, Vec3::ZERO);
    }

    #[test]
    fn ridge_endpoints_select_the_corresponding_noise_basis() {
        let config = one_octave_config();
        let validated = config.validate().unwrap();
        let direction = direction();
        let gain = OctaveGain::new(0.5).unwrap();
        let fbm = derivative_damped_fbm_3d(SEED, direction, validated.detail, gain);
        let ridged = ridged_multifractal_3d(SEED, direction, validated.ridged, gain);
        for (weight, expected) in [(0.0, fbm.value), (1.0, ridged.value)] {
            let bake = constant_bake(TerrainCellControls {
                base_elevation: 0.7,
                detail_amplitude: 0.01,
                ridge_weight: weight,
                octave_gain: 0.5,
                abyssal_amplitude: 0.0,
            });
            let sample = terrain_height(inputs(direction, &bake, &[]), validated);
            assert_eq!(sample.height, 0.7 + 0.01 * expected);
        }
    }

    #[test]
    fn octave_gain_changes_roughness_response() {
        let config = TerrainHeightConfig {
            maximum_coast_warp: 0.0,
            ..TerrainHeightConfig::default()
        }
        .validate()
        .unwrap();
        let make_bake = |gain| {
            constant_bake(TerrainCellControls {
                base_elevation: 0.7,
                detail_amplitude: 0.02,
                ridge_weight: 0.25,
                octave_gain: gain,
                abyssal_amplitude: 0.0,
            })
        };
        let smooth_bake = make_bake(0.0);
        let rough_bake = make_bake(0.85);
        let smooth = terrain_height(inputs(direction(), &smooth_bake, &[]), config);
        let rough = terrain_height(inputs(direction(), &rough_bake, &[]), config);
        assert_ne!(smooth.height.to_bits(), rough.height.to_bits());
        assert_ne!(smooth.derivative, rough.derivative);
    }

    #[test]
    fn coast_taper_reaches_zero_and_warp_stays_within_its_tangent_bound() {
        let config = TerrainHeightConfig::default();
        let at_coast = constant_bake(TerrainCellControls {
            base_elevation: SEA_LEVEL,
            detail_amplitude: 1.0,
            ridge_weight: 0.0,
            octave_gain: 0.5,
            abyssal_amplitude: 1.0,
        });
        let sample = terrain_height(
            inputs(direction(), &at_coast, &[]),
            config.validate().unwrap(),
        );
        assert_eq!(sample.height, SEA_LEVEL);
        assert_eq!(sample.derivative, Vec3::ZERO);

        for direction in [
            Vec3::X,
            Vec3::new(1.0, 2.0, 3.0).normalized(),
            Vec3::new(-0.7, 0.2, -0.4).normalized(),
        ] {
            let warp = coast_warp(
                direction,
                SEED,
                1.0,
                Vec3::ZERO,
                config.coast_warp_frequency,
                config.maximum_coast_warp,
            );
            assert!(warp.tangent.dot(direction).abs() < 2.0e-7);
            assert!(warp.tangent.length() * warp.scale <= config.maximum_coast_warp);
            assert!((warp.direction.length() - 1.0).abs() <= f32::EPSILON);
        }
    }

    #[test]
    fn every_stamp_has_compact_support_and_stable_accumulation_order() {
        let direction = Vec3::Z;
        let config = TerrainHeightConfig::default();
        for kind in [
            TerrainStampKind::Hotspot,
            TerrainStampKind::VolcanicArc,
            TerrainStampKind::OceanicSeamount,
            TerrainStampKind::OceanicAbyssalHill,
        ] {
            let profile = match kind {
                TerrainStampKind::Hotspot => config.hotspot,
                TerrainStampKind::VolcanicArc => config.volcanic_arc,
                TerrainStampKind::OceanicSeamount => config.oceanic_seamount,
                TerrainStampKind::OceanicAbyssalHill => config.oceanic_abyssal_hill,
            };
            let center = TerrainStampInput {
                cell: 0,
                kind,
                source_index: 0,
                position: direction,
                strength: 0.5,
            };
            assert_eq!(
                stamp_contribution(direction, center, config).value,
                profile.amplitude * 0.5
            );
            let outside = TerrainStampInput {
                position: Vec3::new(profile.radius * 1.01, 0.0, 1.0).normalized(),
                ..center
            };
            assert_eq!(
                stamp_contribution(direction, outside, config),
                NoiseSample3::default()
            );
        }

        let bake = constant_bake(TerrainCellControls {
            base_elevation: 0.25,
            ..TerrainCellControls::default()
        });
        let stamps: Vec<_> = [
            TerrainStampKind::Hotspot,
            TerrainStampKind::VolcanicArc,
            TerrainStampKind::OceanicSeamount,
            TerrainStampKind::OceanicAbyssalHill,
        ]
        .into_iter()
        .enumerate()
        .map(|(source_index, kind)| TerrainStampInput {
            cell: 0,
            kind,
            source_index,
            position: direction,
            strength: 0.1,
        })
        .collect();
        let expected = stamps.iter().fold(0.25, |height, stamp| {
            height + stamp_contribution(direction, *stamp, config).value
        });
        assert_eq!(
            terrain_height(
                inputs(direction, &bake, &stamps),
                config.validate().unwrap(),
            )
            .height,
            expected
        );
    }

    #[test]
    fn evaluation_depends_on_direction_not_cube_face_provenance() {
        let bake = varying_bake();
        let config = one_octave_config().validate().unwrap();
        let left = face_to_direction(FaceCoordinates {
            face: CubeFace::PositiveX,
            u: -1.0,
            v: 0.25,
        })
        .unwrap();
        let right = face_to_direction(FaceCoordinates {
            face: CubeFace::PositiveZ,
            u: 1.0,
            v: 0.25,
        })
        .unwrap();
        assert_eq!(left, right);
        assert_eq!(
            terrain_height(inputs(left, &bake, &[]), config),
            terrain_height(inputs(right, &bake, &[]), config)
        );
    }

    #[test]
    fn derivatives_include_noise_controls_warp_and_stamps() {
        let direction = face_to_direction(FaceCoordinates {
            face: CubeFace::PositiveZ,
            u: 0.17,
            v: -0.21,
        })
        .unwrap();
        let mut config = one_octave_config();
        config.maximum_coast_warp = 0.008;
        let stamp = TerrainStampInput {
            cell: 0,
            kind: TerrainStampKind::Hotspot,
            source_index: 0,
            position: (direction + tangent(direction, Vec3::X) * 0.005).normalized(),
            strength: 0.2,
        };
        let bake = varying_bake();
        let config = config.validate().unwrap();
        assert_directional_derivative(&bake, &[stamp], config, direction, Vec3::X, 2.5e-2);
        assert_directional_derivative(&bake, &[stamp], config, direction, Vec3::Y, 2.5e-2);
    }

    #[test]
    fn invalid_configs_are_rejected() {
        let default = TerrainHeightConfig::default();
        let configs = [
            TerrainHeightConfig {
                coast_half_width: 0.0,
                ..default
            },
            TerrainHeightConfig {
                coast_warp_frequency: f32::NAN,
                ..default
            },
            TerrainHeightConfig {
                maximum_coast_warp: 0.251,
                ..default
            },
            TerrainHeightConfig {
                hotspot: TerrainStampProfile {
                    radius: 0.0,
                    ..default.hotspot
                },
                ..default
            },
            TerrainHeightConfig {
                oceanic_seamount: TerrainStampProfile {
                    amplitude: -0.1,
                    ..default.oceanic_seamount
                },
                ..default
            },
            TerrainHeightConfig {
                detail_octaves: OctaveConfig {
                    frequency: f32::INFINITY,
                    ..default.detail_octaves
                },
                ..default
            },
        ];

        for config in configs {
            assert!(config.validate().is_err(), "config={config:?}");
        }
    }
}
