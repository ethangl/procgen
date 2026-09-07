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
pub use gradient::{NoiseSample3, fold_seed_u64_to_u32, gradient_noise_3d};

/// WGSL mirror of the canonical CPU noise implementation.
///
/// The source is backend-neutral shader code and does not introduce a `wgpu`
/// dependency into this crate. Its integer and floating-point agreement is
/// exercised by the test-only `procgen-gpu-tests` workspace crate.
pub const WGSL_SOURCE: &str = include_str!("../wgsl/noise.wgsl");
