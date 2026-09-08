//! Canonical CPU terrain-height evaluation.

use std::{error::Error, fmt};

use procgen_core::{
    RandomStream, ScalarFieldSample3, Vec3,
    random_streams::{
        TERRAIN_ABYSSAL_NOISE, TERRAIN_COAST_WARP_X, TERRAIN_COAST_WARP_Y, TERRAIN_COAST_WARP_Z,
        TERRAIN_DETAIL_NOISE,
    },
};
use procgen_noise::{
    DerivativeDampedConfig, FractalParameterError, OctaveConfig, OctaveGain,
    RidgedMultifractalConfig, Validated, derivative_damped_fbm_3d, fold_seed_u64_to_u32,
    ridged_multifractal_3d,
};

use crate::{
    TerrainControlBake, TerrainStampInput,
    bake::sample_controls,
    coast::{TerrainCoastConfig, TerrainCoastError, coast_taper, coast_warp},
    field::UNIT_DIRECTION_TOLERANCE,
    stamp::{
        StampCap, TerrainStampError, TerrainStampProfile, TerrainStampProfiles, stamp_contribution,
    },
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainDetailConfig {
    pub octaves: OctaveConfig,
    pub derivative_damping: f32,
    pub ridge_offset: f32,
    pub ridge_gain: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainAbyssalConfig {
    pub octaves: OctaveConfig,
    pub derivative_damping: f32,
}

/// Concrete parameters for the backend-neutral terrain-height function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainHeightConfig {
    pub detail: TerrainDetailConfig,
    pub abyssal: TerrainAbyssalConfig,
    pub coast: TerrainCoastConfig,
    pub stamps: TerrainStampProfiles,
}

impl Default for TerrainHeightConfig {
    fn default() -> Self {
        Self {
            detail: TerrainDetailConfig {
                octaves: OctaveConfig {
                    octaves: 11,
                    frequency: 64.0,
                    lacunarity: 2.0,
                },
                derivative_damping: 0.75,
                ridge_offset: 1.0,
                ridge_gain: 2.0,
            },
            abyssal: TerrainAbyssalConfig {
                octaves: OctaveConfig {
                    octaves: 6,
                    frequency: 128.0,
                    lacunarity: 2.0,
                },
                derivative_damping: 1.0,
            },
            coast: TerrainCoastConfig {
                half_width: 0.14,
                warp_frequency: 8.0,
                maximum_warp: 0.01,
            },
            stamps: TerrainStampProfiles {
                hotspot: TerrainStampProfile {
                    radius: 0.018,
                    amplitude: 0.12,
                    cap: StampCap::Cubic,
                },
                volcanic_arc: TerrainStampProfile {
                    radius: 0.0063,
                    amplitude: 0.15,
                    cap: StampCap::Quadratic,
                },
                oceanic_seamount: TerrainStampProfile {
                    radius: 0.0314,
                    amplitude: 0.08,
                    cap: StampCap::Quadratic,
                },
                oceanic_abyssal_hill: TerrainStampProfile {
                    radius: 0.0157,
                    amplitude: 0.025,
                    cap: StampCap::Cubic,
                },
            },
        }
    }
}

impl TerrainHeightConfig {
    /// Validates structural parameters once before repeated point evaluation.
    pub fn validate(self) -> Result<ValidatedTerrainHeightConfig, TerrainHeightError> {
        let detail = self.detail.damped_config().validate()?;
        let ridged = self.detail.ridged_config().validate()?;
        let abyssal = self.abyssal.validate()?;
        self.coast.validate()?;
        self.stamps.validate()?;

        Ok(ValidatedTerrainHeightConfig {
            detail,
            ridged,
            abyssal,
            coast: self.coast,
            stamps: self.stamps,
        })
    }
}

impl TerrainDetailConfig {
    fn damped_config(self) -> DerivativeDampedConfig {
        DerivativeDampedConfig {
            octaves: self.octaves,
            damping: self.derivative_damping,
        }
    }

    fn ridged_config(self) -> RidgedMultifractalConfig {
        RidgedMultifractalConfig {
            octaves: self.octaves,
            ridge_offset: self.ridge_offset,
            ridge_gain: self.ridge_gain,
        }
    }
}

impl TerrainAbyssalConfig {
    fn validate(self) -> Result<Validated<DerivativeDampedConfig>, FractalParameterError> {
        DerivativeDampedConfig {
            octaves: self.octaves,
            damping: self.derivative_damping,
        }
        .validate()
    }
}

/// Terrain-height configuration validated for repeated sampling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValidatedTerrainHeightConfig {
    detail: Validated<DerivativeDampedConfig>,
    ridged: Validated<RidgedMultifractalConfig>,
    abyssal: Validated<DerivativeDampedConfig>,
    coast: TerrainCoastConfig,
    pub(crate) stamps: TerrainStampProfiles,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainHeightError {
    Coast(TerrainCoastError),
    Stamp(TerrainStampError),
    Noise(FractalParameterError),
}

impl fmt::Display for TerrainHeightError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Coast(error) => error.fmt(formatter),
            Self::Stamp(error) => error.fmt(formatter),
            Self::Noise(error) => error.fmt(formatter),
        }
    }
}

impl Error for TerrainHeightError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Coast(error) => Some(error),
            Self::Stamp(error) => Some(error),
            Self::Noise(error) => Some(error),
        }
    }
}

impl From<FractalParameterError> for TerrainHeightError {
    fn from(error: FractalParameterError) -> Self {
        Self::Noise(error)
    }
}

impl From<TerrainCoastError> for TerrainHeightError {
    fn from(error: TerrainCoastError) -> Self {
        Self::Coast(error)
    }
}

impl From<TerrainStampError> for TerrainHeightError {
    fn from(error: TerrainStampError) -> Self {
        Self::Stamp(error)
    }
}

/// Borrowed point-evaluation inputs. Spatial indexing remains a caller concern,
/// so `stamps` contains only relevant entries in their established order.
#[derive(Clone, Copy, Debug)]
pub struct TerrainHeightInputs<'a> {
    pub direction: Vec3,
    pub controls: &'a TerrainControlBake,
    pub stamps: &'a [TerrainStampInput],
    pub noise_keys: TerrainNoiseKeys,
}

/// Evaluates canonical CPU terrain detail at one unit direction.
///
/// `stamps` must retain the stable order from [`crate::TerrainControls`], even
/// after a spatial index narrows it to stamps relevant to this point. Stamp
/// overlap is accumulated in that order. The default profiles give hotspots
/// and abyssal hills a cubic compact-support cap `(1-r^2/R^2)^3`, and volcanic
/// arcs and seamounts the sharper quadratic cap `(1-r^2/R^2)^2`. All caps and
/// their first derivatives are zero at their support edge.
///
/// The unwarped baked base controls coast proximity. It remains the authority
/// used to decide where warping and taper apply; this function does not return
/// or mutate a land/ocean classification.
/// The returned sample's `value` is normalized elevation and its derivative is
/// tangent to the unit sphere for later normal construction.
pub fn terrain_height(
    inputs: TerrainHeightInputs<'_>,
    config: ValidatedTerrainHeightConfig,
) -> ScalarFieldSample3 {
    let TerrainHeightInputs {
        direction,
        controls,
        stamps,
        noise_keys,
    } = inputs;
    assert!(direction.is_finite(), "terrain direction must be finite");
    assert!(
        (direction.length_squared() - 1.0).abs() <= UNIT_DIRECTION_TOLERANCE,
        "terrain direction must be unit length"
    );

    let original = sample_controls(controls, direction);
    let coast_taper = coast_taper(original.base_elevation, config.coast.half_width);
    let warp = coast_warp(
        direction,
        noise_keys.coast_warp,
        ScalarFieldSample3::constant(1.0) - coast_taper,
        config.coast,
    );
    let controls = sample_controls(controls, warp.direction).map(|sample| warp.pullback(sample));
    let gain = OctaveGain::new(controls.octave_gain.value)
        .expect("a terrain-control bake must preserve octave gain in [0, 1]");

    let fbm = warp.pullback(derivative_damped_fbm_3d(
        noise_keys.detail,
        warp.direction,
        config.detail,
        gain,
    ));
    let ridged = warp.pullback(ridged_multifractal_3d(
        noise_keys.detail,
        warp.direction,
        config.ridged,
        gain,
    ));
    let blended = fbm + (ridged - fbm) * controls.ridge_weight;

    let abyssal = warp.pullback(derivative_damped_fbm_3d(
        noise_keys.abyssal,
        warp.direction,
        config.abyssal,
        gain,
    ));
    let additive = controls.detail_amplitude * blended + controls.abyssal_amplitude * abyssal;
    let mut height = controls.base_elevation + coast_taper * additive;
    for stamp in stamps {
        height += stamp_contribution(direction, *stamp, config.stamps.profile(stamp.kind));
    }

    if height.value <= 0.0 || height.value >= 1.0 {
        ScalarFieldSample3 {
            value: height.value.clamp(0.0, 1.0),
            derivative: Vec3::ZERO,
        }
    } else {
        ScalarFieldSample3 {
            value: height.value,
            derivative: height.derivative - direction * height.derivative.dot(direction),
        }
    }
}

/// Noise keys derived once from the explicit terrain seed for repeated sampling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainNoiseKeys {
    detail: u32,
    abyssal: u32,
    coast_warp: [u32; 3],
}

impl TerrainNoiseKeys {
    pub fn new(seed: u64) -> Self {
        let derive =
            |stream| fold_seed_u64_to_u32(RandomStream::new(seed, stream).sample_u64(0, 0));
        Self {
            detail: derive(TERRAIN_DETAIL_NOISE),
            abyssal: derive(TERRAIN_ABYSSAL_NOISE),
            coast_warp: [
                derive(TERRAIN_COAST_WARP_X),
                derive(TERRAIN_COAST_WARP_Y),
                derive(TERRAIN_COAST_WARP_Z),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        TerrainCellControls, TerrainStampKind,
        test_support::{TERRAIN_TEST_SEED, constant_bake, height_inputs},
    };
    use procgen_cubesphere::{CubeFace, FaceCoordinates, face_to_direction};
    use procgen_tectonics::SEA_LEVEL;

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
        let default = TerrainHeightConfig::default();
        TerrainHeightConfig {
            detail: TerrainDetailConfig {
                octaves: OctaveConfig {
                    octaves: 1,
                    frequency: 2.0,
                    lacunarity: 2.0,
                },
                ..default.detail
            },
            abyssal: TerrainAbyssalConfig {
                octaves: OctaveConfig {
                    octaves: 1,
                    frequency: 3.0,
                    lacunarity: 2.0,
                },
                ..default.abyssal
            },
            coast: TerrainCoastConfig {
                maximum_warp: 0.0,
                ..default.coast
            },
            ..default
        }
    }

    fn direction() -> Vec3 {
        Vec3::new(0.371, -0.517, 0.771).normalized()
    }

    fn tangent(direction: Vec3, axis: Vec3) -> Vec3 {
        (axis - direction * axis.dot(direction)).normalized()
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
            height_inputs((direction - axis * STEP).normalized(), bake, stamps),
            config,
        );
        let after = terrain_height(
            height_inputs((direction + axis * STEP).normalized(), bake, stamps),
            config,
        );
        let sample = terrain_height(height_inputs(direction, bake, stamps), config);
        let finite_difference = (after.value - before.value) / (2.0 * STEP);
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
            position: Vec3::new(0.39, -0.50, 0.78).normalized(),
            strength: 0.7,
        };
        let stamps = [stamp];
        let sample = terrain_height(
            height_inputs(direction(), &bake, &stamps),
            TerrainHeightConfig::default().validate().unwrap(),
        );
        assert_eq!(
            [
                sample.value.to_bits(),
                sample.derivative.x.to_bits(),
                sample.derivative.y.to_bits(),
                sample.derivative.z.to_bits(),
            ],
            [0x3F24_A404, 0x3F96_1AFF, 0x3F4F_8F1D, 0xBCA8_EEA7]
        );
        assert!(sample.derivative.dot(direction()).abs() < 2.0e-6);
        assert_eq!(
            sample,
            terrain_height(
                height_inputs(direction(), &bake, &stamps),
                TerrainHeightConfig::default().validate().unwrap(),
            )
        );
        assert_ne!(
            sample,
            terrain_height(
                TerrainHeightInputs {
                    noise_keys: TerrainNoiseKeys::new(TERRAIN_TEST_SEED + 1),
                    ..height_inputs(direction(), &bake, &stamps)
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
            height_inputs(direction(), &bake, &[]),
            TerrainHeightConfig::default().validate().unwrap(),
        );
        assert_eq!(sample.value, 0.72);
        assert_eq!(sample.derivative, Vec3::ZERO);
    }

    #[test]
    fn ridge_endpoints_select_the_corresponding_noise_basis() {
        let config = one_octave_config();
        let validated = config.validate().unwrap();
        let direction = direction();
        let gain = OctaveGain::new(0.5).unwrap();
        let key = TerrainNoiseKeys::new(TERRAIN_TEST_SEED).detail;
        let fbm = derivative_damped_fbm_3d(key, direction, validated.detail, gain);
        let ridged = ridged_multifractal_3d(key, direction, validated.ridged, gain);
        for (weight, expected) in [(0.0, fbm.value), (1.0, ridged.value)] {
            let bake = constant_bake(TerrainCellControls {
                base_elevation: 0.7,
                detail_amplitude: 0.01,
                ridge_weight: weight,
                octave_gain: 0.5,
                abyssal_amplitude: 0.0,
            });
            let sample = terrain_height(height_inputs(direction, &bake, &[]), validated);
            assert_eq!(sample.value, 0.7 + 0.01 * expected);
        }
    }

    #[test]
    fn octave_gain_changes_roughness_response() {
        let default = TerrainHeightConfig::default();
        let config = TerrainHeightConfig {
            coast: TerrainCoastConfig {
                maximum_warp: 0.0,
                ..default.coast
            },
            ..default
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
        let smooth = terrain_height(height_inputs(direction(), &smooth_bake, &[]), config);
        let rough = terrain_height(height_inputs(direction(), &rough_bake, &[]), config);
        assert_ne!(smooth.value.to_bits(), rough.value.to_bits());
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
            height_inputs(direction(), &at_coast, &[]),
            config.validate().unwrap(),
        );
        assert_eq!(sample.value, SEA_LEVEL);
        assert_eq!(sample.derivative, Vec3::ZERO);

        for direction in [
            Vec3::X,
            Vec3::new(1.0, 2.0, 3.0).normalized(),
            Vec3::new(-0.7, 0.2, -0.4).normalized(),
        ] {
            let warp = coast_warp(
                direction,
                TerrainNoiseKeys::new(TERRAIN_TEST_SEED).coast_warp,
                ScalarFieldSample3::constant(1.0),
                config.coast,
            );
            assert!((warp.direction - direction).length() <= config.coast.maximum_warp);
            assert!((warp.direction.length() - 1.0).abs() <= f32::EPSILON);
        }
    }

    #[test]
    fn every_stamp_has_compact_support_and_stable_accumulation_order() {
        let direction = Vec3::Z;
        let config = TerrainHeightConfig::default();
        for kind in TerrainStampKind::ALL {
            let profile = config.stamps.profile(kind);
            let center = TerrainStampInput {
                cell: 0,
                kind,
                source_index: 0,
                position: direction,
                strength: 0.5,
            };
            assert_eq!(
                stamp_contribution(direction, center, profile).value,
                profile.amplitude * 0.5
            );
            let outside = TerrainStampInput {
                position: Vec3::new(profile.radius * 1.01, 0.0, 1.0).normalized(),
                ..center
            };
            assert_eq!(
                stamp_contribution(direction, outside, profile),
                ScalarFieldSample3::default()
            );
        }

        let bake = constant_bake(TerrainCellControls {
            base_elevation: 0.25,
            ..TerrainCellControls::default()
        });
        let stamps: Vec<_> = TerrainStampKind::ALL
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
            height + stamp_contribution(direction, *stamp, config.stamps.profile(stamp.kind)).value
        });
        assert_eq!(
            terrain_height(
                height_inputs(direction, &bake, &stamps),
                config.validate().unwrap(),
            )
            .value,
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
            terrain_height(height_inputs(left, &bake, &[]), config),
            terrain_height(height_inputs(right, &bake, &[]), config)
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
        let base_config = one_octave_config();
        let config = TerrainHeightConfig {
            coast: TerrainCoastConfig {
                maximum_warp: 0.008,
                ..base_config.coast
            },
            ..base_config
        };
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
                coast: TerrainCoastConfig {
                    half_width: 0.0,
                    ..default.coast
                },
                ..default
            },
            TerrainHeightConfig {
                coast: TerrainCoastConfig {
                    warp_frequency: f32::NAN,
                    ..default.coast
                },
                ..default
            },
            TerrainHeightConfig {
                coast: TerrainCoastConfig {
                    maximum_warp: 0.251,
                    ..default.coast
                },
                ..default
            },
            TerrainHeightConfig {
                stamps: TerrainStampProfiles {
                    hotspot: TerrainStampProfile {
                        radius: 0.0,
                        ..default.stamps.hotspot
                    },
                    ..default.stamps
                },
                ..default
            },
            TerrainHeightConfig {
                stamps: TerrainStampProfiles {
                    oceanic_seamount: TerrainStampProfile {
                        amplitude: -0.1,
                        ..default.stamps.oceanic_seamount
                    },
                    ..default.stamps
                },
                ..default
            },
            TerrainHeightConfig {
                detail: TerrainDetailConfig {
                    octaves: OctaveConfig {
                        frequency: f32::INFINITY,
                        ..default.detail.octaves
                    },
                    ..default.detail
                },
                ..default
            },
        ];

        for config in configs {
            assert!(config.validate().is_err(), "config={config:?}");
        }
    }
}
