use bevy::prelude::*;
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, SphericalDelaunay};
use procgen_tectonics::{
    BoundaryClassification, CrustClassification, CrustClassificationConfig, PlateKinematics,
    PlateKinematicsConfig, PlateMigration, PlateMigrationConfig, PlateMigrationError,
    PlatePartition, PlatePartitionConfig, classify_boundaries, classify_crust,
    generate_plate_kinematics, migrate_plates_once, partition_plates,
};
use std::{
    error::Error,
    time::{Duration, Instant},
};

pub const WORLD_RADIUS: f32 = 1.0;

#[derive(Clone, Copy, Debug, Resource)]
pub struct GenerationSettings {
    pub fibonacci: FibonacciConfig,
    pub plates: PlatePartitionConfig,
    pub crust: CrustClassificationConfig,
    pub kinematics: PlateKinematicsConfig,
    pub migration: PlateMigrationConfig,
}

impl Default for GenerationSettings {
    fn default() -> Self {
        Self {
            fibonacci: FibonacciConfig {
                jitter: 0.5,
                seed: 7,
                ..FibonacciConfig::new(2_048)
            },
            plates: PlatePartitionConfig {
                major_plate_count: 5,
                minor_plate_count: 11,
                major_head_start_rounds: 5,
                seed: 7,
            },
            crust: CrustClassificationConfig::new(7),
            kinematics: PlateKinematicsConfig::new(7),
            migration: PlateMigrationConfig::default(),
        }
    }
}

#[derive(Resource, Default)]
pub struct GenerationStatus {
    pub last_error: Option<String>,
}

#[derive(Message, Default)]
pub struct RegenerateWorld;

pub struct WorldModelPlugin;

impl Plugin for WorldModelPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<RegenerateWorld>()
            .init_resource::<GenerationSettings>()
            .init_resource::<GenerationStatus>()
            .init_resource::<GeneratedWorld>()
            .add_systems(
                Update,
                regenerate_world.run_if(on_message::<RegenerateWorld>),
            );
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StageTiming {
    pub label: &'static str,
    pub duration: Duration,
}

#[derive(Clone, Debug, Default)]
pub struct GenerationTimings {
    stages: Vec<StageTiming>,
}

impl GenerationTimings {
    fn record<T, E>(
        &mut self,
        label: &'static str,
        operation: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        let started = Instant::now();
        let result = operation();
        self.stages.push(StageTiming {
            label,
            duration: started.elapsed(),
        });
        result
    }

    pub fn stages(&self) -> &[StageTiming] {
        &self.stages
    }

    pub fn total(&self) -> Duration {
        self.stages.iter().map(|stage| stage.duration).sum()
    }
}

#[derive(Resource)]
pub struct GeneratedWorld {
    pub voronoi: SphereMesh,
    pub plates: PlatePartition,
    pub crust: CrustClassification,
    pub kinematics: PlateKinematics,
    pub migration: PlateMigration,
    pub boundaries: BoundaryClassification,
    pub timings: GenerationTimings,
    pub config: GenerationSettings,
}

impl GeneratedWorld {
    pub fn generate(config: GenerationSettings) -> Result<Self, Box<dyn Error>> {
        let mut timings = GenerationTimings::default();
        let points = timings.record("Sampling", || fibonacci_sphere(config.fibonacci))?;
        let delaunay = timings.record("Delaunay", || SphericalDelaunay::build(points))?;
        let voronoi = timings.record("Voronoi", || {
            SphereMesh::from_delaunay(&delaunay, WORLD_RADIUS)
        })?;
        let mut plates = timings.record("Plate partition", || {
            partition_plates(&voronoi, config.plates)
        })?;
        let crust = timings.record("Crust", || classify_crust(&voronoi, &plates, config.crust))?;
        let kinematics = timings.record("Plate kinematics", || {
            generate_plate_kinematics(plates.plate_count, config.kinematics)
        })?;
        let migration_boundaries = timings.record("Pre-migration boundaries", || {
            classify_boundaries(&voronoi, &plates, &kinematics)
        })?;
        let migration = timings.record("Plate migration", || {
            let migration = migrate_plates_once(
                &voronoi,
                &plates,
                &crust,
                &migration_boundaries,
                config.migration,
            )?;
            migration.apply(&mut plates);
            Ok::<_, PlateMigrationError>(migration)
        })?;
        let boundaries = timings.record("Boundaries", || {
            classify_boundaries(&voronoi, &plates, &kinematics)
        })?;
        Ok(Self {
            voronoi,
            plates,
            crust,
            kinematics,
            migration,
            boundaries,
            timings,
            config,
        })
    }
}

impl FromWorld for GeneratedWorld {
    fn from_world(world: &mut World) -> Self {
        let config = *world.resource::<GenerationSettings>();
        Self::generate(config).expect("default world generation must succeed")
    }
}

fn regenerate_world(
    settings: Res<GenerationSettings>,
    mut world: ResMut<GeneratedWorld>,
    mut status: ResMut<GenerationStatus>,
) {
    match GeneratedWorld::generate(*settings) {
        Ok(generated) => {
            *world = generated;
            status.last_error = None;
        }
        Err(error) => status.last_error = Some(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_tectonics::CrustClass;
    #[test]
    fn generates_consistent_viewer_counts() {
        let world = GeneratedWorld::generate(GenerationSettings {
            fibonacci: FibonacciConfig::new(128),
            plates: PlatePartitionConfig::new(4, 4),
            crust: CrustClassificationConfig::new(7),
            kinematics: PlateKinematicsConfig::new(9),
            migration: PlateMigrationConfig::default(),
        })
        .unwrap();

        assert_eq!(world.voronoi.cell_count(), 128);
        assert_eq!(world.voronoi.vertex_count(), 252);
        assert_eq!(world.voronoi.edge_count(), 378);
        assert!(world.crust.plate_count(CrustClass::Oceanic) > 0);
        assert!(world.crust.plate_count(CrustClass::Continental) > 0);
        assert!(world.migration.migrated_cell_count() > 0);
    }

    #[test]
    fn regeneration_message_replaces_the_active_world() {
        let mut app = App::new();
        let current = GeneratedWorld::generate(GenerationSettings {
            fibonacci: FibonacciConfig::new(32),
            plates: PlatePartitionConfig::new(2, 2),
            crust: CrustClassificationConfig::new(7),
            kinematics: PlateKinematicsConfig::new(3),
            migration: PlateMigrationConfig::default(),
        })
        .unwrap();
        let requested = GenerationSettings {
            fibonacci: FibonacciConfig::new(64),
            plates: PlatePartitionConfig::new(3, 3),
            crust: CrustClassificationConfig {
                target_ocean_fraction: 0.6,
                seed: 7,
            },
            kinematics: PlateKinematicsConfig::new(4),
            migration: PlateMigrationConfig {
                minimum_convergence: 0.4,
            },
        };
        app.insert_resource(current)
            .insert_resource(requested)
            .add_plugins(WorldModelPlugin);

        app.world_mut().write_message(RegenerateWorld);
        app.update();

        let world = app.world().resource::<GeneratedWorld>();
        assert_eq!(world.config.fibonacci, requested.fibonacci);
        assert_eq!(world.config.plates, requested.plates);
        assert_eq!(world.config.crust, requested.crust);
        assert_eq!(world.config.kinematics, requested.kinematics);
        assert_eq!(world.config.migration, requested.migration);
        assert_eq!(world.voronoi.cell_count(), requested.fibonacci.count);
        assert_eq!(world.plates.plate_count, requested.plates.plate_count());
    }
}
