//! Backend-neutral packing contract for the hand-mirrored terrain shader.

use bytemuck::{Pod, Zeroable};
use procgen_core::Vec3;

use crate::{
    TerrainControlBake, TerrainHeightConfig, TerrainNoiseKeys, TerrainStampInput, TerrainStampKind,
};

/// Absolute CPU/WGSL height tolerance in normalized elevation units.
///
/// On 2026-09-08, `wgsl_terrain_tiles_agree_with_canonical_cpu_and_share_edges`
/// measured a maximum `2.503395081e-6` across 29,575 representative level-1,
/// level-4, level-8, and level-12 samples plus fade endpoints on Apple M1 Max
/// via Metal. The tolerance is ten times that maximum, rounded upward.
pub const TERRAIN_WGSL_VALUE_TOLERANCE: f32 = 3.0e-5;

/// CPU/WGSL rendered-normal tolerance in radians.
///
/// On 2026-09-08, the same dispatch measured a maximum `1.637477577e-1`
/// radians using the agreement test's explicit `0.036` display relief.
/// Divergence is concentrated at the finest octave because CPU and Metal
/// transcendental cube-sphere mapping differ before high-frequency sampling.
/// The tolerance is ten times that maximum, rounded upward, and remains
/// provisional until the mapping and CUDA path are calibrated.
pub const TERRAIN_WGSL_NORMAL_ANGLE_TOLERANCE: f32 = 1.7;

/// Checked-in WGSL mirror of canonical terrain height and tile evaluation.
///
/// The source composes the settled `procgen-noise` kernel and
/// `procgen-cubesphere` mapping/field sources, then expects its consumer to
/// provide storage accessors for control texels and stamps. This keeps every
/// algorithm with its owning crate and leaves the wgpu bind-group layout to
/// the consumer.
pub const TERRAIN_WGSL_SOURCE: &str = concat!(
    include_str!("../../procgen-noise/wgsl/noise.wgsl"),
    "\n",
    include_str!("../../procgen-cubesphere/wgsl/mapping.wgsl"),
    "\n",
    include_str!("../../procgen-cubesphere/wgsl/field.wgsl"),
    "\n",
    include_str!("../wgsl/terrain.wgsl")
);

/// One stable terrain stamp in the shader storage layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct TerrainGpuStamp {
    pub position: [f32; 3],
    pub strength: f32,
    pub kind: u32,
    pub padding: [u32; 3],
}

/// One named terrain-stamp profile in the shader uniform layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct TerrainGpuStampProfile {
    pub radius: f32,
    pub amplitude: f32,
    pub cap: u32,
    pub padding: u32,
}

/// Scalar and vector parameters shared by every tile from one generated world.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct TerrainGpuParameters {
    pub control_resolution: u32,
    pub stamp_count: u32,
    pub detail_key: u32,
    pub abyssal_key: u32,
    pub coast_warp_key_x: u32,
    pub coast_warp_key_y: u32,
    pub coast_warp_key_z: u32,
    pub detail_octaves: u32,
    pub abyssal_octaves: u32,
    pub padding_0: [u32; 3],
    pub detail_frequency: f32,
    pub detail_lacunarity: f32,
    pub detail_derivative_damping: f32,
    pub detail_ridge_offset: f32,
    pub detail_ridge_gain: f32,
    pub abyssal_frequency: f32,
    pub abyssal_lacunarity: f32,
    pub abyssal_derivative_damping: f32,
    pub coast_half_width: f32,
    pub coast_warp_frequency: f32,
    pub coast_maximum_warp: f32,
    pub padding_1: f32,
    pub stamp_profiles: [TerrainGpuStampProfile; 4],
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
            control_resolution: controls.resolution(),
            stamp_count: u32::try_from(stamps.len()).expect("terrain stamp count must fit in u32"),
            detail_key: noise_keys.detail,
            abyssal_key: noise_keys.abyssal,
            coast_warp_key_x: noise_keys.coast_warp[0],
            coast_warp_key_y: noise_keys.coast_warp[1],
            coast_warp_key_z: noise_keys.coast_warp[2],
            detail_octaves: config.detail.octaves.octaves,
            abyssal_octaves: config.abyssal.octaves.octaves,
            padding_0: [0; 3],
            detail_frequency: config.detail.octaves.frequency,
            detail_lacunarity: config.detail.octaves.lacunarity,
            detail_derivative_damping: config.detail.derivative_damping,
            detail_ridge_offset: config.detail.ridge_offset,
            detail_ridge_gain: config.detail.ridge_gain,
            abyssal_frequency: config.abyssal.octaves.frequency,
            abyssal_lacunarity: config.abyssal.octaves.lacunarity,
            abyssal_derivative_damping: config.abyssal.derivative_damping,
            coast_half_width: config.coast.half_width,
            coast_warp_frequency: config.coast.warp_frequency,
            coast_maximum_warp: config.coast.maximum_warp,
            padding_1: 0.0,
            stamp_profiles: TerrainStampKind::ALL.map(|kind| {
                let profile = profiles.profile(kind);
                TerrainGpuStampProfile {
                    radius: profile.radius,
                    amplitude: profile.amplitude,
                    cap: profile.cap as u32,
                    padding: 0,
                }
            }),
        }
    }
}

pub fn pack_control_bake(controls: &TerrainControlBake) -> Vec<[f32; 5]> {
    let mut texels = Vec::with_capacity(
        procgen_cubesphere::CubeFace::ALL.len()
            * controls.resolution() as usize
            * controls.resolution() as usize,
    );
    for face in procgen_cubesphere::CubeFace::ALL {
        texels.extend_from_slice(controls.face(face).texels());
    }
    texels
}

pub fn pack_stamps(stamps: &[TerrainStampInput]) -> Vec<TerrainGpuStamp> {
    stamps
        .iter()
        .map(|stamp| {
            let Vec3 { x, y, z } = stamp.position;
            TerrainGpuStamp {
                position: [x, y, z],
                strength: stamp.strength,
                kind: stamp.kind as u32,
                padding: [0; 3],
            }
        })
        .collect()
}
