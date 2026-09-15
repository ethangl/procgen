//! Backend-neutral GPU density input layout. Device ownership stays with consumers.
use bytemuck::{Pod, Zeroable};

use crate::{
    MAX_DESIGN_OCTAVES, PlanetDesignField, VOXEL_HALO, VOXEL_SAMPLE_COUNT, VOXEL_SAMPLE_SIDE,
    VoxelChunkAddress,
    noise::{ABS_MEAN, ABS_STD_DEV, BASIS_STD_DEV, SQUARE_MEAN, SQUARE_STD_DEV},
    voxel_density::{SQRT_ITERATIONS, SQRT_SEED},
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuOctave {
    wavelength_m: f32,
    amplitude_m: f32,
    sharpness: f32,
    perturbation: f32,
    slope_erosion: f32,
    altitude_erosion: f32,
    ridge_erosion: f32,
    enabled: u32,
}

/// Eight words, the same 32 bytes as one octave, so the octave array keeps its
/// uniform alignment; the two pads are always zero.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuVolume {
    wavelength_m: f32,
    amplitude_m: f32,
    sharpness: f32,
    fade_m: f32,
    enabled: u32,
    key: u32,
    pad: [u32; 2],
}

/// Uniform layout for an already validated field. Seeds are narrowed only by
/// the canonical field constructor; this upload reuses that resolved key.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct VoxelGpuParameters {
    radius_m: f32,
    height_limit_m: f32,
    key: u32,
    octave_count: u32,
    arithmetic: [f32; 4],
    volume: GpuVolume,
    octaves: [GpuOctave; MAX_DESIGN_OCTAVES],
}
impl VoxelGpuParameters {
    pub fn new(field: &PlanetDesignField) -> Self {
        let config = field.config();
        let mut result = Self {
            radius_m: config.radius_m,
            height_limit_m: config.height_limit_m,
            key: field.noise_key(),
            octave_count: config.octaves.len() as u32,
            arithmetic: [1.0, 0.0, 0.0, 0.0],
            volume: GpuVolume {
                wavelength_m: config.volume.wavelength_m,
                amplitude_m: config.volume.amplitude_m,
                sharpness: config.volume.sharpness,
                fade_m: config.volume.fade_m,
                enabled: u32::from(config.volume.enabled),
                key: field.volume_key(),
                pad: [0; 2],
            },
            octaves: [GpuOctave::zeroed(); MAX_DESIGN_OCTAVES],
        };
        for (target, source) in result.octaves.iter_mut().zip(&config.octaves) {
            *target = GpuOctave {
                wavelength_m: source.wavelength_m,
                amplitude_m: source.amplitude_m,
                sharpness: source.sharpness,
                perturbation: source.perturbation,
                slope_erosion: source.slope_erosion,
                altitude_erosion: source.altitude_erosion,
                ridge_erosion: source.ridge_erosion,
                enabled: u32::from(source.enabled),
            };
        }
        result
    }
}

/// Storage record for one exact integer chunk origin and dyadic sample spacing.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct VoxelGpuChunk {
    origin_m: [i32; 3],
    spacing_m: i32,
}
impl VoxelGpuChunk {
    pub fn new(address: VoxelChunkAddress) -> Self {
        let p = address.origin();
        Self {
            origin_m: [p.x_m, p.y_m, p.z_m],
            spacing_m: address.spacing_m(),
        }
    }
}

/// Full-band potential kernel: group 0 binds parameters (uniform), chunk records
/// (read-only storage), and f32 potentials (storage). Output is chunk-major,
/// then x-fastest over the complete halo grid. Dispatch ceil(samples / 64)
/// workgroups along x. The caller owns buffer capacity and device dispatch limits.
/// Normal visual generation keeps the output on the GPU; only audits read it back.
pub fn voxel_density_shader() -> String {
    format!(
        "{}\n\
         const PILOT_MAX_OCTAVES: u32 = {MAX_DESIGN_OCTAVES}u;\n\
         const PILOT_SAMPLE_SIDE: u32 = {VOXEL_SAMPLE_SIDE}u;\n\
         const PILOT_SAMPLE_COUNT: u32 = {VOXEL_SAMPLE_COUNT}u;\n\
         const PILOT_HALO: i32 = {VOXEL_HALO};\n\
         const PILOT_SQRT_SEED: u32 = {SQRT_SEED}u;\n\
         const PILOT_SQRT_ITERATIONS: u32 = {SQRT_ITERATIONS}u;\n\
         const PILOT_NOISE_SCALE: f32 = {:?};\n\
         const PILOT_BASIS_STD: f32 = {BASIS_STD_DEV:?};\n\
         const PILOT_ABS_MEAN: f32 = {ABS_MEAN:?};\n\
         const PILOT_ABS_STD: f32 = {ABS_STD_DEV:?};\n\
         const PILOT_SQUARE_MEAN: f32 = {SQUARE_MEAN:?};\n\
         const PILOT_SQUARE_STD: f32 = {SQUARE_STD_DEV:?};\n{}",
        procgen_noise::WGSL_SOURCE,
        procgen_noise::GRADIENT_NOISE_VALUE_BOUND.recip(),
        include_str!("voxel_density.wgsl"),
    )
}

/// Input record for the sample_points density entry point.
#[cfg(feature = "gpu")]
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct VoxelGpuPoint {
    position_m: [i32; 3],
    reserved: i32,
}
#[cfg(feature = "gpu")]
impl VoxelGpuPoint {
    pub fn new(p: crate::VoxelPosition) -> Self {
        Self {
            position_m: [p.x_m, p.y_m, p.z_m],
            reserved: 0,
        }
    }
}
