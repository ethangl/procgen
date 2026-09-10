//! Coast taper, bounded domain warp, and their derivative propagation.

use std::{error::Error, fmt};

use procgen_core::{ScalarFieldSample3, Vec3};
use procgen_noise::{GRADIENT_NOISE_VALUE_BOUND, gradient_noise_3d};

const SQRT_3: f32 = 1.732_050_8;
const GRADIENT_NOISE_VECTOR_BOUND: f32 = SQRT_3 * GRADIENT_NOISE_VALUE_BOUND;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainCoastConfig {
    /// Half-width of the normalized-elevation band around sea level affected
    /// by coast behavior.
    pub half_width: f32,
    /// Frequency of the three scalar fields forming the tangent-space warp.
    pub warp_frequency: f32,
    /// Maximum pre-normalization tangent displacement on the unit sphere.
    pub maximum_warp: f32,
}

impl TerrainCoastConfig {
    pub(crate) fn validate(self) -> Result<(), TerrainCoastError> {
        if !self.half_width.is_finite() || self.half_width <= 0.0 {
            return Err(TerrainCoastError::InvalidHalfWidth);
        }
        if !self.warp_frequency.is_finite() || self.warp_frequency <= 0.0 {
            return Err(TerrainCoastError::InvalidWarpFrequency);
        }
        if !self.maximum_warp.is_finite() || !(0.0..=0.25).contains(&self.maximum_warp) {
            return Err(TerrainCoastError::InvalidMaximumWarp);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainCoastError {
    InvalidHalfWidth,
    InvalidWarpFrequency,
    InvalidMaximumWarp,
}

impl fmt::Display for TerrainCoastError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidHalfWidth => "terrain coast half-width is invalid",
            Self::InvalidWarpFrequency => "terrain coast warp frequency is invalid",
            Self::InvalidMaximumWarp => "terrain maximum coast warp is invalid",
        })
    }
}

impl Error for TerrainCoastError {}

#[derive(Clone, Copy)]
pub(crate) struct DomainWarp {
    pub(crate) direction: Vec3,
    jacobian_transpose: [Vec3; 3],
}

impl DomainWarp {
    pub(crate) fn pullback(self, sample: ScalarFieldSample3) -> ScalarFieldSample3 {
        ScalarFieldSample3 {
            value: sample.value,
            derivative: transpose_product(self.jacobian_transpose, sample.derivative),
        }
    }
}

pub(crate) fn coast_warp(
    direction: Vec3,
    keys: [u32; 3],
    weight: ScalarFieldSample3,
    config: TerrainCoastConfig,
) -> DomainWarp {
    let samples = keys.map(|key| gradient_noise_3d(key, direction * config.warp_frequency));
    let raw = Vec3::new(samples[0].value, samples[1].value, samples[2].value);
    let raw_derivatives = samples.map(|sample| sample.derivative * config.warp_frequency);
    let tangent = raw - direction * raw.dot(direction);
    let maximum_scale = config.maximum_warp / GRADIENT_NOISE_VECTOR_BOUND;
    let scale = maximum_scale * weight.value;
    let scale_derivative = weight.derivative * maximum_scale;
    let displaced = direction + tangent * scale;
    let inverse_length = displaced.length().recip();
    let warped_direction = displaced * inverse_length;
    let raw_dot_source = raw.dot(direction);
    let projection_derivative = transpose_product(raw_derivatives, direction) + raw;
    let jacobian_transpose = [Vec3::X, Vec3::Y, Vec3::Z].map(|axis| {
        let normalized = (axis - warped_direction * axis.dot(warped_direction)) * inverse_length;
        normalized
            + scale_derivative * normalized.dot(tangent)
            + (transpose_product(raw_derivatives, normalized)
                - normalized * raw_dot_source
                - projection_derivative * normalized.dot(direction))
                * scale
    });
    DomainWarp {
        direction: warped_direction,
        jacobian_transpose,
    }
}

fn transpose_product(rows: [Vec3; 3], vector: Vec3) -> Vec3 {
    rows[0] * vector.x + rows[1] * vector.y + rows[2] * vector.z
}

pub(crate) fn coast_taper(
    base: ScalarFieldSample3,
    sea_level: f32,
    half_width: f32,
) -> ScalarFieldSample3 {
    let signed = ScalarFieldSample3 {
        value: base.value - sea_level,
        derivative: base.derivative,
    };
    let distance = if signed.value > 0.0 { signed } else { -signed };
    if distance.value >= half_width {
        return ScalarFieldSample3::constant(1.0);
    }
    let t = distance * half_width.recip();
    t * t * (ScalarFieldSample3::constant(3.0) - t * 2.0)
}
