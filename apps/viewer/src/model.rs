use crate::cache::WorldCache;
use bevy::prelude::*;
use procgen_climate::{
    AtmosphericCirculation, ClimateCoupling, ClimateCouplingConfig, ClimateCouplingDiagnostics,
    ClimateCouplingInputs, Cryosphere, MoistureTransport, RadiativeEquilibriumTemperature,
    SeasonalThermalResponse, SolarForcing, SolarForcingConfig, derive_coupled_climate,
    derive_solar_forcing,
};
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

#[derive(Clone, Copy, Debug, PartialEq, Resource)]
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
    /// Earth-like defaults are explicit and caller-editable.
    pub planet: Planet,
    pub solar_forcing: SolarForcingConfig,
    pub climate_coupling: ClimateCouplingConfig,
}

impl Default for GenerationSettings {
    fn default() -> Self {
        Self {
            fibonacci: FibonacciConfig {
                jitter: 0.5,
                seed: 7,
                ..FibonacciConfig::new(65_536)
            },
            plates: PlatePartitionConfig {
                major_plate_count: 6,
                minor_plate_count: 111,
                major_head_start_rounds: 6,
                growth_roughness: 99,
                seed: 7,
            },
            crust: CrustClassificationConfig {
                target_ocean_fraction: 0.75,
                ..CrustClassificationConfig::new(7)
            },
            kinematics: PlateKinematicsConfig::new(7),
            evolution: PlateEvolutionConfig {
                step_count: 9,
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
            climate_coupling: ClimateCouplingConfig::EARTHLIKE,
        }
    }
}

#[derive(Debug, Resource)]
pub enum GenerationStatus {
    StartupLoaded {
        duration: Duration,
    },
    StartupGenerated {
        duration: Duration,
        cache_notice: Option<String>,
    },
    Regenerated {
        cache_notice: Option<String>,
    },
    GenerationFailed {
        error: String,
    },
    CacheCleared {
        existed: bool,
    },
    CacheClearFailed {
        error: String,
    },
}

#[derive(Message, Default)]
pub struct RegenerateWorld;

#[derive(Message, Default)]
pub struct ClearWorldCache;

pub struct WorldModelPlugin;

impl Plugin for WorldModelPlugin {
    fn build(&self, app: &mut App) {
        let cache = app
            .world()
            .get_resource::<WorldCache>()
            .cloned()
            .unwrap_or_default();
        let fallback_settings = app
            .world()
            .get_resource::<GenerationSettings>()
            .copied()
            .unwrap_or_default();
        let (world, status) = load_or_generate_world(&cache, fallback_settings);
        let settings = world.config;

        app.insert_resource(cache)
            .insert_resource(settings)
            .insert_resource(status)
            .insert_resource(world);
        app.add_message::<RegenerateWorld>()
            .add_message::<ClearWorldCache>()
            .add_systems(
                Update,
                (
                    regenerate_world.run_if(on_message::<RegenerateWorld>),
                    clear_world_cache.run_if(on_message::<ClearWorldCache>),
                ),
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
    pub radiative_equilibrium: RadiativeEquilibriumTemperature,
    pub seasonal_thermal: SeasonalThermalResponse,
    pub atmospheric_circulation: AtmosphericCirculation,
    pub moisture_transport: MoistureTransport,
    pub cryosphere: Cryosphere,
    pub cell_albedo: Vec<f32>,
    pub climate_coupling_diagnostics: ClimateCouplingDiagnostics,
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
        let climate = timings.record("Coupled climate", || {
            derive_coupled_climate(
                &voronoi,
                ClimateCouplingInputs {
                    planet: config.planet,
                    solar_forcing: &solar_forcing,
                    solar_forcing_config: config.solar_forcing,
                    final_elevation: &isostasy.cell_elevations,
                },
                config.climate_coupling,
            )
        })?;
        let ClimateCoupling {
            cell_albedo,
            radiative_equilibrium,
            seasonal_thermal,
            atmospheric_circulation,
            moisture_transport,
            cryosphere,
            diagnostics: climate_coupling_diagnostics,
        } = climate;

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
            radiative_equilibrium,
            seasonal_thermal,
            atmospheric_circulation,
            moisture_transport,
            cryosphere,
            cell_albedo,
            climate_coupling_diagnostics,
            timings,
            config,
        })
    }
}

fn load_or_generate_world(
    cache: &WorldCache,
    fallback_settings: GenerationSettings,
) -> (GeneratedWorld, GenerationStatus) {
    let started = Instant::now();
    match cache.load() {
        Ok(world) => (
            world,
            GenerationStatus::StartupLoaded {
                duration: started.elapsed(),
            },
        ),
        Err(load_error) => {
            let (world, store_notice) = generate_and_store(fallback_settings, cache)
                .expect("default world generation must succeed");
            let mut notices = Vec::new();
            if !WorldCache::is_missing(&load_error) {
                notices.push(format!("Ignored world cache: {load_error}"));
            }
            if let Some(store_notice) = store_notice {
                notices.push(store_notice);
            }
            (
                world,
                GenerationStatus::StartupGenerated {
                    duration: started.elapsed(),
                    cache_notice: (!notices.is_empty()).then(|| notices.join(" ")),
                },
            )
        }
    }
}

fn generate_and_store(
    settings: GenerationSettings,
    cache: &WorldCache,
) -> Result<(GeneratedWorld, Option<String>), Box<dyn Error>> {
    let world = GeneratedWorld::generate(settings)?;
    let cache_notice = cache
        .store(&world)
        .err()
        .map(|error| format!("Could not save world cache: {error}"));
    Ok((world, cache_notice))
}

fn regenerate_world(
    settings: Res<GenerationSettings>,
    cache: Res<WorldCache>,
    mut world: ResMut<GeneratedWorld>,
    mut status: ResMut<GenerationStatus>,
) {
    match generate_and_store(*settings, &cache) {
        Ok((generated, cache_notice)) => {
            *world = generated;
            *status = GenerationStatus::Regenerated { cache_notice };
        }
        Err(error) => {
            *status = GenerationStatus::GenerationFailed {
                error: error.to_string(),
            }
        }
    }
}

fn clear_world_cache(cache: Res<WorldCache>, mut status: ResMut<GenerationStatus>) {
    *status = match cache.clear() {
        Ok(existed) => GenerationStatus::CacheCleared { existed },
        Err(error) => GenerationStatus::CacheClearFailed {
            error: error.to_string(),
        },
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{cache as test_cache, settings as test_settings};
    use procgen_tectonics::{CrustClass, PlateMigrationConfig};
    use std::fs;

    #[test]
    fn default_generation_profile_generates() {
        GeneratedWorld::generate(GenerationSettings::default()).unwrap();
    }

    #[test]
    fn generates_consistent_viewer_counts() {
        let world = GeneratedWorld::generate(GenerationSettings {
            fibonacci: FibonacciConfig::new(128),
            plates: PlatePartitionConfig::new(4, 4),
            crust: CrustClassificationConfig::new(7),
            kinematics: PlateKinematicsConfig::new(9),
            evolution: PlateEvolutionConfig {
                step_count: 11,
                ..Default::default()
            },
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
        assert_eq!(
            world
                .radiative_equilibrium
                .daily_effective_temperature_kelvin
                .len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world.seasonal_thermal.selected_temperature_kelvin.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world.seasonal_thermal.annual_mean_temperature_kelvin.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world.seasonal_thermal.annual_amplitude_kelvin.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world
                .atmospheric_circulation
                .cell_wind_meters_per_second
                .len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world
                .atmospheric_circulation
                .cell_temperature_gradient_kelvin_per_radian
                .len(),
            world.voronoi.cell_count()
        );
        assert!(
            world
                .atmospheric_circulation
                .diagnostics
                .wind_speed_meters_per_second
                .maximum
                > 0.0
        );
        assert!(
            world
                .atmospheric_circulation
                .diagnostics
                .speed_capped_cell_count
                < world.voronoi.cell_count()
        );
        assert_eq!(
            world.moisture_transport.cell_humidity_kg_per_m2.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world
                .moisture_transport
                .cell_precipitation_kg_per_m2_per_day
                .len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world.cryosphere.cell_snow_cover_fraction.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world.cryosphere.cell_land_ice_cover_fraction.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(
            world.cryosphere.cell_sea_ice_cover_fraction.len(),
            world.voronoi.cell_count()
        );
        assert_eq!(world.cell_albedo.len(), world.voronoi.cell_count());
        assert!(
            world.climate_coupling_diagnostics.iterations
                <= world.config.climate_coupling.maximum_iterations
        );
        assert!(
            world
                .moisture_transport
                .diagnostics
                .mass_balance_error_kg_per_m2
                .abs()
                <= 1.0e-10
        );
        assert_eq!(
            world
                .radiative_equilibrium
                .annual_effective_temperature_kelvin
                .len(),
            world.voronoi.cell_count()
        );
    }

    #[test]
    fn startup_loads_cached_world_and_restores_its_settings() {
        let (cache_dir, cache) = test_cache("startup-load");
        let cached_settings = test_settings(32, 41);
        let cached_world = GeneratedWorld::generate(cached_settings).unwrap();
        cache.store(&cached_world).unwrap();

        let mut app = App::new();
        app.insert_resource(cache)
            .insert_resource(test_settings(48, 42))
            .add_plugins(WorldModelPlugin);

        assert_eq!(
            *app.world().resource::<GenerationSettings>(),
            cached_settings
        );
        let world = app.world().resource::<GeneratedWorld>();
        assert_eq!(world.config, cached_settings);
        assert_eq!(world.voronoi.cell_count(), 32);
        assert!(world.timings.stages().is_empty());
        assert!(matches!(
            app.world().resource::<GenerationStatus>(),
            GenerationStatus::StartupLoaded { .. }
        ));

        fs::remove_dir_all(cache_dir).unwrap();
    }

    #[test]
    fn corrupt_startup_cache_is_nonfatal_and_replaced_after_generation() {
        let (cache_dir, cache) = test_cache("startup-corrupt");
        fs::create_dir_all(&cache_dir).unwrap();
        fs::write(cache_dir.join("world.bin"), b"not a snapshot").unwrap();
        let requested = test_settings(32, 43);

        let mut app = App::new();
        app.insert_resource(cache.clone())
            .insert_resource(requested)
            .add_plugins(WorldModelPlugin);

        assert_eq!(app.world().resource::<GeneratedWorld>().config, requested);
        let status = app.world().resource::<GenerationStatus>();
        assert!(matches!(
            status,
            GenerationStatus::StartupGenerated {
                cache_notice: Some(notice),
                ..
            } if notice.contains("Ignored")
        ));
        assert_eq!(cache.load().unwrap().config, requested);

        fs::remove_dir_all(cache_dir).unwrap();
    }

    #[test]
    fn explicit_regeneration_bypasses_cache_and_replaces_snapshot() {
        let mut app = App::new();
        let current = GeneratedWorld::generate(GenerationSettings {
            fibonacci: FibonacciConfig::new(32),
            plates: PlatePartitionConfig::new(2, 2),
            crust: CrustClassificationConfig::new(7),
            kinematics: PlateKinematicsConfig::new(3),
            evolution: PlateEvolutionConfig {
                step_count: 11,
                ..Default::default()
            },
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
            climate_coupling: ClimateCouplingConfig {
                maximum_iterations: 32,
                ..ClimateCouplingConfig::EARTHLIKE
            },
        };
        let (cache_dir, cache) = test_cache("regenerate");
        cache.store(&current).unwrap();
        app.insert_resource(cache.clone())
            .add_plugins(WorldModelPlugin);
        app.insert_resource(requested);

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
        assert_eq!(world.config.climate_coupling, requested.climate_coupling);
        assert_eq!(world.voronoi.cell_count(), requested.fibonacci.count);
        assert_eq!(world.plates.plate_count, requested.plates.plate_count());
        let cached_world = cache.load().unwrap();
        assert_eq!(cached_world.config, requested);
        assert_eq!(cached_world.voronoi.cell_count(), requested.fibonacci.count);
        assert!(matches!(
            app.world().resource::<GenerationStatus>(),
            GenerationStatus::Regenerated { .. }
        ));
        fs::remove_dir_all(cache_dir).unwrap();
    }
}
