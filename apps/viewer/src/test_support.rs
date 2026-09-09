use crate::{
    cache::WorldCache,
    model::{
        ClimateSettings, ClimateWorld, CompleteWorld, GeneratedWorld, GenerationSettings,
        GenerationTimings, GeologySettings, GeologyWorld, TectonicsSettings, TectonicsWorld,
        build_mesh,
    },
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

pub(crate) fn tectonics_settings(cell_count: usize, seed: u64) -> TectonicsSettings {
    TectonicsSettings {
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
        ..TectonicsSettings::default()
    }
}

/// Builds the mesh and generates the tectonics phase from one settings
/// profile, the sequencing `GeneratedWorld::generate_phase` owns outside tests.
pub(crate) fn tectonics_world(settings: TectonicsSettings) -> TectonicsWorld {
    let mut timings = GenerationTimings::default();
    let voronoi = build_mesh(settings.fibonacci, &mut timings).unwrap();
    TectonicsWorld::generate(voronoi, settings, timings).unwrap()
}

pub(crate) fn geology_settings(seed: u64) -> GeologySettings {
    GeologySettings {
        hotspots: HotspotFieldConfig {
            hotspot_count: 3,
            maximum_trail_cells: 4,
            seed,
        },
        oceanic_peaks: OceanicPeakFieldConfig::new(seed),
        ..GeologySettings::default()
    }
}

pub(crate) fn settings(cell_count: usize, seed: u64) -> GenerationSettings {
    GenerationSettings {
        tectonics: tectonics_settings(cell_count, seed),
        geology: geology_settings(seed),
        climate: ClimateSettings::default(),
    }
}

/// Every phase result for one settings profile, owned so tests can corrupt
/// individual fields before encoding a snapshot.
pub(crate) struct Fixture {
    pub tectonics: TectonicsWorld,
    pub geology: GeologyWorld,
    pub climate: ClimateWorld,
}

impl Fixture {
    pub(crate) fn generate(settings: GenerationSettings) -> Self {
        let tectonics = tectonics_world(settings.tectonics);
        let geology = GeologyWorld::generate(&tectonics, settings.geology).unwrap();
        let climate = ClimateWorld::generate(&tectonics, &geology, settings.climate).unwrap();
        Self {
            tectonics,
            geology,
            climate,
        }
    }

    pub(crate) fn new(cell_count: usize, seed: u64) -> Self {
        Self::generate(settings(cell_count, seed))
    }

    pub(crate) fn complete(&self) -> CompleteWorld<'_> {
        CompleteWorld {
            tectonics: &self.tectonics,
            geology: &self.geology,
            climate: &self.climate,
        }
    }

    pub(crate) fn into_world(self) -> GeneratedWorld {
        GeneratedWorld::from_phases(self.tectonics, self.geology, self.climate)
    }
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
