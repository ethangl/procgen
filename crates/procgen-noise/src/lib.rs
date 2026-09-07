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

/// Provisional absolute CPU/backend tolerance for noise values in normalized units.
///
/// This is ten times the intended cross-backend measured maximum. The current
/// Metal measurement is recorded by `procgen-gpu-tests`; CUDA calibration is
/// still pending.
pub const PROVISIONAL_NOISE_VALUE_ABSOLUTE_TOLERANCE: f32 = 1.0e-5;

/// Provisional CPU/backend angular tolerance for noise derivatives, in radians.
///
/// Derivatives are compared by direction so increasing octave frequency does
/// not turn ordinary floating-point scaling into a false agreement failure.
pub const PROVISIONAL_NOISE_DERIVATIVE_ANGLE_TOLERANCE: f32 = 1.0e-3;

/// WGSL mirror of the canonical CPU noise implementation.
///
/// The source is backend-neutral shader code and does not introduce a `wgpu`
/// dependency into this crate. Its integer and floating-point agreement is
/// exercised by the test-only `procgen-gpu-tests` workspace crate.
pub const WGSL_SOURCE: &str = include_str!("../wgsl/noise.wgsl");
