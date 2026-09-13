//! Area-sampled broad relief, separate from mesh extrema and cave floors.
use crate::PlanetField;
use procgen_core::Vec3;
use rayon::prelude::*;

pub const HEIGHT_DISTRIBUTION_SAMPLES: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct HeightDistribution {
    pub minimum: f32,
    pub p05: f32,
    pub median: f32,
    pub p95: f32,
    pub maximum: f32,
}

pub(crate) fn sample(field: &PlanetField) -> HeightDistribution {
    // Equal-area Fibonacci directions; this diagnostic never changes the field.
    let mut heights: Vec<_> = (0..HEIGHT_DISTRIBUTION_SAMPLES)
        .into_par_iter()
        .map(|i| {
            let z = 1.0 - 2.0 * (i as f32 + 0.5) / HEIGHT_DISTRIBUTION_SAMPLES as f32;
            let angle = i as f32 * 2.399_963_1;
            let r = (1.0 - z * z).sqrt();
            let direction = Vec3::new(r * angle.cos(), r * angle.sin(), z).normalized();
            field.height(direction)
        })
        .collect();
    heights.sort_unstable_by(f32::total_cmp);
    HeightDistribution {
        minimum: heights[0],
        p05: heights[HEIGHT_DISTRIBUTION_SAMPLES * 5 / 100],
        median: heights[HEIGHT_DISTRIBUTION_SAMPLES / 2],
        p95: heights[HEIGHT_DISTRIBUTION_SAMPLES * 95 / 100],
        maximum: heights[HEIGHT_DISTRIBUTION_SAMPLES - 1],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PLANET_PRESETS;

    #[test]
    fn presets_have_broad_relief_without_a_large_radius_offset() {
        for preset in &PLANET_PRESETS {
            for seed in [0, 42, 4_294_967_338] {
                let field = preset.planet.validate(seed).unwrap();
                let heights = sample(&field);
                let scale = preset.planet.terrain.height_scale;
                // The middle 90% must span at least half the configured height
                // scale. The previous composition reached only 0.11–0.16 times it.
                assert!(
                    heights.p95 - heights.p05 > scale * 0.5,
                    "{} {seed}: {heights:?}",
                    preset.id
                );
                assert!(
                    heights.median.abs() < scale * 0.25,
                    "{} {seed}: {heights:?}",
                    preset.id
                );
                assert!(heights.minimum >= -scale && heights.maximum <= scale);
            }
        }
    }

    #[test]
    fn distribution_is_schedule_independent_and_zero_scale_is_flat() {
        let mut config = PLANET_PRESETS[0].planet;
        let field = config.validate(42).unwrap();
        let run = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| sample(&field))
        };
        assert_eq!(run(1), run(4));
        config.terrain.height_scale = 0.0;
        let flat = sample(&config.validate(42).unwrap());
        assert_eq!(
            flat,
            HeightDistribution {
                minimum: 0.0,
                p05: 0.0,
                median: 0.0,
                p95: 0.0,
                maximum: 0.0
            }
        );
    }
}
