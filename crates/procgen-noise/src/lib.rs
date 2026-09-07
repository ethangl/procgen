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
pub use gradient::{NoiseSample3, fold_seed_u64_to_u32, gradient_noise_3d, lattice_gradient_3d};

/// Absolute CPU/backend tolerance for noise values in normalized units.
///
/// This value remains provisional pending CUDA calibration. On Apple M1 Max
/// via Metal, commit `00ec06ad12f83cc0f95fe2c37697aab70010c969`
/// measured a maximum `1.192092896e-7` across 12 representative samples.
pub const NOISE_VALUE_TOLERANCE: f32 = 1.0e-5;

/// CPU/backend angular tolerance for noise derivatives, in radians.
///
/// Derivatives are compared by direction so increasing octave frequency does
/// not turn ordinary floating-point scaling into a false agreement failure.
/// This value remains provisional pending CUDA calibration. On Apple M1 Max
/// via Metal, commit `00ec06ad12f83cc0f95fe2c37697aab70010c969`
/// measured a maximum `4.657744739e-6` across 12 representative samples.
pub const NOISE_DERIVATIVE_ANGLE_TOLERANCE: f32 = 1.0e-3;

/// WGSL mirror of the canonical CPU noise implementation.
///
/// The source is backend-neutral shader code and does not introduce a `wgpu`
/// dependency into this crate. Its integer and floating-point agreement is
/// exercised by the test-only `procgen-gpu-tests` workspace crate.
pub const WGSL_SOURCE: &str = include_str!("../wgsl/noise.wgsl");
