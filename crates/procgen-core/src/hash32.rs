const SEED_BIAS: u32 = 0x9E37_79B9;
const STREAM_MIX: u32 = 0x85EB_CA6B;
const ITEM_MIX: u32 = 0xC2B2_AE35;
const SAMPLE_MIX: u32 = 0x27D4_EB2F;

/// Hashes one explicitly addressed 32-bit sample.
///
/// The output is stable for a `(seed, stream, item, sample)` address and does
/// not depend on evaluation order. The implementation uses only `u32` xor,
/// shifts, and wrapping addition and multiplication, so the same expression
/// order and constants can be mirrored bit-for-bit in WGSL and CUDA.
pub const fn hash_u32(seed: u32, stream: u32, item: u32, sample: u32) -> u32 {
    let value = seed.wrapping_add(SEED_BIAS)
        ^ stream.wrapping_mul(STREAM_MIX)
        ^ item.wrapping_mul(ITEM_MIX)
        ^ sample.wrapping_mul(SAMPLE_MIX);
    mix32(value)
}

const fn mix32(mut value: u32) -> u32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7FEB_352D);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846C_A68B);
    value ^ (value >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_test_vectors() {
        assert_eq!(hash_u32(0, 0, 0, 0), 0x01FC_E552);
        assert_eq!(hash_u32(7, 0, 0, 0), 0x3FD9_ABDB);
        assert_eq!(hash_u32(7, 4, 1, 1), 0x158B_4830);
        assert_eq!(
            hash_u32(u32::MAX, u32::MAX, u32::MAX, u32::MAX),
            0x91DF_8AC1
        );
    }

    #[test]
    fn coordinates_select_independent_values() {
        let values = [
            hash_u32(42, 3, 8, 2),
            hash_u32(43, 3, 8, 2),
            hash_u32(42, 4, 8, 2),
            hash_u32(42, 3, 9, 2),
            hash_u32(42, 3, 8, 3),
        ];

        for (index, value) in values.iter().enumerate() {
            assert!(!values[..index].contains(value));
        }
    }
}
