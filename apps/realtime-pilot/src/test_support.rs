use procgen_core::{Vec3, hash_u32};

pub fn reference_noise_positions() -> impl Iterator<Item = Vec3> {
    (0..16_384).map(|i| {
        let axis = |axis| (hash_u32(902, i, axis, 0) >> 8) as f32 / 16_777_216.0 * 64.0 - 32.0;
        Vec3::new(axis(0), axis(1), axis(2))
    })
}

pub fn positions() -> impl Iterator<Item = Vec3> {
    (0..96).map(|i| {
        Vec3::new(
            ((i * 37 % 97) as f32 + 0.37) / 48.5 - 1.0,
            ((i * 13 % 89) as f32 + 0.19) / 44.5 - 1.0,
            ((i * 53 % 83) as f32 + 0.61) / 41.5 - 1.0,
        )
    })
}
