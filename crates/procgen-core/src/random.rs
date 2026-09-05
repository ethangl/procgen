use crate::Vec3;

const ITEM_MIX: u64 = 0xD1B5_4A32_D192_ED03;
const STREAM_MIX: u64 = 0x8CB9_2BA7_2F3D_8DD7;
const SAMPLE_STEP: u64 = 0x9E37_79B9_7F4A_7C15;

/// A deterministic, counter-addressable random stream.
///
/// Every value is addressed by `(seed, stream, item, sample)`, so results are
/// independent of evaluation order and thread scheduling. Stream identifiers
/// should be stable numeric constants owned by the consuming algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RandomStream {
    seed: u64,
    stream: u64,
}

impl RandomStream {
    pub const fn new(seed: u64, stream: u64) -> Self {
        Self { seed, stream }
    }

    pub const fn sample_u64(self, item: u64, sample: u64) -> u64 {
        let state = self.seed ^ item.wrapping_mul(ITEM_MIX) ^ self.stream.wrapping_mul(STREAM_MIX);
        mix64(state.wrapping_add(sample.wrapping_add(1).wrapping_mul(SAMPLE_STEP)))
    }

    /// Returns a reproducible value in `[0, 1)` using 24 significant bits.
    pub fn unit_f32(self, item: u64, sample: u64) -> f32 {
        (self.sample_u64(item, sample) >> 40) as f32 / (1_u32 << 24) as f32
    }

    /// Returns a reproducible value in `[-1, 1)`.
    pub fn signed_f32(self, item: u64, sample: u64) -> f32 {
        self.unit_f32(item, sample) * 2.0 - 1.0
    }

    /// Samples a uniformly distributed unit vector with stable axis ordering.
    pub fn unit_vector(self, item: u64) -> Vec3 {
        let z = self.signed_f32(item, 0);
        let theta = self.unit_f32(item, 1) * std::f32::consts::TAU;
        let ring = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(ring * theta.cos(), ring * theta.sin(), z)
    }
}

const fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_integer_test_vectors() {
        let stream = RandomStream::new(7, 0);
        assert_eq!(stream.sample_u64(0, 0), 7_191_089_600_892_374_487);
        assert_eq!(stream.sample_u64(1, 0), 6_951_516_134_914_417_455);
        assert_eq!(stream.sample_u64(1, 1), 15_068_862_340_908_800_810);
        assert_eq!(
            RandomStream::new(7, 4).sample_u64(1, 1),
            10_419_817_649_146_102_190
        );
    }

    #[test]
    fn coordinates_select_independent_values() {
        let stream = RandomStream::new(42, 3);
        let base = stream.sample_u64(8, 2);
        assert_ne!(base, RandomStream::new(43, 3).sample_u64(8, 2));
        assert_ne!(base, RandomStream::new(42, 4).sample_u64(8, 2));
        assert_ne!(base, stream.sample_u64(9, 2));
        assert_ne!(base, stream.sample_u64(8, 3));
    }

    #[test]
    fn float_ranges_are_explicit() {
        let stream = RandomStream::new(19, 2);
        for item in 0..1_000 {
            let unit = stream.unit_f32(item, 0);
            let signed = stream.signed_f32(item, 1);
            assert!((0.0..1.0).contains(&unit));
            assert!((-1.0..1.0).contains(&signed));
        }
    }

    #[test]
    fn unit_vectors_are_deterministic_and_normalized() {
        let stream = RandomStream::new(19, 2);
        assert_eq!(
            stream.unit_vector(7),
            RandomStream::new(19, 2).unit_vector(7)
        );
        for item in 0..1_000 {
            assert!((stream.unit_vector(item).length() - 1.0).abs() < 1.0e-6);
        }
    }
}
