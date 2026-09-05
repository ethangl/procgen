/// Produces a stable FNV-1a fingerprint for deterministic test vectors.
pub fn fingerprint(values: impl IntoIterator<Item = u64>) -> u64 {
    values
        .into_iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, value| {
            (hash ^ value).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_order_sensitive() {
        assert_eq!(fingerprint([1, 2, 3]), 15_035_938_162_879_559_083);
        assert_ne!(fingerprint([1, 2, 3]), fingerprint([3, 2, 1]));
    }
}
