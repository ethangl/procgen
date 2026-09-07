const WORD_0_BIAS: u32 = 0x9E37_79B9;
const WORD_1_MIX: u32 = 0x85EB_CA6B;
const WORD_2_MIX: u32 = 0xC2B2_AE35;
const WORD_3_MIX: u32 = 0x27D4_EB2F;

/// Cross-backend vectors for [`hash_u32`], stored as `([word0, ..., word3], hash)`.
///
/// The values remain provisional until the first noise implementation consumes
/// this primitive. CPU and GPU agreement tests should share this table rather
/// than duplicate its literals.
pub const HASH_U32_TEST_VECTORS: [([u32; 4], u32); 4] = [
    ([0, 0, 0, 0], 0x01FC_E552),
    ([7, 0, 0, 0], 0x3FD9_ABDB),
    ([7, 4, 1, 1], 0x158B_4830),
    ([u32::MAX; 4], 0x91DF_8AC1),
];

/// Hashes four explicitly addressed 32-bit words.
///
/// The output is stable for a `(word0, word1, word2, word3)` address and does
/// not depend on evaluation order. The implementation uses only `u32` xor,
/// shifts, and wrapping addition and multiplication, so the same expression
/// order and constants can be mirrored bit-for-bit in WGSL and CUDA.
pub const fn hash_u32(word0: u32, word1: u32, word2: u32, word3: u32) -> u32 {
    let value = word0.wrapping_add(WORD_0_BIAS)
        ^ word1.wrapping_mul(WORD_1_MIX)
        ^ word2.wrapping_mul(WORD_2_MIX)
        ^ word3.wrapping_mul(WORD_3_MIX);
    mix32(value)
}

// Chris Wellons's lowbias32 finalizer:
// https://nullprogram.com/blog/2018/07/31/
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
        for &([word0, word1, word2, word3], expected) in &HASH_U32_TEST_VECTORS {
            assert_eq!(hash_u32(word0, word1, word2, word3), expected);
        }
    }

    #[test]
    fn words_select_independent_values() {
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
