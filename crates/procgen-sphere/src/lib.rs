//! Spherical geometry and deterministic point distributions.
//!
//! Coordinates are right-handed and Y-up: latitude is measured from the XZ
//! plane, and longitude rotates from +X toward +Z.

use procgen_core::{RandomStream, Vec3};
use rayon::prelude::*;
use std::fmt;

const GOLDEN_RATIO: f32 = 1.618_034;
const PARALLEL_THRESHOLD: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FibonacciConfig {
    pub count: usize,
    pub jitter: f32,
    pub seed: u64,
}

impl FibonacciConfig {
    pub const fn new(count: usize) -> Self {
        Self {
            count,
            jitter: 0.0,
            seed: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FibonacciError {
    TooFewPoints,
    InvalidJitter,
}

impl fmt::Display for FibonacciError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooFewPoints => formatter.write_str("a sphere requires at least four points"),
            Self::InvalidJitter => formatter.write_str("jitter must be finite and between 0 and 1"),
        }
    }
}

impl std::error::Error for FibonacciError {}

/// Generates approximately uniform points on the unit sphere.
///
/// Jitter is an angular displacement relative to average point spacing. Each
/// point derives its random values from `(seed, index)`, so results do not
/// depend on scheduling or CPU thread count.
pub fn fibonacci_sphere(config: FibonacciConfig) -> Result<Vec<Vec3>, FibonacciError> {
    if config.count < 4 {
        return Err(FibonacciError::TooFewPoints);
    }
    if !config.jitter.is_finite() || !(0.0..=1.0).contains(&config.jitter) {
        return Err(FibonacciError::InvalidJitter);
    }

    let point = |index| fibonacci_point(index, config);
    let points = if config.count >= PARALLEL_THRESHOLD {
        (0..config.count).into_par_iter().map(point).collect()
    } else {
        (0..config.count).map(point).collect()
    };

    Ok(points)
}

fn fibonacci_point(index: usize, config: FibonacciConfig) -> Vec3 {
    let count = config.count as f32;
    let mut cos_theta = 1.0 - 2.0 * (index as f32 + 0.5) / count;
    let mut theta = cos_theta.clamp(-1.0, 1.0).acos();
    let mut phi = std::f32::consts::TAU * index as f32 / GOLDEN_RATIO;

    if config.jitter > 0.0 {
        let spacing = (4.0 * std::f32::consts::PI / count).sqrt();
        let random = RandomStream::new(config.seed, 0);
        theta = (theta + random.signed_f32(index as u64, 0) * config.jitter * spacing)
            .clamp(0.001, std::f32::consts::PI - 0.001);
        phi += random.signed_f32(index as u64, 1) * config.jitter * spacing;
        cos_theta = theta.cos();
    }

    let sin_theta = theta.sin();
    Vec3::new(sin_theta * phi.cos(), cos_theta, sin_theta * phi.sin()).normalized()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_parameters() {
        assert_eq!(
            fibonacci_sphere(FibonacciConfig::new(3)),
            Err(FibonacciError::TooFewPoints)
        );

        let mut config = FibonacciConfig::new(4);
        config.jitter = 1.01;
        assert_eq!(fibonacci_sphere(config), Err(FibonacciError::InvalidJitter));
    }

    #[test]
    fn points_lie_on_unit_sphere() {
        let mut config = FibonacciConfig::new(32_768);
        config.jitter = 1.0;
        config.seed = 42;

        for point in fibonacci_sphere(config).unwrap() {
            assert!((point.length() - 1.0).abs() < 1.0e-5);
        }
    }

    #[test]
    fn unjittered_distribution_is_centered() {
        let points = fibonacci_sphere(FibonacciConfig::new(4_096)).unwrap();
        let sum = points
            .into_iter()
            .fold(Vec3::ZERO, |sum, point| sum + point);

        assert!((sum * (1.0 / 4_096.0)).length() < 1.0e-3);
    }

    #[test]
    fn seed_controls_jitter_only() {
        let mut config = FibonacciConfig::new(128);
        let unjittered = fibonacci_sphere(config).unwrap();
        config.seed = 99;
        assert_eq!(unjittered, fibonacci_sphere(config).unwrap());

        config.jitter = 0.5;
        let first = fibonacci_sphere(config).unwrap();
        assert_eq!(first, fibonacci_sphere(config).unwrap());
        config.seed = 100;
        assert_ne!(first, fibonacci_sphere(config).unwrap());
    }

    #[test]
    fn output_is_independent_of_thread_count() {
        let mut config = FibonacciConfig::new(PARALLEL_THRESHOLD);
        config.jitter = 0.5;
        config.seed = 7;

        let one = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(|| fibonacci_sphere(config).unwrap());
        let four = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap()
            .install(|| fibonacci_sphere(config).unwrap());

        assert_eq!(one, four);
    }
}
