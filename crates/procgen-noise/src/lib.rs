//! Deterministic, backend-neutral noise primitives.

use procgen_core::{Vec3, hash_u32};

/// A cubic-lattice gradient-noise sample and its spatial derivative.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoiseSample3 {
    pub value: f32,
    pub derivative: Vec3,
}

/// Samples cubic-lattice gradient noise with a quintic interpolation curve.
///
/// The supplied seed is narrowed once before lattice addressing. Positions
/// whose floored components fit in `i32` are supported. Lattice hashing uses
/// the signed coordinates' bit patterns, making the integer path directly
/// reproducible in WGSL and CUDA.
pub fn gradient_noise_3d(seed: u64, position: Vec3) -> NoiseSample3 {
    let seed = narrow_seed_to_u32(seed);
    let cell = [
        position.x.floor() as i32,
        position.y.floor() as i32,
        position.z.floor() as i32,
    ];
    let offset = Vec3::new(
        position.x - cell[0] as f32,
        position.y - cell[1] as f32,
        position.z - cell[2] as f32,
    );
    let (wx, dwx) = axis_weights(offset.x);
    let (wy, dwy) = axis_weights(offset.y);
    let (wz, dwz) = axis_weights(offset.z);

    let mut value = 0.0;
    let mut derivative = Vec3::ZERO;

    for corner_z in 0..2 {
        for corner_y in 0..2 {
            for corner_x in 0..2 {
                let gradient = lattice_gradient(
                    seed,
                    cell[0].wrapping_add(corner_x as i32),
                    cell[1].wrapping_add(corner_y as i32),
                    cell[2].wrapping_add(corner_z as i32),
                );
                let displacement =
                    offset - Vec3::new(corner_x as f32, corner_y as f32, corner_z as f32);
                let contribution = gradient.dot(displacement);
                let weight = wx[corner_x] * wy[corner_y] * wz[corner_z];

                value += contribution * weight;
                derivative = derivative
                    + gradient * weight
                    + Vec3::new(
                        dwx[corner_x] * wy[corner_y] * wz[corner_z],
                        wx[corner_x] * dwy[corner_y] * wz[corner_z],
                        wx[corner_x] * wy[corner_y] * dwz[corner_z],
                    ) * contribution;
            }
        }
    }

    NoiseSample3 { value, derivative }
}

// The fixed domain tags keep this conversion distinct from lattice addresses.
// Both halves of the workspace-standard seed participate in the result.
const fn narrow_seed_to_u32(seed: u64) -> u32 {
    const SEED_DOMAIN_TAG: u32 = 0x5345_4544; // ASCII "SEED"
    const NOISE_DOMAIN_TAG: u32 = 0x4E4F_4953; // ASCII "NOIS"

    hash_u32(
        seed as u32,
        (seed >> 32) as u32,
        SEED_DOMAIN_TAG,
        NOISE_DOMAIN_TAG,
    )
}

fn lattice_gradient(seed: u32, x: i32, y: i32, z: i32) -> Vec3 {
    GRADIENTS[(hash_u32(seed, x as u32, y as u32, z as u32) % GRADIENTS.len() as u32) as usize]
}

const GRADIENTS: [Vec3; 12] = [
    Vec3::new(1.0, 1.0, 0.0),
    Vec3::new(-1.0, 1.0, 0.0),
    Vec3::new(1.0, -1.0, 0.0),
    Vec3::new(-1.0, -1.0, 0.0),
    Vec3::new(1.0, 0.0, 1.0),
    Vec3::new(-1.0, 0.0, 1.0),
    Vec3::new(1.0, 0.0, -1.0),
    Vec3::new(-1.0, 0.0, -1.0),
    Vec3::new(0.0, 1.0, 1.0),
    Vec3::new(0.0, -1.0, 1.0),
    Vec3::new(0.0, 1.0, -1.0),
    Vec3::new(0.0, -1.0, -1.0),
];

/// Quintic fade weights for the low and high corner on one axis and their derivatives.
fn axis_weights(value: f32) -> ([f32; 2], [f32; 2]) {
    let fade = value * value * value * (value * (value * 6.0 - 15.0) + 10.0);
    let fade_derivative = 30.0 * value * value * (value * (value - 2.0) + 1.0);
    ([1.0 - fade, fade], [-fade_derivative, fade_derivative])
}

#[cfg(test)]
mod tests {
    use super::*;

    const DERIVATIVE_TEST_SEED: u64 = 0xCAFE_BABE_DEAD_BEEF;

    #[test]
    fn seed_narrowing_has_stable_vectors_and_uses_both_halves() {
        let vectors = [
            (0_u64, 0x0127_A4EC_u32),
            (1_u64, 0x72CF_298C_u32),
            (0x0000_0001_0000_0000, 0xEF16_0C68_u32),
            (0x0123_4567_89AB_CDEF, 0x494A_B0F6_u32),
            (u64::MAX, 0x1AA8_79D0_u32),
        ];

        for (seed, expected) in vectors {
            assert_eq!(narrow_seed_to_u32(seed), expected, "seed {seed:#018x}");
        }
        assert_ne!(narrow_seed_to_u32(1), narrow_seed_to_u32(1_u64 << 32));
    }

    #[test]
    fn stable_samples() {
        let vectors = [
            (
                0,
                Vec3::new(0.25, 0.5, 0.75),
                [0x3EAF_F770, 0x3E5C_6E80, 0xBF11_328E, 0xBFA3_3BA0],
            ),
            (
                7,
                Vec3::new(-1.25, 2.5, 9.75),
                [0x3F0C_77C8, 0xBEC3_FCC0, 0xBEDC_7DBC, 0xBE7A_7D00],
            ),
            (
                0x0123_4567_89AB_CDEF,
                Vec3::new(12.345, -67.89, 0.125),
                [0xBEEA_265B, 0xBF28_D843, 0xBF14_8D6D, 0x3ED8_54D9],
            ),
        ];

        for (seed, position, expected) in vectors {
            let sample = gradient_noise_3d(seed, position);
            assert_eq!(
                [
                    sample.value.to_bits(),
                    sample.derivative.x.to_bits(),
                    sample.derivative.y.to_bits(),
                    sample.derivative.z.to_bits(),
                ],
                expected
            );
        }
    }

    #[test]
    fn analytic_derivative_matches_central_difference() {
        const STEP: f32 = 1.0e-3;
        const TOLERANCE: f32 = 2.0e-3;
        let positions = [
            Vec3::new(0.23, 0.47, 0.81),
            Vec3::new(-4.31, 2.72, -1.19),
            Vec3::new(18.125, -7.625, 3.375),
        ];

        for position in positions {
            let sample = gradient_noise_3d(DERIVATIVE_TEST_SEED, position);
            let finite_difference = Vec3::new(
                central_difference(position, Vec3::new(STEP, 0.0, 0.0)),
                central_difference(position, Vec3::new(0.0, STEP, 0.0)),
                central_difference(position, Vec3::new(0.0, 0.0, STEP)),
            );
            assert!((sample.derivative - finite_difference).length() < TOLERANCE);
        }
    }

    fn central_difference(position: Vec3, offset: Vec3) -> f32 {
        let positive = gradient_noise_3d(DERIVATIVE_TEST_SEED, position + offset).value;
        let negative = gradient_noise_3d(DERIVATIVE_TEST_SEED, position - offset).value;
        (positive - negative) / (2.0 * offset.length())
    }

    #[test]
    fn value_and_derivative_are_continuous_at_lattice_boundaries() {
        const EPSILON: f32 = 1.0e-4;
        let left = gradient_noise_3d(42, Vec3::new(3.0 - EPSILON, -2.25, 7.75));
        let boundary = gradient_noise_3d(42, Vec3::new(3.0, -2.25, 7.75));
        let right = gradient_noise_3d(42, Vec3::new(3.0 + EPSILON, -2.25, 7.75));

        assert!((left.value - boundary.value).abs() < 3.0e-4);
        assert!((right.value - boundary.value).abs() < 3.0e-4);
        assert!((left.derivative - boundary.derivative).length() < 5.0e-4);
        assert!((right.derivative - boundary.derivative).length() < 5.0e-4);

        let lattice = gradient_noise_3d(42, Vec3::new(3.0, -2.0, 8.0));
        assert_eq!(lattice.value, 0.0);
    }

    #[test]
    fn samples_are_deterministic_and_seeded() {
        let position = Vec3::new(-12.75, 4.5, 31.125);
        let first = gradient_noise_3d(19, position);

        assert_eq!(first, gradient_noise_3d(19, position));
        assert_ne!(first, gradient_noise_3d(20, position));
    }

    #[test]
    fn representative_samples_are_finite() {
        let coordinates = [-32_768.75, -1.0, -0.0, 0.0, 1.0, 32_768.25];

        for &x in &coordinates {
            for &y in &coordinates {
                let sample = gradient_noise_3d(u64::MAX, Vec3::new(x, y, -x));
                assert!(sample.value.is_finite());
                assert!(sample.derivative.x.is_finite());
                assert!(sample.derivative.y.is_finite());
                assert!(sample.derivative.z.is_finite());
            }
        }
    }
}
