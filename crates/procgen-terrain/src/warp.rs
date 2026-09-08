//! Bounded coastline domain warp and its precomputed derivative transform.

use procgen_core::Vec3;
use procgen_cubesphere::CubeFieldSample;
use procgen_noise::{GRADIENT_NOISE_VALUE_BOUND, NoiseSample3, gradient_noise_3d};
use procgen_tectonics::SEA_LEVEL;

use crate::TerrainCellControls;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainCoastConfig {
    /// Half-width of the normalized-elevation band affected by coast behavior.
    pub half_width: f32,
    /// Frequency of the three scalar fields forming the tangent-space warp.
    pub warp_frequency: f32,
    /// Maximum pre-normalization tangent displacement on the unit sphere.
    pub maximum_warp: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct DomainWarp {
    pub(crate) direction: Vec3,
    jacobian_transpose: [Vec3; 3],
}

impl DomainWarp {
    pub(crate) fn pullback(self, sample: NoiseSample3) -> NoiseSample3 {
        NoiseSample3 {
            value: sample.value,
            derivative: transpose_product(self.jacobian_transpose, sample.derivative),
        }
    }

    pub(crate) fn pullback_controls(
        self,
        sample: CubeFieldSample<{ TerrainCellControls::<f32>::CHANNELS }>,
    ) -> TerrainCellControls<NoiseSample3> {
        TerrainCellControls::from_channels(std::array::from_fn(|channel| {
            self.pullback(NoiseSample3 {
                value: sample.values[channel],
                derivative: sample.derivatives[channel],
            })
        }))
    }
}

impl TerrainCellControls<NoiseSample3> {
    pub(crate) fn from_cube_sample(
        sample: CubeFieldSample<{ TerrainCellControls::<f32>::CHANNELS }>,
    ) -> Self {
        Self::from_channels(std::array::from_fn(|channel| NoiseSample3 {
            value: sample.values[channel],
            derivative: sample.derivatives[channel],
        }))
    }
}

pub(crate) fn coast_warp(
    direction: Vec3,
    seeds: [u64; 3],
    weight: NoiseSample3,
    config: TerrainCoastConfig,
) -> DomainWarp {
    let samples = seeds.map(|seed| gradient_noise_3d(seed, direction * config.warp_frequency));
    let raw = Vec3::new(samples[0].value, samples[1].value, samples[2].value);
    let raw_derivatives = samples.map(|sample| sample.derivative * config.warp_frequency);
    let tangent = raw - direction * raw.dot(direction);
    let vector_bound = 3.0_f32.sqrt() * GRADIENT_NOISE_VALUE_BOUND;
    let maximum_scale = config.maximum_warp / vector_bound;
    let scale = maximum_scale * weight.value;
    let scale_derivative = weight.derivative * maximum_scale;
    let displaced = direction + tangent * scale;
    let inverse_length = displaced.length().recip();
    let warped_direction = displaced * inverse_length;
    let raw_dot_source = raw.dot(direction);
    let projection_derivative = transpose_product(raw_derivatives, direction) + raw;
    let jacobian_transpose = [Vec3::X, Vec3::Y, Vec3::Z].map(|derivative| {
        let normalized =
            (derivative - warped_direction * derivative.dot(warped_direction)) * inverse_length;
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

pub(crate) fn coast_taper(base: NoiseSample3, half_width: f32) -> NoiseSample3 {
    let signed = NoiseSample3 {
        value: base.value - SEA_LEVEL,
        derivative: base.derivative,
    };
    let distance = if signed.value > 0.0 {
        signed
    } else {
        signed * -1.0
    };
    if distance.value >= half_width {
        return NoiseSample3::constant(1.0);
    }
    let t = distance * half_width.recip();
    t * t * (NoiseSample3::constant(3.0) - t * 2.0)
}
