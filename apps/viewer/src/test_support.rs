use crate::{
    cache::WorldCache,
    model::{GeneratedWorld, GenerationSettings},
};
use procgen_geology::{HotspotFieldConfig, OceanicPeakFieldConfig};
use procgen_sphere::FibonacciConfig;
use procgen_tectonics::{
    CrustClassificationConfig, PlateEvolutionConfig, PlateKinematicsConfig, PlatePartitionConfig,
};
use std::{
    env,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) fn settings(cell_count: usize, seed: u64) -> GenerationSettings {
    GenerationSettings {
        fibonacci: FibonacciConfig {
            jitter: 0.25,
            seed,
            ..FibonacciConfig::new(cell_count)
        },
        plates: PlatePartitionConfig {
            seed,
            ..PlatePartitionConfig::new(2, 2)
        },
        crust: CrustClassificationConfig::new(seed),
        kinematics: PlateKinematicsConfig::new(seed),
        evolution: PlateEvolutionConfig {
            step_count: 4,
            ..Default::default()
        },
        hotspots: HotspotFieldConfig {
            hotspot_count: 3,
            maximum_trail_cells: 4,
            seed,
        },
        oceanic_peaks: OceanicPeakFieldConfig::new(seed),
        ..GenerationSettings::default()
    }
}

pub(crate) fn fixture(cell_count: usize, seed: u64) -> GeneratedWorld {
    GeneratedWorld::generate(settings(cell_count, seed)).unwrap()
}

pub(crate) fn cache(name: &str) -> (PathBuf, WorldCache) {
    let directory = env::temp_dir().join(format!(
        "procgen-viewer-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let cache = WorldCache::new(directory.join("world.bin"));
    (directory, cache)
}
