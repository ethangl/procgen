//! Canonical fixed-level CPU terrain-tile generation.

use std::{error::Error, fmt};

use procgen_core::{ScalarFieldSample3, Vec3};
use procgen_cubesphere::{TILE_QUADS, TILE_VERTICES, TileAddress};
use rayon::prelude::*;

use crate::{
    TerrainControlBake, TerrainHeightInputs, TerrainNoiseKeys, TerrainStampInput,
    ValidatedTerrainHeightConfig, terrain_height,
};

/// Number of core samples in one 65 by 65 terrain tile.
pub const TERRAIN_TILE_SAMPLE_COUNT: usize = (TILE_VERTICES * TILE_VERTICES) as usize;

const TANGENT_RELATIVE_TOLERANCE: f32 = 2.0e-5;
const CULL_ROUNDING_MARGIN: f32 = 16.0 * f32::EPSILON;

/// Borrowed inputs for one canonical CPU terrain tile.
///
/// Octave selection is explicit in `config`; this stage does not infer an
/// octave count from the address or fade octaves between levels.
#[derive(Clone, Copy, Debug)]
pub struct TerrainTileInputs<'a> {
    pub address: TileAddress,
    pub controls: &'a TerrainControlBake,
    /// Validated terrain stamps in their established stable order.
    pub stamps: &'a [TerrainStampInput],
    pub noise_keys: TerrainNoiseKeys,
}

/// Normalized height and tangent derivative for the core vertex grid.
///
/// Samples are row-major, from the tile's lower-left vertex. Directions are
/// deliberately omitted because they are derivable from `TileAddress`.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainTile {
    pub samples: Vec<ScalarFieldSample3>,
}

impl TerrainTile {
    pub fn sample(&self, x: u32, y: u32) -> Option<ScalarFieldSample3> {
        if x >= TILE_VERTICES || y >= TILE_VERTICES {
            return None;
        }
        self.samples.get((y * TILE_VERTICES + x) as usize).copied()
    }

    /// Checks the shape, normalized finite heights, and tangent derivatives.
    pub fn validate(&self, address: TileAddress) -> Result<(), TerrainTileError> {
        if self.samples.len() != TERRAIN_TILE_SAMPLE_COUNT {
            return Err(TerrainTileError::InvalidShape);
        }
        for (index, sample) in self.samples.iter().enumerate() {
            if !sample.value.is_finite() || !sample.derivative.is_finite() {
                return Err(TerrainTileError::NonFiniteSample);
            }
            if !(0.0..=1.0).contains(&sample.value) {
                return Err(TerrainTileError::HeightOutOfRange);
            }
            let x = index as u32 % TILE_VERTICES;
            let y = index as u32 / TILE_VERTICES;
            let direction = address
                .grid_vertex(x, y)
                .expect("tile result dimensions keep every local vertex in range")
                .direction();
            let tangent_error = sample.derivative.dot(direction).abs();
            let allowed = TANGENT_RELATIVE_TOLERANCE * (1.0 + sample.derivative.length());
            if tangent_error > allowed {
                return Err(TerrainTileError::NonTangentDerivative);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainTileError {
    InvalidShape,
    NonFiniteSample,
    HeightOutOfRange,
    NonTangentDerivative,
}

impl fmt::Display for TerrainTileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidShape => {
                write!(
                    formatter,
                    "terrain tile must contain a {TILE_VERTICES} by {TILE_VERTICES} core grid"
                )
            }
            Self::NonFiniteSample => formatter.write_str("terrain tile samples must be finite"),
            Self::HeightOutOfRange => {
                formatter.write_str("terrain tile heights must be normalized")
            }
            Self::NonTangentDerivative => {
                formatter.write_str("terrain tile derivatives must be tangent to the sphere")
            }
        }
    }
}

impl Error for TerrainTileError {}

/// Generates one addressed terrain tile on the canonical CPU path.
///
/// Each vertex direction comes from the address's integer `FaceGridVertex`.
/// Work is parallel by vertex; because every sample is independent and stamps
/// retain their serial accumulation order, thread count cannot change results.
pub fn generate_terrain_tile(
    inputs: TerrainTileInputs<'_>,
    config: ValidatedTerrainHeightConfig,
) -> TerrainTile {
    let directions = tile_directions(inputs.address);
    let stamps = cull_stamps(&directions, inputs.stamps, config);
    let samples = evaluate_directions(&directions, inputs, &stamps, config);
    let tile = TerrainTile { samples };
    debug_assert_eq!(tile.validate(inputs.address), Ok(()));
    tile
}

fn tile_directions(address: TileAddress) -> Vec<Vec3> {
    (0..TERRAIN_TILE_SAMPLE_COUNT)
        .map(|index| {
            let x = index as u32 % TILE_VERTICES;
            let y = index as u32 / TILE_VERTICES;
            address
                .grid_vertex(x, y)
                .expect("the settled tile dimensions are valid local coordinates")
                .direction()
        })
        .collect()
}

fn cull_stamps(
    directions: &[Vec3],
    stamps: &[TerrainStampInput],
    config: ValidatedTerrainHeightConfig,
) -> Vec<TerrainStampInput> {
    let center = directions[((TILE_QUADS / 2) * TILE_VERTICES + TILE_QUADS / 2) as usize];
    let tile_radius = directions
        .iter()
        .map(|direction| (*direction - center).length())
        .fold(0.0, f32::max);

    stamps
        .iter()
        .copied()
        .filter(|stamp| {
            let support = config.stamps.profile(stamp.kind).radius;
            // A small outward allowance keeps the conservative triangle bound
            // conservative after the f32 distance calculations.
            (stamp.position - center).length() <= tile_radius + support + CULL_ROUNDING_MARGIN
        })
        .collect()
}

fn evaluate_directions(
    directions: &[Vec3],
    inputs: TerrainTileInputs<'_>,
    stamps: &[TerrainStampInput],
    config: ValidatedTerrainHeightConfig,
) -> Vec<ScalarFieldSample3> {
    directions
        .par_iter()
        .map(|&direction| {
            terrain_height(
                TerrainHeightInputs {
                    direction,
                    controls: inputs.controls,
                    stamps,
                    noise_keys: inputs.noise_keys,
                },
                config,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        TerrainAbyssalConfig, TerrainCellControls, TerrainCoastConfig, TerrainDetailConfig,
        TerrainHeightConfig, TerrainStampKind,
    };
    use procgen_cubesphere::CubeFace;
    use procgen_noise::OctaveConfig;
    use rayon::ThreadPoolBuilder;

    const SEED: u64 = 0x0123_4567_89AB_CDEF;

    fn bake() -> TerrainControlBake {
        let texel = TerrainCellControls {
            base_elevation: 0.61,
            detail_amplitude: 0.018,
            ridge_weight: 0.42,
            octave_gain: 0.51,
            abyssal_amplitude: 0.006,
        }
        .to_channels();
        TerrainControlBake::from_face_texels(4, std::array::from_fn(|_| vec![texel; 16])).unwrap()
    }

    fn config(octaves: u32) -> ValidatedTerrainHeightConfig {
        let default = TerrainHeightConfig::default();
        TerrainHeightConfig {
            detail: TerrainDetailConfig {
                octaves: OctaveConfig {
                    octaves,
                    ..default.detail.octaves
                },
                ..default.detail
            },
            abyssal: TerrainAbyssalConfig {
                octaves: OctaveConfig {
                    octaves: octaves.min(6),
                    ..default.abyssal.octaves
                },
                ..default.abyssal
            },
            coast: TerrainCoastConfig {
                maximum_warp: 0.0,
                ..default.coast
            },
            ..default
        }
        .validate()
        .unwrap()
    }

    fn inputs<'a>(
        address: TileAddress,
        controls: &'a TerrainControlBake,
        stamps: &'a [TerrainStampInput],
    ) -> TerrainTileInputs<'a> {
        TerrainTileInputs {
            address,
            controls,
            stamps,
            noise_keys: TerrainNoiseKeys::new(SEED),
        }
    }

    #[test]
    fn pinned_tile_output_is_stable() {
        let bake = bake();
        let address = TileAddress::new(CubeFace::PositiveZ, 5, 17, 9).unwrap();
        let tile = generate_terrain_tile(inputs(address, &bake, &[]), config(3));
        assert_eq!(tile.validate(address), Ok(()));
        let pinned = [0, 64, 2_112, 4_160, 4_224].map(|index| {
            let sample = tile.samples[index];
            [
                sample.value.to_bits(),
                sample.derivative.x.to_bits(),
                sample.derivative.y.to_bits(),
                sample.derivative.z.to_bits(),
            ]
        });
        assert_eq!(
            pinned,
            [
                [0x3F1E_53A0, 0x3F00_0318, 0xBDAD_A632, 0xBD60_E2FD],
                [0x3F1D_D4B4, 0xBD67_EB95, 0x3F93_CA14, 0x3ED6_5FF4],
                [0x3F1E_4F1A, 0x3E82_B1B4, 0xBF63_FFC7, 0xBEA0_4836],
                [0x3F1D_7839, 0x3D0A_CAEC, 0x3F24_115F, 0x3E45_5F89],
                [0x3F1E_EAFF, 0xBF83_169D, 0xBD8A_4FBF, 0x3DA4_9EF0],
            ]
        );
    }

    #[test]
    fn output_is_identical_across_thread_counts() {
        let bake = bake();
        let address = TileAddress::new(CubeFace::NegativeY, 7, 38, 91).unwrap();
        let generate = || generate_terrain_tile(inputs(address, &bake, &[]), config(5));
        let one = ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(generate);
        let four = ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap()
            .install(generate);
        assert_eq!(one, four);
    }

    #[test]
    fn same_level_edges_and_face_seams_are_bit_identical() {
        let bake = bake();
        let config = config(2);
        let left_address = TileAddress::new(CubeFace::PositiveZ, 3, 2, 5).unwrap();
        let right_address = TileAddress::new(CubeFace::PositiveZ, 3, 3, 5).unwrap();
        let left = generate_terrain_tile(inputs(left_address, &bake, &[]), config);
        let right = generate_terrain_tile(inputs(right_address, &bake, &[]), config);
        for y in 0..TILE_VERTICES {
            assert_eq!(left.sample(TILE_QUADS, y), right.sample(0, y));
        }
        let upper_address = TileAddress::new(CubeFace::PositiveZ, 3, 2, 6).unwrap();
        let upper = generate_terrain_tile(inputs(upper_address, &bake, &[]), config);
        for x in 0..TILE_VERTICES {
            assert_eq!(left.sample(x, TILE_QUADS), upper.sample(x, 0));
        }

        let face_left_address = TileAddress::new(CubeFace::PositiveX, 3, 0, 5).unwrap();
        let face_right_address = TileAddress::new(CubeFace::PositiveZ, 3, 7, 5).unwrap();
        let face_left = generate_terrain_tile(inputs(face_left_address, &bake, &[]), config);
        let face_right = generate_terrain_tile(inputs(face_right_address, &bake, &[]), config);
        for y in 0..TILE_VERTICES {
            assert_eq!(face_left.sample(0, y), face_right.sample(TILE_QUADS, y));
        }
    }

    #[test]
    fn stamp_culling_matches_unculled_evaluation_and_keeps_order() {
        let bake = bake();
        let address = TileAddress::new(CubeFace::PositiveZ, 8, 100, 110).unwrap();
        let directions = tile_directions(address);
        let near = directions[TERRAIN_TILE_SAMPLE_COUNT / 2];
        let stamps = [
            TerrainStampInput {
                cell: 0,
                kind: TerrainStampKind::Hotspot,
                source_index: 0,
                position: near,
                strength: 0.4,
            },
            TerrainStampInput {
                cell: 1,
                kind: TerrainStampKind::VolcanicArc,
                source_index: 1,
                position: -near,
                strength: 0.8,
            },
            TerrainStampInput {
                cell: 2,
                kind: TerrainStampKind::OceanicSeamount,
                source_index: 2,
                position: near,
                strength: 0.3,
            },
        ];
        let config = config(2);
        let culled = cull_stamps(&directions, &stamps, config);
        assert_eq!(culled, vec![stamps[0], stamps[2]]);
        let generated = generate_terrain_tile(inputs(address, &bake, &stamps), config);
        let unculled = TerrainTile {
            samples: evaluate_directions(
                &directions,
                inputs(address, &bake, &stamps),
                &stamps,
                config,
            ),
        };
        assert_eq!(generated, unculled);
    }

    #[test]
    fn validation_rejects_each_invalid_output_invariant() {
        let address = TileAddress::new(CubeFace::PositiveX, 0, 0, 0).unwrap();
        let mut tile = TerrainTile {
            samples: vec![ScalarFieldSample3::default(); TERRAIN_TILE_SAMPLE_COUNT],
        };
        assert_eq!(tile.validate(address), Ok(()));
        tile.samples.pop();
        assert_eq!(tile.validate(address), Err(TerrainTileError::InvalidShape));
        tile.samples.push(ScalarFieldSample3::constant(f32::NAN));
        assert_eq!(
            tile.validate(address),
            Err(TerrainTileError::NonFiniteSample)
        );
        *tile.samples.last_mut().unwrap() = ScalarFieldSample3::constant(1.1);
        assert_eq!(
            tile.validate(address),
            Err(TerrainTileError::HeightOutOfRange)
        );
        tile.samples.fill(ScalarFieldSample3::default());
        tile.samples[0].derivative = address.grid_vertex(0, 0).unwrap().direction();
        assert_eq!(
            tile.validate(address),
            Err(TerrainTileError::NonTangentDerivative)
        );
    }

    #[test]
    fn level_twelve_tile_generates_and_validates() {
        let bake = bake();
        let address = TileAddress::new(CubeFace::NegativeX, 12, 2_941, 1_107).unwrap();
        let tile = generate_terrain_tile(inputs(address, &bake, &[]), config(11));
        assert_eq!(tile.samples.len(), TERRAIN_TILE_SAMPLE_COUNT);
        assert_eq!(tile.validate(address), Ok(()));
    }
}
