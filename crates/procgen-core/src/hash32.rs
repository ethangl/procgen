const HASH32_SEED_BIAS: u32 = 0x9E37_79B9;
const HASH32_X_MIX: u32 = 0x85EB_CA6B;
const HASH32_Y_MIX: u32 = 0xC2B2_AE35;
const HASH32_Z_MIX: u32 = 0x27D4_EB2F;

/// Cross-backend vectors for [`hash_u32`], stored as `([seed, x, y, z], hash)`.
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

/// Hashes one cubic-lattice coordinate for a seed.
///
/// The output is stable for a `(seed, x, y, z)` address and does not depend on
/// evaluation order. The implementation uses only `u32` xor, shifts, and
/// wrapping addition and multiplication, so the same expression order and
/// constants can be mirrored bit-for-bit in WGSL and CUDA.
pub const fn hash_u32(seed: u32, x: u32, y: u32, z: u32) -> u32 {
    let value = seed.wrapping_add(HASH32_SEED_BIAS)
        ^ x.wrapping_mul(HASH32_X_MIX)
        ^ y.wrapping_mul(HASH32_Y_MIX)
        ^ z.wrapping_mul(HASH32_Z_MIX);
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
        for &([seed, x, y, z], expected) in &HASH_U32_TEST_VECTORS {
            assert_eq!(hash_u32(seed, x, y, z), expected);
        }
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
