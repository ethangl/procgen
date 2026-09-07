//! Deterministic, backend-neutral noise primitives.

mod fractal;
mod gradient;

#[cfg(test)]
mod test_support;

pub use fractal::{
    DerivativeDampedConfig, FractalParameterError, MAX_OCTAVES, OctaveConfig, OctaveGain,
    RidgedMultifractalConfig, Validated, amplitude_sum, derivative_damped_fbm_3d, fbm_3d,
    ridged_multifractal_3d,
};
pub use gradient::{NoiseSample3, gradient_noise_3d};
