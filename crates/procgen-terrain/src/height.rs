//! Canonical CPU terrain-height evaluation.

use std::{error::Error, fmt};

use procgen_core::{
    RandomStream, Vec3,
    random_streams::{
        TERRAIN_ABYSSAL_NOISE, TERRAIN_COAST_WARP_X, TERRAIN_COAST_WARP_Y, TERRAIN_COAST_WARP_Z,
        TERRAIN_DETAIL_NOISE,
    },
};
use procgen_noise::{
    DerivativeDampedConfig, FractalParameterError, NoiseSample3, OctaveConfig, OctaveGain,
    RidgedMultifractalConfig, Validated, derivative_damped_fbm_3d, ridged_multifractal_3d,
};

use crate::{
    TerrainCellControls, TerrainControlBake, TerrainStampInput, TerrainStampKind,
    field::UNIT_DIRECTION_TOLERANCE,
    stamp::{StampCap, TerrainStampProfile, TerrainStampProfiles, stamp_contribution},
    warp::{TerrainCoastConfig, coast_taper, coast_warp},
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
        let detail = DerivativeDampedConfig {
            octaves: self.detail.octaves,
            damping: self.detail.derivative_damping,
        }
        .validate()?;
        let ridged = RidgedMultifractalConfig {
            octaves: self.detail.octaves,
            ridge_offset: self.detail.ridge_offset,
            ridge_gain: self.detail.ridge_gain,
        }
        .validate()?;
        let abyssal = DerivativeDampedConfig {
            octaves: self.abyssal.octaves,
            damping: self.abyssal.derivative_damping,
        }
        .validate()?;

        if !self.coast.half_width.is_finite() || self.coast.half_width <= 0.0 {
            return Err(TerrainHeightError::InvalidCoastHalfWidth);
        }
        if !self.coast.warp_frequency.is_finite() || self.coast.warp_frequency <= 0.0 {
            return Err(TerrainHeightError::InvalidCoastWarpFrequency);
        }
        if !self.coast.maximum_warp.is_finite() || !(0.0..=0.25).contains(&self.coast.maximum_warp)
        {
            return Err(TerrainHeightError::InvalidMaximumCoastWarp);
        }
        for kind in TerrainStampKind::ALL {
            let profile = self.stamps.profile(kind);
            if !profile.radius.is_finite()
                || !(0.0..=2.0).contains(&profile.radius)
                || profile.radius == 0.0
                || !profile.amplitude.is_finite()
                || !(0.0..=1.0).contains(&profile.amplitude)
            {
                return Err(TerrainHeightError::InvalidStampProfile(kind));
            }
        }

        Ok(ValidatedTerrainHeightConfig {
            detail,
            ridged,
            abyssal,
            coast: self.coast,
            stamps: self.stamps,
        })
    }
}

/// Terrain-height configuration validated for repeated sampling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValidatedTerrainHeightConfig {
    detail: Validated<DerivativeDampedConfig>,
    ridged: Validated<RidgedMultifractalConfig>,
    abyssal: Validated<DerivativeDampedConfig>,
    coast: TerrainCoastConfig,
    stamps: TerrainStampProfiles,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainHeightError {
    InvalidCoastHalfWidth,
    InvalidCoastWarpFrequency,
    InvalidMaximumCoastWarp,
    InvalidStampProfile(TerrainStampKind),
    Noise(FractalParameterError),
}

impl fmt::Display for TerrainHeightError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCoastHalfWidth => {
                formatter.write_str("terrain-height coast half-width is invalid")
            }
            Self::InvalidCoastWarpFrequency => {
                formatter.write_str("terrain-height coast warp frequency is invalid")
            }
            Self::InvalidMaximumCoastWarp => {
                formatter.write_str("terrain-height maximum coast warp is invalid")
            }
            Self::InvalidStampProfile(kind) => {
                write!(
                    formatter,
                    "terrain-height {kind:?} stamp profile is invalid"
                )
            }
            Self::Noise(error) => error.fmt(formatter),
        }
    }
}

impl Error for TerrainHeightError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidCoastHalfWidth
            | Self::InvalidCoastWarpFrequency
            | Self::InvalidMaximumCoastWarp
            | Self::InvalidStampProfile(_) => None,
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
/// overlap is accumulated in that order. The default profiles give hotspots
/// and abyssal hills a cubic compact-support cap `(1-r^2/R^2)^3`, and volcanic
/// arcs and seamounts the sharper quadratic cap `(1-r^2/R^2)^2`. All caps and
/// their first derivatives are zero at their support edge.
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

    let original = TerrainCellControls::from_cube_sample(
        controls
            .sample_with_derivatives(direction)
            .expect("a finite unit direction must map to a cube face"),
    );
    let coast_taper = coast_taper(original.base_elevation, config.coast.half_width);
    let seeds = terrain_noise_seeds(seed);
    let warp = coast_warp(
        direction,
        seeds.coast_warp,
        NoiseSample3::constant(1.0) - coast_taper,
        config.coast,
    );
    let controls = warp.pullback_controls(
        controls
            .sample_with_derivatives(warp.direction)
            .expect("a warped unit direction must map to a cube face"),
    );
    let gain = OctaveGain::new(controls.octave_gain.value)
        .expect("a terrain-control bake must preserve octave gain in [0, 1]");

    let fbm = warp.pullback(derivative_damped_fbm_3d(
        seeds.detail,
        warp.direction,
        config.detail,
        gain,
    ));
    let ridged = warp.pullback(ridged_multifractal_3d(
        seeds.detail,
        warp.direction,
        config.ridged,
        gain,
    ));
    let blended = fbm + (ridged - fbm) * controls.ridge_weight;

    let abyssal = warp.pullback(derivative_damped_fbm_3d(
        seeds.abyssal,
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
        TerrainHeightSample {
            height: height.value.clamp(0.0, 1.0),
            derivative: Vec3::ZERO,
        }
    } else {
        TerrainHeightSample {
            height: height.value,
            derivative: height.derivative - direction * height.derivative.dot(direction),
        }
    }
}

#[derive(Clone, Copy)]
struct TerrainNoiseSeeds {
    detail: u64,
    abyssal: u64,
    coast_warp: [u64; 3],
}

fn terrain_noise_seeds(seed: u64) -> TerrainNoiseSeeds {
    let derive = |stream| RandomStream::new(seed, stream).sample_u64(0, 0);
    TerrainNoiseSeeds {
        detail: derive(TERRAIN_DETAIL_NOISE),
        abyssal: derive(TERRAIN_ABYSSAL_NOISE),
        coast_warp: [
            derive(TERRAIN_COAST_WARP_X),
            derive(TERRAIN_COAST_WARP_Y),
            derive(TERRAIN_COAST_WARP_Z),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_cubesphere::{CubeFace, FaceCoordinates, face_to_direction};
    use procgen_tectonics::SEA_LEVEL;

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
            position: Vec3::new(0.39, -0.50, 0.78).normalized(),
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
            [0x3F24_A404, 0x3F96_1AFF, 0x3F4F_8F1D, 0xBCA8_EEA7]
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
        let seed = terrain_noise_seeds(SEED).detail;
        let fbm = derivative_damped_fbm_3d(seed, direction, validated.detail, gain);
        let ridged = ridged_multifractal_3d(seed, direction, validated.ridged, gain);
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
                terrain_noise_seeds(SEED).coast_warp,
                NoiseSample3::constant(1.0),
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
            height + stamp_contribution(direction, *stamp, config.stamps.profile(stamp.kind)).value
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
