/// Grid steps per unit that `quantized_fingerprint` rounds a float onto, as a
/// power of two.
///
/// A step of 1/1024 is about a thousandth of a normalized field's range, which
/// is four orders of magnitude coarser than the last bit of an `f32` near one.
/// A field built from add, multiply, divide, and square root alone is
/// bit-identical on every machine, so nothing crosses a grid boundary; the
/// coarse grid is the margin that keeps the pin holding the algorithm even
/// when a later stage puts a less exact operation upstream of it.
const QUANTIZED_FINGERPRINT_BITS: u32 = 10;

/// Produces a stable FNV-1a fingerprint for deterministic test vectors.
///
/// Fingerprint integer test vectors only. Float bits are never pinned, because
/// libm results differ across machines. A float field is pinned through
/// [`quantized_fingerprint`] instead.
pub fn fingerprint(values: impl IntoIterator<Item = u64>) -> u64 {
    values
        .into_iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, value| {
            (hash ^ value).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

/// Fingerprints a float field by its step on a 1/1024 grid rather than by its
/// bits.
///
/// Scaling by a power of two is exact, so the grid step is an integer fact
/// about the value, and two machines can disagree about it only for a value
/// sitting within one part in ten thousand of a step boundary. Pinning
/// `to_bits()` instead would pin the toolchain: a single differing last bit
/// changes the whole hash.
pub fn quantized_fingerprint(values: impl IntoIterator<Item = f32>) -> u64 {
    let scale = (1_u32 << QUANTIZED_FINGERPRINT_BITS) as f32;
    fingerprint(
        values
            .into_iter()
            .map(|value| (value * scale).round() as i64 as u64),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_order_sensitive() {
        assert_eq!(fingerprint([1, 2, 3]), 15_035_938_162_879_559_083);
        assert_ne!(fingerprint([1, 2, 3]), fingerprint([3, 2, 1]));
    }

    #[test]
    fn quantizing_ignores_changes_below_the_grid_and_keeps_the_ones_above_it() {
        let step = 1.0 / (1_u32 << QUANTIZED_FINGERPRINT_BITS) as f32;
        // Mid-step values, so a last-bit perturbation cannot cross a boundary.
        let field = [0.125_5, -0.250_5, 0.875_5];
        let nudged = field.map(|value: f32| f32::from_bits(value.to_bits() + 1));

        assert_eq!(
            quantized_fingerprint(field),
            quantized_fingerprint(nudged),
            "a last-bit difference must not move the pin"
        );
        assert_ne!(
            quantized_fingerprint(field),
            quantized_fingerprint(field.map(|value| value + step)),
            "a whole grid step must move it"
        );
    }
}
