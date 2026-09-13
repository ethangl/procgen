use procgen_core::Vec3;

pub fn positions() -> impl Iterator<Item = Vec3> {
    (0..96).map(|i| {
        Vec3::new(
            ((i * 37 % 97) as f32 + 0.37) / 48.5 - 1.0,
            ((i * 13 % 89) as f32 + 0.19) / 44.5 - 1.0,
            ((i * 53 % 83) as f32 + 0.61) / 41.5 - 1.0,
        )
    })
}
