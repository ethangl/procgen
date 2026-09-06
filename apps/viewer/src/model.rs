use bevy::prelude::*;
use procgen_climate::{SolarForcing, SolarForcingConfig, derive_solar_forcing};
use procgen_geology::{
    CratonField, CratonFieldConfig, GeologicalElevation, GeologicalElevationConfig,
    GeologicalElevationInputs, HotspotField, HotspotFieldConfig, IsostaticAdjustment,
    IsostaticAdjustmentConfig, IsostaticAdjustmentInputs, OceanicPeakField, OceanicPeakFieldConfig,
    SedimentaryBasinField, SedimentaryBasinFieldConfig, VolcanicArcField, VolcanicArcFieldConfig,
    compose_geological_elevation, derive_craton_field, derive_isostatic_adjustment,
    derive_oceanic_peak_field, derive_sedimentary_basin_field, derive_volcanic_arc_field,
    generate_hotspot_field,
};
use procgen_planet::Planet;
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, SphericalDelaunay};
use procgen_tectonics::{
    BaseElevation, BaseElevationConfig, BoundaryClassification, BoundaryDeformation,
    BoundaryDeformationConfig, CoarseElevation, CoarseElevationConfig, CrustClassification,
    CrustClassificationConfig, PlateEvolution, PlateEvolutionConfig, PlateEvolutionDiagnostics,
    PlateKinematics, PlateKinematicsConfig, PlatePartition, PlatePartitionConfig, SeafloorAge,
    SeafloorAgeConfig, classify_crust, compose_coarse_elevation, derive_base_elevation,
    derive_boundary_deformation, derive_seafloor_age, evolve_plate_ownership,
    generate_plate_kinematics, partition_plates,
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
    pub evolution: PlateEvolutionConfig,
    pub seafloor_age: SeafloorAgeConfig,
    pub base_elevation: BaseElevationConfig,
    pub deformation: BoundaryDeformationConfig,
    pub elevation: CoarseElevationConfig,
    pub hotspots: HotspotFieldConfig,
    pub oceanic_peaks: OceanicPeakFieldConfig,
    pub volcanic_arcs: VolcanicArcFieldConfig,
    pub cratons: CratonFieldConfig,
    pub basins: SedimentaryBasinFieldConfig,
    pub geological_elevation: GeologicalElevationConfig,
    pub isostasy: IsostaticAdjustmentConfig,
    pub planet: Planet,
    pub solar_forcing: SolarForcingConfig,
}

impl Default for GenerationSettings {
    fn default() -> Self {
        Self {
            fibonacci: FibonacciConfig {
                jitter: 0.5,
                seed: 7,
                ..FibonacciConfig::new(32_768)
            },
            plates: PlatePartitionConfig {
                major_plate_count: 11,
                minor_plate_count: 111,
                major_head_start_rounds: 5,
                seed: 7,
            },
            crust: CrustClassificationConfig {
                target_ocean_fraction: 0.75,
                ..CrustClassificationConfig::new(7)
            },
            kinematics: PlateKinematicsConfig::new(7),
            evolution: PlateEvolutionConfig {
                step_count: 11,
                ..Default::default()
            },
            seafloor_age: SeafloorAgeConfig::default(),
            base_elevation: BaseElevationConfig::default(),
            deformation: BoundaryDeformationConfig::default(),
            elevation: CoarseElevationConfig::default(),
            hotspots: HotspotFieldConfig::new(7),
            oceanic_peaks: OceanicPeakFieldConfig::new(7),
            volcanic_arcs: VolcanicArcFieldConfig::default(),
            cratons: CratonFieldConfig::default(),
            basins: SedimentaryBasinFieldConfig::default(),
            geological_elevation: GeologicalElevationConfig::default(),
            isostasy: IsostaticAdjustmentConfig::default(),
            planet: Planet::EARTH,
            solar_forcing: SolarForcingConfig::default(),
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
    pub boundaries: BoundaryClassification,
    pub evolution: PlateEvolutionDiagnostics,
    pub seafloor_age: SeafloorAge,
    pub base_elevation: BaseElevation,
    pub deformation: BoundaryDeformation,
    pub elevation: CoarseElevation,
    pub hotspots: HotspotField,
    pub oceanic_peaks: OceanicPeakField,
    pub volcanic_arcs: VolcanicArcField,
    pub cratons: CratonField,
    pub basins: SedimentaryBasinField,
    pub geological_elevation: GeologicalElevation,
    pub isostasy: IsostaticAdjustment,
    pub solar_forcing: SolarForcing,
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
        let initial_plates = timings.record("Plate partition", || {
            partition_plates(&voronoi, config.plates)
        })?;
        let crust = timings.record("Crust", || {
            classify_crust(&voronoi, &initial_plates, config.crust)
        })?;
        let kinematics = timings.record("Plate kinematics", || {
            generate_plate_kinematics(initial_plates.plate_count, config.kinematics)
        })?;
        let evolution_result = timings.record("Plate evolution", || {
            evolve_plate_ownership(
                &voronoi,
                &initial_plates,
                &crust,
                &kinematics,
                config.evolution,
            )
        })?;
        let PlateEvolution {
            partition: plates,
            boundaries,
            diagnostics: evolution,
        } = evolution_result;
        let seafloor_age = timings.record("Seafloor age", || {
            derive_seafloor_age(&voronoi, &plates, &crust, &boundaries, config.seafloor_age)
        })?;
        let base_elevation = timings.record("Base elevation", || {
            derive_base_elevation(&seafloor_age, config.base_elevation)
        })?;
        let deformation = timings.record("Boundary deformation", || {
            derive_boundary_deformation(&voronoi, &plates, &crust, &boundaries, config.deformation)
        })?;
        let elevation = timings.record("Tectonic elevation", || {
            compose_coarse_elevation(&voronoi, &base_elevation, &deformation, config.elevation)
        })?;
        let hotspots = timings.record("Mantle hotspots", || {
            generate_hotspot_field(&voronoi, &plates, &kinematics, config.hotspots)
        })?;
        let oceanic_peaks = timings.record("Oceanic peaks", || {
            derive_oceanic_peak_field(&voronoi, &hotspots, &seafloor_age, config.oceanic_peaks)
        })?;
        let volcanic_arcs = timings.record("Volcanic arcs", || {
            derive_volcanic_arc_field(&voronoi, &plates, &crust, &boundaries, config.volcanic_arcs)
        })?;
        let cratons = timings.record("Cratons", || {
            derive_craton_field(&voronoi, &plates, &crust, &elevation, config.cratons)
        })?;
        let basins = timings.record("Sedimentary basins", || {
            derive_sedimentary_basin_field(&voronoi, &plates, &crust, &elevation, config.basins)
        })?;
        let geological_elevation = timings.record("Geological elevation", || {
            compose_geological_elevation(
                &voronoi,
                GeologicalElevationInputs {
                    tectonic_elevation: &elevation,
                    hotspots: &hotspots,
                    volcanic_arcs: &volcanic_arcs,
                    cratons: &cratons,
                    basins: &basins,
                    continental_base: config.base_elevation.continental_base,
                },
                config.geological_elevation,
            )
        })?;
        let isostasy = timings.record("Isostatic adjustment", || {
            derive_isostatic_adjustment(
                &voronoi,
                IsostaticAdjustmentInputs {
                    plates: &plates,
                    crust: &crust,
                    boundaries: &boundaries,
                    cratons: &cratons,
                    basins: &basins,
                    geological_elevation: &geological_elevation,
                },
                config.isostasy,
            )
        })?;
        let solar_forcing = timings.record("Solar forcing", || {
            derive_solar_forcing(&voronoi, config.planet, config.solar_forcing)
        })?;

        Ok(Self {
            voronoi,
            plates,
            crust,
            kinematics,
            boundaries,
            evolution,
            seafloor_age,
            base_elevation,
            deformation,
            elevation,
            hotspots,
            oceanic_peaks,
            volcanic_arcs,
            cratons,
            basins,
            geological_elevation,
            isostasy,
            solar_forcing,
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
    use procgen_tectonics::{CrustClass, PlateMigrationConfig};

    #[test]
    fn generates_consistent_viewer_counts() {
        let world = GeneratedWorld::generate(GenerationSettings {
            fibonacci: FibonacciConfig::new(128),
            plates: PlatePartitionConfig::new(4, 4),
            crust: CrustClassificationConfig::new(7),
            kinematics: PlateKinematicsConfig::new(9),
            ..GenerationSettings::default()
        })
        .unwrap();

        assert_eq!(world.voronoi.cell_count(), 128);
        assert_eq!(world.voronoi.vertex_count(), 252);
        assert_eq!(world.voronoi.edge_count(), 378);
        assert!(world.crust.plate_count(CrustClass::Oceanic) > 0);
        assert!(world.crust.plate_count(CrustClass::Continental) > 0);
        assert!(world.evolution.migrated_cell_count > 0);
        assert_eq!(
            world.seafloor_age.cell_ages.len(),
            world.voronoi.cell_count()
        );
        assert!(world.seafloor_age.diagnostics.oceanic_cell_count > 0);
        assert_eq!(
            world.base_elevation.cell_elevations.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world.deformation.cell_deformation.len(),
            world.voronoi.cell_count()
        );
        assert!(world.deformation.diagnostics.affected_cell_count() > 0);
        assert_eq!(
            world.elevation.cell_elevations.len(),
            world.voronoi.cell_count()
        );
        assert!(world.elevation.diagnostics.minimum >= 0.0);
        assert!(world.elevation.diagnostics.maximum <= 1.0);
        assert_eq!(
            world.hotspots.cell_intensities.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world.hotspots.hotspots.len(),
            world.config.hotspots.hotspot_count
        );
        assert_eq!(
            world.oceanic_peaks.cell_densities.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world.volcanic_arcs.cell_strengths.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world.cratons.cell_strengths.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(world.basins.cell_basins.len(), world.voronoi.cell_count());
        assert_eq!(
            world.geological_elevation.cell_elevations.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world.isostasy.cell_support.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world.isostasy.cell_elevations.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world.solar_forcing.daily_mean_insolation.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world.solar_forcing.annual_mean_insolation.len(),
            world.voronoi.cell_count()
        );
    }

    #[test]
    fn regeneration_message_replaces_the_active_world() {
        let mut app = App::new();
        let current = GeneratedWorld::generate(GenerationSettings {
            fibonacci: FibonacciConfig::new(32),
            plates: PlatePartitionConfig::new(2, 2),
            crust: CrustClassificationConfig::new(7),
            kinematics: PlateKinematicsConfig::new(3),
            ..GenerationSettings::default()
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
            evolution: PlateEvolutionConfig {
                step_count: 8,
                migration: PlateMigrationConfig {
                    minimum_convergence: 0.4,
                },
            },
            seafloor_age: SeafloorAgeConfig { ridge_less_age: 16 },
            base_elevation: BaseElevationConfig {
                cooling_age: 12,
                ..Default::default()
            },
            deformation: BoundaryDeformationConfig {
                saturation_speed: 1.5,
                ..Default::default()
            },
            elevation: CoarseElevationConfig {
                smoothing_passes: 4,
                ..Default::default()
            },
            hotspots: HotspotFieldConfig {
                hotspot_count: 9,
                maximum_trail_cells: 6,
                seed: 5,
            },
            oceanic_peaks: OceanicPeakFieldConfig {
                maximum_young_age: 6,
                seamount_density_scale: 0.8,
                abyssal_hill_density_scale: 0.4,
                maximum_position_offset: 0.7,
                maximum_seamount_height: 0.9,
                maximum_abyssal_hill_height: 0.2,
                seed: 13,
            },
            volcanic_arcs: VolcanicArcFieldConfig {
                minimum_boundary_edges: 2,
                inland_offset_cells: 3,
                peak_density_divisor: 3,
                strength_saturation: 0.75,
            },
            cratons: CratonFieldConfig {
                minimum_boundary_distance: 4,
                ramp_width: 5,
            },
            basins: SedimentaryBasinFieldConfig {
                maximum_elevation: 0.62,
                minimum_cell_count: 4,
                maximum_ocean_perimeter_fraction: 0.4,
            },
            geological_elevation: GeologicalElevationConfig {
                hotspot_uplift: 0.1,
                volcanic_arc_uplift: 0.15,
                craton_flattening: 0.6,
                basin_flattening: 0.7,
            },
            isostasy: IsostaticAdjustmentConfig {
                adjustment_strength: 0.5,
                maximum_boundary_distance: 6,
                ..Default::default()
            },
            planet: Planet::EARTH,
            solar_forcing: SolarForcingConfig {
                orbital_phase: 0.25,
                annual_sample_count: 48,
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
        assert_eq!(world.config.evolution, requested.evolution);
        assert_eq!(world.config.seafloor_age, requested.seafloor_age);
        assert_eq!(world.config.base_elevation, requested.base_elevation);
        assert_eq!(world.config.deformation, requested.deformation);
        assert_eq!(world.config.elevation, requested.elevation);
        assert_eq!(world.config.hotspots, requested.hotspots);
        assert_eq!(world.config.oceanic_peaks, requested.oceanic_peaks);
        assert_eq!(world.config.volcanic_arcs, requested.volcanic_arcs);
        assert_eq!(world.config.cratons, requested.cratons);
        assert_eq!(world.config.basins, requested.basins);
        assert_eq!(
            world.config.geological_elevation,
            requested.geological_elevation
        );
        assert_eq!(world.config.isostasy, requested.isostasy);
        assert_eq!(world.config.planet, requested.planet);
        assert_eq!(world.config.solar_forcing, requested.solar_forcing);
        assert_eq!(world.voronoi.cell_count(), requested.fibonacci.count);
        assert_eq!(world.plates.plate_count, requested.plates.plate_count());
    }
}
