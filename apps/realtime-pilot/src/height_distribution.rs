//! Area-sampled broad relief, separate from mesh extrema and cave floors.
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

pub(crate) fn sample_heights(height: impl Fn(Vec3) -> f32 + Sync) -> HeightDistribution {
    // Equal-area Fibonacci directions; this diagnostic never changes the field.
    let mut heights: Vec<_> = (0..HEIGHT_DISTRIBUTION_SAMPLES)
        .into_par_iter()
        .map(|i| {
            let z = 1.0 - 2.0 * (i as f32 + 0.5) / HEIGHT_DISTRIBUTION_SAMPLES as f32;
            let angle = i as f32 * 2.399_963_1;
            let r = (1.0 - z * z).sqrt();
            let direction = Vec3::new(r * angle.cos(), r * angle.sin(), z).normalized();
            height(direction)
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
    use crate::PlanetDesignConfig;

    /// The starter design is the preset every new world begins from, so its
    /// relief must already be broad rather than a thin skin on the sphere.
    #[test]
    fn the_starter_design_has_broad_relief_centered_near_the_datum() {
        for seed in [0, 42, 4_294_967_338] {
            let config = PlanetDesignConfig::starter(seed);
            let field = config.validate().unwrap();
            let heights = field.height_distribution();
            let limit = config.height_limit_m;
            // The middle 90% must span at least half the configured height limit.
            assert!(
                heights.p95 - heights.p05 > limit * 0.5,
                "{seed}: {heights:?}"
            );
            assert!(heights.median.abs() < limit * 0.25, "{seed}: {heights:?}");
            assert!(heights.minimum >= -limit && heights.maximum <= limit);
        }
    }

    #[test]
    fn the_distribution_is_schedule_independent() {
        let field = PlanetDesignConfig::starter(42).validate().unwrap();
        let run = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| field.height_distribution())
        };
        assert_eq!(run(1), run(4));
    }
}
