//! Deterministic, backend-neutral noise primitives.

mod fractal;
mod gradient;

#[cfg(test)]
mod test_support;

pub use fractal::{
    DerivativeDampedConfig, FractalParameterError, MAX_OCTAVES, OctaveBand, OctaveConfig,
    OctaveGain, RidgedMultifractalConfig, Validated, amplitude_sum, derivative_damped_fbm_3d,
    fbm_3d, ridged_multifractal_3d,
};
pub use gradient::{
    GRADIENT_NOISE_VALUE_BOUND, fold_seed_u64_to_u32, gradient_noise_3d, lattice_gradient_3d,
};

/// Absolute CPU/backend tolerance for noise values in normalized units.
///
/// This value remains provisional pending CUDA calibration. On 2026-09-08,
/// `wgsl_noise_agrees_with_canonical_cpu` measured a maximum
/// `1.192092896e-7` across 12 representative samples on Apple M1 Max via Metal.
pub const NOISE_VALUE_TOLERANCE: f32 = 1.0e-5;

/// CPU/backend angular tolerance for noise derivatives, in radians.
///
/// Derivatives are compared by direction so increasing octave frequency does
/// not turn ordinary floating-point scaling into a false agreement failure.
/// This value remains provisional pending CUDA calibration. On 2026-09-08,
/// `wgsl_noise_agrees_with_canonical_cpu` measured a maximum
/// `2.291890496e-6` across 12 representative samples on Apple M1 Max via Metal.
pub const NOISE_DERIVATIVE_ANGLE_TOLERANCE: f32 = 1.0e-3;

/// WGSL mirror of the canonical CPU noise implementation.
///
/// The source is backend-neutral shader code and does not introduce a `wgpu`
/// dependency into this crate. Its integer and floating-point agreement is
/// exercised by the test-only `procgen-gpu-tests` workspace crate.
pub const WGSL_SOURCE: &str = concat!(
    include_str!("../../procgen-core/wgsl/hash.wgsl"),
    "\n",
    include_str!("../wgsl/noise.wgsl")
);
