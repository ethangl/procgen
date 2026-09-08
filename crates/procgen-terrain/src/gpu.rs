//! Backend-neutral packing contract for the hand-mirrored terrain shader.

use procgen_core::Vec3;

use crate::{
    StampCap, TerrainControlBake, TerrainHeightConfig, TerrainNoiseKeys, TerrainStampInput,
    TerrainStampKind,
};

/// Absolute CPU/WGSL height tolerance in normalized elevation units.
///
/// On 2026-09-08, `wgsl_terrain_tiles_agree_with_canonical_cpu` measured a
/// maximum `6.556510925e-7` across 8,450 level-4 tile samples on Apple M1 Max
/// via Metal. The tolerance is ten times that measured maximum, rounded upward.
pub const TERRAIN_WGSL_VALUE_TOLERANCE: f32 = 1.0e-5;

/// CPU/WGSL derivative-direction tolerance in radians.
///
/// On 2026-09-08, `wgsl_terrain_tiles_agree_with_canonical_cpu` measured a
/// maximum `1.119942754e-3` across 8,450 level-4 tile samples on Apple M1 Max
/// via Metal. The tolerance is ten times that measured maximum, rounded upward.
pub const TERRAIN_WGSL_DERIVATIVE_ANGLE_TOLERANCE: f32 = 1.2e-2;

/// Checked-in WGSL mirror of canonical terrain height and tile evaluation.
///
/// The source calls the settled functions from `procgen-noise` and expects its
/// consumer to provide storage accessors for control texels and stamps. This
/// keeps the terrain algorithm independent of a particular wgpu bind-group
/// layout.
pub const TERRAIN_WGSL_SOURCE: &str = concat!(
    include_str!("../../procgen-noise/wgsl/noise.wgsl"),
    "\n",
    include_str!("../wgsl/terrain.wgsl")
);

/// One control texel in the shader storage layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainGpuControlTexel {
    pub channels_0: [f32; 4],
    pub channels_1: [f32; 4],
}

/// One stable terrain stamp in the shader storage layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainGpuStamp {
    pub position_strength: [f32; 4],
    pub kind_padding: [u32; 4],
}

/// Scalar and vector parameters shared by every tile from one generated world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainGpuParameters {
    pub dimensions: [u32; 4],
    pub noise_keys_0: [u32; 4],
    pub noise_keys_1: [u32; 4],
    pub detail_octaves: [u32; 4],
    pub detail: [f32; 4],
    pub abyssal: [f32; 4],
    pub coast: [f32; 4],
    pub stamp_profiles: [[f32; 4]; 4],
}

impl TerrainGpuParameters {
    pub fn new(
        controls: &TerrainControlBake,
        stamps: &[TerrainStampInput],
        noise_keys: TerrainNoiseKeys,
        config: TerrainHeightConfig,
    ) -> Self {
        config
            .validate()
            .expect("GPU terrain parameters require a valid height configuration");
        let profiles = config.stamps;
        Self {
            dimensions: [
                controls.resolution(),
                u32::try_from(stamps.len()).expect("terrain stamp count must fit in u32"),
                0,
                0,
            ],
            noise_keys_0: [
                noise_keys.detail,
                noise_keys.abyssal,
                noise_keys.coast_warp[0],
                noise_keys.coast_warp[1],
            ],
            noise_keys_1: [noise_keys.coast_warp[2], 0, 0, 0],
            detail_octaves: [
                config.detail.octaves.octaves,
                config.abyssal.octaves.octaves,
                0,
                0,
            ],
            detail: [
                config.detail.octaves.frequency,
                config.detail.octaves.lacunarity,
                config.detail.derivative_damping,
                config.detail.ridge_offset,
            ],
            abyssal: [
                config.abyssal.octaves.frequency,
                config.abyssal.octaves.lacunarity,
                config.abyssal.derivative_damping,
                config.detail.ridge_gain,
            ],
            coast: [
                config.coast.half_width,
                config.coast.warp_frequency,
                config.coast.maximum_warp,
                0.0,
            ],
            stamp_profiles: TerrainStampKind::ALL.map(|kind| {
                let profile = profiles.profile(kind);
                [
                    profile.radius,
                    profile.amplitude,
                    match profile.cap {
                        StampCap::Quadratic => 2.0,
                        StampCap::Cubic => 3.0,
                    },
                    0.0,
                ]
            }),
        }
    }
}

pub fn pack_control_bake(controls: &TerrainControlBake) -> Vec<TerrainGpuControlTexel> {
    procgen_cubesphere::CubeFace::ALL
        .into_iter()
        .flat_map(|face| controls.face(face).texels())
        .map(|channels| TerrainGpuControlTexel {
            channels_0: [channels[0], channels[1], channels[2], channels[3]],
            channels_1: [channels[4], 0.0, 0.0, 0.0],
        })
        .collect()
}

pub fn pack_stamps(stamps: &[TerrainStampInput]) -> Vec<TerrainGpuStamp> {
    stamps
        .iter()
        .map(|stamp| {
            let Vec3 { x, y, z } = stamp.position;
            TerrainGpuStamp {
                position_strength: [x, y, z, stamp.strength],
                kind_padding: [stamp.kind as u32, 0, 0, 0],
            }
        })
        .collect()
}
