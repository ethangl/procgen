use crate::{
    cache::WorldCache,
    model::{
        ClimateSettings, ClimateWorld, CompleteWorld, GeneratedWorld, GenerationSettings,
        GenerationTimings, GeologySettings, GeologyWorld, TectonicsSettings, TectonicsWorld,
        build_mesh,
    },
};
use procgen_geology::{
    CratonFieldConfig, HotspotFieldConfig, IsostaticAdjustmentConfig, OceanicPeakFieldConfig,
    VolcanicArcFieldConfig,
};
use procgen_sphere::FibonacciConfig;
use procgen_sphere_mesh::{DEFAULT_CELL_COUNT, hop_length};
use procgen_tectonics::{
    BaseElevationConfig, BoundaryDeformationConfig, BoundaryEffect, CoarseElevationConfig,
    ContinentalRiftProfile, CrustClassificationConfig, DEFAULT_STEP_DURATION, PlateEvolutionConfig,
    PlateKinematicsConfig, PlatePartitionConfig,
};
use std::{
    env,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

// Every length default was set against the 65,536-cell mesh, and these test
// meshes are coarse enough that a default of a few of its cells would round to
// one hop here. Each fixture therefore states the reach the default means,
// through `hop_length`, rather than taking the default itself, exactly as it
// does for the step duration: a fixture stands in for the default world, not
// for a world generated at thirty-two cells.

/// The default deformation profiles with every depth carried to a mesh of
/// `cell_count` cells.
fn scaled_deformation(cell_count: usize) -> BoundaryDeformationConfig {
    let scale = hop_length(cell_count, 1.0) / hop_length(DEFAULT_CELL_COUNT, 1.0);
    let default = BoundaryDeformationConfig::default();
    let deepen = |effect: BoundaryEffect| BoundaryEffect {
        depth: effect.depth * scale,
        ..effect
    };
    BoundaryDeformationConfig {
        convergent: deepen(default.convergent),
        transform: deepen(default.transform),
        collision: deepen(default.collision),
        trench: deepen(default.trench),
        island_arc: deepen(default.island_arc),
        rift: ContinentalRiftProfile {
            decay_depth: default.rift.decay_depth * scale,
            ..default.rift
        },
        ..default
    }
}

pub(crate) fn tectonics_settings(cell_count: usize, seed: u64) -> TectonicsSettings {
    TectonicsSettings {
        fibonacci: FibonacciConfig {
            jitter: 0.25,
            seed,
            ..FibonacciConfig::new(cell_count)
        },
        plates: PlatePartitionConfig {
            arc_count: 2,
            // Minor plates of roughly thirty-two cells, so the small test
            // meshes split into a handful of plates rather than one per cell.
            piece_fraction: 32.0 / cell_count as f32,
            seed,
            ..PlatePartitionConfig::default()
        },
        crust: CrustClassificationConfig::new(seed),
        kinematics: PlateKinematicsConfig::new(seed),
        evolution: PlateEvolutionConfig {
            step_count: 4,
            // The default duration is scaled to the 65,536-cell mesh. Cell
            // width goes as the reciprocal square root of cell count, so a
            // step on these much coarser test meshes has to be that much
            // longer to move a plate the same one cell.
            step_duration: DEFAULT_STEP_DURATION
                * (DEFAULT_CELL_COUNT as f32 / cell_count as f32).sqrt(),
            deformation: scaled_deformation(cell_count),
            ..Default::default()
        },
        base_elevation: BaseElevationConfig {
            margin_width: hop_length(cell_count, 3.0),
            ..BaseElevationConfig::default()
        },
        elevation: CoarseElevationConfig {
            smoothing_radius: hop_length(cell_count, 2.0),
            ..CoarseElevationConfig::default()
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

pub(crate) fn geology_settings(cell_count: usize, seed: u64) -> GeologySettings {
    GeologySettings {
        hotspots: HotspotFieldConfig {
            hotspot_count: 3,
            maximum_trail_length: hop_length(cell_count, 4.0),
            province_radius: hop_length(cell_count, 5.0),
            province_rim: hop_length(cell_count, 2.0),
            ..HotspotFieldConfig::new(seed)
        },
        oceanic_peaks: OceanicPeakFieldConfig::new(seed),
        volcanic_arcs: VolcanicArcFieldConfig {
            inland_offset: hop_length(cell_count, 2.0),
            ..VolcanicArcFieldConfig::default()
        },
        cratons: CratonFieldConfig {
            minimum_boundary_distance: hop_length(cell_count, 3.0),
            ramp_width: hop_length(cell_count, 3.0),
        },
        isostasy: IsostaticAdjustmentConfig {
            maximum_boundary_distance: hop_length(cell_count, 5.0),
            ..IsostaticAdjustmentConfig::default()
        },
        ..GeologySettings::default()
    }
}

pub(crate) fn settings(cell_count: usize, seed: u64) -> GenerationSettings {
    GenerationSettings {
        tectonics: tectonics_settings(cell_count, seed),
        geology: geology_settings(cell_count, seed),
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

    /// Generates every phase for one cell count and seed. Climate coupling
    /// does not reach a fixed point on every mesh this small, so each
    /// fixture's seed is one that converges at its cell count.
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
