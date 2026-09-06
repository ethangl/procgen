use crate::{
    AtmosphericCirculation, AtmosphericCirculationConfig, AtmosphericCirculationError,
    AtmosphericCirculationInputs, Cryosphere, CryosphereConfig, CryosphereError, CryosphereInputs,
    MoistureTransport, MoistureTransportConfig, MoistureTransportError, MoistureTransportInputs,
    RadiativeEquilibriumConfig, RadiativeEquilibriumError, RadiativeEquilibriumTemperature,
    STEFAN_BOLTZMANN_CONSTANT, SeasonalThermalConfig, SeasonalThermalError, SeasonalThermalInputs,
    SeasonalThermalResponse, SolarForcing, Surface, derive_atmospheric_circulation,
    derive_cryosphere, derive_moisture_transport,
    derive_radiative_equilibrium_temperature_with_albedo,
    derive_seasonal_thermal_response_with_albedo, validate_range,
};
use procgen_planet::Planet;
use procgen_sphere_mesh::SphereMesh;
use std::{fmt, ops::RangeInclusive};

pub const CLIMATE_COUPLING_ITERATION_LIMIT_RANGE: RangeInclusive<usize> = 1..=256;
pub const CLIMATE_COUPLING_TOLERANCE_RANGE: RangeInclusive<f64> = 0.0..=10_000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimateAlbedoConfig {
    pub land: f64,
    pub ocean: f64,
    pub snow: f64,
    pub ice: f64,
}

impl ClimateAlbedoConfig {
    pub const EARTHLIKE: Self = Self {
        land: 0.20,
        ocean: 0.06,
        snow: 0.80,
        ice: 0.60,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimateCouplingConfig {
    pub maximum_iterations: usize,
    /// Fraction of each newly diagnosed albedo update applied per iteration.
    pub under_relaxation: f64,
    pub albedo_tolerance: f64,
    pub temperature_tolerance_kelvin: f64,
    pub precipitation_tolerance_kg_per_m2_per_day: f64,
    pub cover_fraction_tolerance: f64,
    pub albedo: ClimateAlbedoConfig,
    pub radiative_equilibrium: RadiativeEquilibriumConfig,
    pub seasonal_thermal: SeasonalThermalConfig,
    pub atmospheric_circulation: AtmosphericCirculationConfig,
    pub moisture_transport: MoistureTransportConfig,
    pub cryosphere: CryosphereConfig,
}

impl ClimateCouplingConfig {
    pub const EARTHLIKE: Self = Self {
        maximum_iterations: 64,
        under_relaxation: 0.5,
        albedo_tolerance: 0.05,
        temperature_tolerance_kelvin: 3.0,
        precipitation_tolerance_kg_per_m2_per_day: 0.1,
        cover_fraction_tolerance: 0.15,
        albedo: ClimateAlbedoConfig::EARTHLIKE,
        radiative_equilibrium: RadiativeEquilibriumConfig::EARTHLIKE,
        seasonal_thermal: SeasonalThermalConfig::EARTHLIKE,
        atmospheric_circulation: AtmosphericCirculationConfig::EARTHLIKE,
        moisture_transport: MoistureTransportConfig::EARTHLIKE,
        cryosphere: CryosphereConfig::EARTHLIKE,
    };
}

#[derive(Clone, Copy, Debug)]
pub struct ClimateCouplingInputs<'a> {
    pub planet: Planet,
    pub solar_forcing: &'a SolarForcing,
    pub solar_forcing_config: crate::SolarForcingConfig,
    pub final_elevation: &'a [f32],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimateCouplingDiagnostics {
    pub iterations: usize,
    pub albedo_residual_rms: f64,
    pub temperature_change_rms_kelvin: f64,
    pub precipitation_change_rms_kg_per_m2_per_day: f64,
    pub cover_fraction_change_rms: f64,
    pub maximum_radiative_balance_error_watts_per_square_meter: f64,
    pub moisture_mass_balance_error_kg_per_m2: f64,
    pub snow_mass_balance_error_kg_per_m2: f64,
    pub land_ice_mass_balance_kg_per_m2: f64,
    pub sea_ice_cover_balance_error: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClimateCoupling {
    pub cell_albedo: Vec<f32>,
    pub radiative_equilibrium: RadiativeEquilibriumTemperature,
    pub seasonal_thermal: SeasonalThermalResponse,
    pub atmospheric_circulation: AtmosphericCirculation,
    pub moisture_transport: MoistureTransport,
    pub cryosphere: Cryosphere,
    pub diagnostics: ClimateCouplingDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClimateCouplingError {
    IterationLimitConfig,
    UnderRelaxation,
    Tolerance,
    Albedo,
    ElevationCells,
    Elevation,
    RadiativeEquilibrium(RadiativeEquilibriumError),
    SeasonalThermal(SeasonalThermalError),
    AtmosphericCirculation(AtmosphericCirculationError),
    MoistureTransport(MoistureTransportError),
    Cryosphere(CryosphereError),
    IterationLimit,
}

impl fmt::Display for ClimateCouplingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ClimateCouplingError as Error;
        match self {
            Error::IterationLimitConfig => formatter
                .write_str("climate coupling iteration limit is outside its supported range"),
            Error::UnderRelaxation => formatter
                .write_str("climate coupling under-relaxation must be finite and in (0, 1]"),
            Error::Tolerance => {
                formatter.write_str("climate coupling tolerances must be finite and nonnegative")
            }
            Error::Albedo => formatter
                .write_str("land, ocean, snow, and ice albedos must be finite and in [0, 1]"),
            Error::ElevationCells => formatter.write_str("elevation must match the mesh cells"),
            Error::Elevation => formatter.write_str("elevation must contain only finite values"),
            Error::RadiativeEquilibrium(error) => error.fmt(formatter),
            Error::SeasonalThermal(error) => error.fmt(formatter),
            Error::AtmosphericCirculation(error) => error.fmt(formatter),
            Error::MoistureTransport(error) => error.fmt(formatter),
            Error::Cryosphere(error) => error.fmt(formatter),
            Error::IterationLimit => formatter
                .write_str("coupled climate did not converge before the hard iteration limit"),
        }
    }
}

impl std::error::Error for ClimateCouplingError {}

macro_rules! coupling_error_from {
    ($source:ty, $variant:ident) => {
        impl From<$source> for ClimateCouplingError {
            fn from(error: $source) -> Self {
                Self::$variant(error)
            }
        }
    };
}

coupling_error_from!(RadiativeEquilibriumError, RadiativeEquilibrium);
coupling_error_from!(SeasonalThermalError, SeasonalThermal);
coupling_error_from!(AtmosphericCirculationError, AtmosphericCirculation);
coupling_error_from!(MoistureTransportError, MoistureTransport);
coupling_error_from!(CryosphereError, Cryosphere);

#[derive(Clone)]
struct PreviousFields {
    temperature: Vec<f32>,
    precipitation: Vec<f32>,
    snow: Vec<f32>,
    land_ice: Vec<f32>,
    sea_ice: Vec<f32>,
}

/// Iterates only the explicit surface-albedo feedback around the five existing
/// climate stages. Every invocation starts from the configured fully covered
/// snow/ice albedo, selecting the cold fixed-point branch reproducibly without
/// retaining state between generation runs.
pub fn derive_coupled_climate(
    mesh: &SphereMesh,
    inputs: ClimateCouplingInputs<'_>,
    config: ClimateCouplingConfig,
) -> Result<ClimateCoupling, ClimateCouplingError> {
    validate(mesh, inputs, config)?;
    let mut albedo = inputs
        .final_elevation
        .iter()
        .map(|&elevation| match Surface::from_elevation(elevation) {
            Surface::Land => config.albedo.snow as f32,
            Surface::Ocean => config.albedo.ice as f32,
        })
        .collect::<Vec<_>>();
    let mut previous: Option<PreviousFields> = None;

    for iteration in 1..=config.maximum_iterations {
        let radiative_equilibrium = derive_radiative_equilibrium_temperature_with_albedo(
            mesh,
            inputs.solar_forcing,
            config.radiative_equilibrium,
            &albedo,
        )?;
        let seasonal_thermal = derive_seasonal_thermal_response_with_albedo(
            mesh,
            SeasonalThermalInputs {
                planet: inputs.planet,
                solar_forcing: inputs.solar_forcing_config,
                radiative_equilibrium: config.radiative_equilibrium,
                final_elevation: inputs.final_elevation,
            },
            config.seasonal_thermal,
            &albedo,
        )?;
        let atmospheric_circulation = derive_atmospheric_circulation(
            mesh,
            AtmosphericCirculationInputs {
                planet: inputs.planet,
                selected_temperature_kelvin: &seasonal_thermal.selected_temperature_kelvin,
                final_elevation: inputs.final_elevation,
            },
            config.atmospheric_circulation,
        )?;
        let moisture_transport = derive_moisture_transport(
            mesh,
            MoistureTransportInputs {
                planet: inputs.planet,
                selected_temperature_kelvin: &seasonal_thermal.selected_temperature_kelvin,
                final_elevation: inputs.final_elevation,
                cell_wind_meters_per_second: &atmospheric_circulation.cell_wind_meters_per_second,
            },
            config.moisture_transport,
        )?;
        let cryosphere = derive_cryosphere(
            mesh,
            CryosphereInputs {
                annual_temperature_samples_kelvin: &seasonal_thermal
                    .annual_temperature_samples_kelvin,
                annual_sample_count: seasonal_thermal.annual_sample_count,
                selected_orbital_phase: inputs.solar_forcing_config.orbital_phase,
                orbital_period_days: config.seasonal_thermal.orbital_period_days,
                precipitation_kg_per_m2_per_day: &moisture_transport
                    .cell_precipitation_kg_per_m2_per_day,
                final_elevation: inputs.final_elevation,
            },
            config.cryosphere,
        )?;

        let target_albedo = compose_albedo(inputs.final_elevation, &cryosphere, config.albedo);
        let albedo_residual = rms_difference(mesh, &albedo, &target_albedo);
        let (temperature_change, precipitation_change, cover_change) = previous
            .as_ref()
            .map(|previous| {
                (
                    rms_difference(
                        mesh,
                        &previous.temperature,
                        &seasonal_thermal.selected_temperature_kelvin,
                    ),
                    rms_difference(
                        mesh,
                        &previous.precipitation,
                        &moisture_transport.cell_precipitation_kg_per_m2_per_day,
                    ),
                    rms_difference(mesh, &previous.snow, &cryosphere.cell_snow_cover_fraction)
                        .max(rms_difference(
                            mesh,
                            &previous.land_ice,
                            &cryosphere.cell_land_ice_cover_fraction,
                        ))
                        .max(rms_difference(
                            mesh,
                            &previous.sea_ice,
                            &cryosphere.cell_sea_ice_cover_fraction,
                        )),
                )
            })
            .unwrap_or((0.0, 0.0, 0.0));

        if albedo_residual <= config.albedo_tolerance
            && temperature_change <= config.temperature_tolerance_kelvin
            && precipitation_change <= config.precipitation_tolerance_kg_per_m2_per_day
            && cover_change <= config.cover_fraction_tolerance
        {
            let diagnostics = ClimateCouplingDiagnostics {
                iterations: iteration,
                albedo_residual_rms: albedo_residual,
                temperature_change_rms_kelvin: temperature_change,
                precipitation_change_rms_kg_per_m2_per_day: precipitation_change,
                cover_fraction_change_rms: cover_change,
                maximum_radiative_balance_error_watts_per_square_meter: radiative_balance_error(
                    inputs.solar_forcing,
                    &albedo,
                    &radiative_equilibrium,
                    config.radiative_equilibrium.emissivity,
                ),
                moisture_mass_balance_error_kg_per_m2: moisture_transport
                    .diagnostics
                    .mass_balance_error_kg_per_m2,
                snow_mass_balance_error_kg_per_m2: cryosphere
                    .diagnostics
                    .snow_mass_balance_error_kg_per_m2,
                land_ice_mass_balance_kg_per_m2: cryosphere
                    .diagnostics
                    .land_ice_mass_balance_kg_per_m2,
                sea_ice_cover_balance_error: cryosphere.diagnostics.sea_ice_cover_balance_error,
            };
            return Ok(ClimateCoupling {
                cell_albedo: albedo,
                radiative_equilibrium,
                seasonal_thermal,
                atmospheric_circulation,
                moisture_transport,
                cryosphere,
                diagnostics,
            });
        }

        previous = Some(PreviousFields {
            temperature: seasonal_thermal.selected_temperature_kelvin,
            precipitation: moisture_transport.cell_precipitation_kg_per_m2_per_day,
            snow: cryosphere.cell_snow_cover_fraction,
            land_ice: cryosphere.cell_land_ice_cover_fraction,
            sea_ice: cryosphere.cell_sea_ice_cover_fraction,
        });
        for (value, target) in albedo.iter_mut().zip(target_albedo) {
            *value += (target - *value) * config.under_relaxation as f32;
        }
    }
    Err(ClimateCouplingError::IterationLimit)
}

fn validate(
    mesh: &SphereMesh,
    inputs: ClimateCouplingInputs<'_>,
    config: ClimateCouplingConfig,
) -> Result<(), ClimateCouplingError> {
    validate_range(
        config.maximum_iterations,
        &CLIMATE_COUPLING_ITERATION_LIMIT_RANGE,
        ClimateCouplingError::IterationLimitConfig,
    )?;
    if !config.under_relaxation.is_finite()
        || !(0.0..=1.0).contains(&config.under_relaxation)
        || config.under_relaxation == 0.0
    {
        return Err(ClimateCouplingError::UnderRelaxation);
    }
    for tolerance in [
        config.albedo_tolerance,
        config.temperature_tolerance_kelvin,
        config.precipitation_tolerance_kg_per_m2_per_day,
        config.cover_fraction_tolerance,
    ] {
        validate_range(
            tolerance,
            &CLIMATE_COUPLING_TOLERANCE_RANGE,
            ClimateCouplingError::Tolerance,
        )?;
    }
    for albedo in [
        config.albedo.land,
        config.albedo.ocean,
        config.albedo.snow,
        config.albedo.ice,
    ] {
        validate_range(albedo, &(0.0..=1.0), ClimateCouplingError::Albedo)?;
    }
    if inputs.final_elevation.len() != mesh.cell_count() {
        return Err(ClimateCouplingError::ElevationCells);
    }
    if inputs
        .final_elevation
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(ClimateCouplingError::Elevation);
    }
    Ok(())
}

fn compose_albedo(
    elevation: &[f32],
    cryosphere: &Cryosphere,
    config: ClimateAlbedoConfig,
) -> Vec<f32> {
    elevation
        .iter()
        .enumerate()
        .map(
            |(cell, &elevation)| match Surface::from_elevation(elevation) {
                Surface::Ocean => blend(
                    config.ocean,
                    config.ice,
                    f64::from(cryosphere.cell_sea_ice_cover_fraction[cell]),
                ) as f32,
                Surface::Land => {
                    let ice = blend(
                        config.land,
                        config.ice,
                        f64::from(cryosphere.cell_land_ice_cover_fraction[cell]),
                    );
                    blend(
                        ice,
                        config.snow,
                        f64::from(cryosphere.cell_snow_cover_fraction[cell]),
                    ) as f32
                }
            },
        )
        .collect()
}

fn blend(base: f64, cover: f64, fraction: f64) -> f64 {
    base + (cover - base) * fraction
}

fn rms_difference(mesh: &SphereMesh, left: &[f32], right: &[f32]) -> f64 {
    let total_area = mesh
        .cell_areas
        .iter()
        .map(|&area| f64::from(area))
        .sum::<f64>();
    (left
        .iter()
        .zip(right)
        .zip(&mesh.cell_areas)
        .map(|((&left, &right), &area)| f64::from(left - right).powi(2) * f64::from(area))
        .sum::<f64>()
        / total_area)
        .sqrt()
}

fn radiative_balance_error(
    forcing: &SolarForcing,
    albedo: &[f32],
    temperature: &RadiativeEquilibriumTemperature,
    emissivity: f64,
) -> f64 {
    forcing
        .annual_mean_insolation
        .iter()
        .zip(albedo)
        .zip(&temperature.annual_effective_temperature_kelvin)
        .map(|((&forcing, &albedo), &temperature)| {
            let absorbed = f64::from(forcing) * (1.0 - f64::from(albedo));
            let emitted = emissivity * STEFAN_BOLTZMANN_CONSTANT * f64::from(temperature).powi(4);
            (absorbed - emitted).abs()
        })
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SolarForcingConfig, derive_solar_forcing};
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::SphericalDelaunay;

    fn mesh(count: usize) -> SphereMesh {
        let points = fibonacci_sphere(FibonacciConfig::new(count)).unwrap();
        let delaunay = SphericalDelaunay::build(points).unwrap();
        SphereMesh::from_delaunay(&delaunay, 1.0).unwrap()
    }

    fn setup(
        cell_count: usize,
        luminosity_scale: f64,
    ) -> (
        SphereMesh,
        Planet,
        SolarForcing,
        SolarForcingConfig,
        Vec<f32>,
    ) {
        let mesh = mesh(cell_count);
        let mut planet = Planet::EARTH;
        planet.star.luminosity_watts *= luminosity_scale;
        let solar_config = SolarForcingConfig {
            annual_sample_count: 12,
            ..Default::default()
        };
        let forcing = derive_solar_forcing(&mesh, planet, solar_config).unwrap();
        let elevation = mesh
            .cell_centers
            .iter()
            .map(|center| if center.y > 0.0 { 0.7 } else { 0.3 })
            .collect();
        (mesh, planet, forcing, solar_config, elevation)
    }

    fn config() -> ClimateCouplingConfig {
        ClimateCouplingConfig {
            maximum_iterations: 48,
            albedo_tolerance: 2.0e-4,
            temperature_tolerance_kelvin: 0.05,
            precipitation_tolerance_kg_per_m2_per_day: 2.0e-4,
            cover_fraction_tolerance: 2.0e-4,
            moisture_transport: MoistureTransportConfig {
                step_count: 12,
                ..MoistureTransportConfig::EARTHLIKE
            },
            cryosphere: CryosphereConfig {
                maximum_iterations: 128,
                closure_tolerance: 1.0e-5,
                ..CryosphereConfig::EARTHLIKE
            },
            ..ClimateCouplingConfig::EARTHLIKE
        }
    }

    fn run(
        cell_count: usize,
        luminosity_scale: f64,
        config: ClimateCouplingConfig,
    ) -> Result<ClimateCoupling, ClimateCouplingError> {
        let (mesh, planet, forcing, solar_config, elevation) = setup(cell_count, luminosity_scale);
        derive_coupled_climate(
            &mesh,
            ClimateCouplingInputs {
                planet,
                solar_forcing: &forcing,
                solar_forcing_config: solar_config,
                final_elevation: &elevation,
            },
            config,
        )
    }

    #[test]
    fn repeated_runs_are_exactly_deterministic() {
        let first = run(32, 16.0, config()).unwrap();
        let second = run(32, 16.0, config()).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn equal_surface_albedos_are_zero_feedback() {
        let mut config = config();
        config.albedo = ClimateAlbedoConfig {
            land: 0.3,
            ocean: 0.3,
            snow: 0.3,
            ice: 0.3,
        };
        let result = run(32, 1.0, config).unwrap();
        assert_eq!(result.diagnostics.iterations, 1);
        assert_eq!(result.cell_albedo, vec![0.3_f32; 32]);
        assert_eq!(result.diagnostics.albedo_residual_rms, 0.0);
    }

    #[test]
    fn warm_world_has_no_cryosphere_feedback() {
        let result = run(32, 16.0, config()).unwrap();
        assert!(
            result
                .cryosphere
                .cell_snow_cover_fraction
                .iter()
                .all(|&value| value == 0.0)
        );
        assert!(
            result
                .cryosphere
                .cell_land_ice_cover_fraction
                .iter()
                .all(|&value| value == 0.0)
        );
        assert!(
            result
                .cryosphere
                .cell_sea_ice_cover_fraction
                .iter()
                .all(|&value| value == 0.0)
        );
    }

    #[test]
    fn frozen_world_raises_albedo() {
        let (mesh, planet, forcing, solar_config, mut elevation) = setup(32, 0.05);
        elevation.fill(0.0);
        let result = derive_coupled_climate(
            &mesh,
            ClimateCouplingInputs {
                planet,
                solar_forcing: &forcing,
                solar_forcing_config: solar_config,
                final_elevation: &elevation,
            },
            config(),
        )
        .unwrap();
        assert!(
            result
                .cryosphere
                .cell_snow_cover_fraction
                .iter()
                .chain(&result.cryosphere.cell_sea_ice_cover_fraction)
                .any(|&value| value > 0.0)
        );
        assert!(
            result
                .cell_albedo
                .iter()
                .all(|&value| value > ClimateAlbedoConfig::EARTHLIKE.ocean as f32)
        );
    }

    #[test]
    fn converges_within_configured_tolerances() {
        let config = config();
        let result = run(32, 16.0, config).unwrap();
        assert!(result.diagnostics.iterations < config.maximum_iterations);
        assert!(result.diagnostics.albedo_residual_rms <= config.albedo_tolerance);
        assert!(
            result.diagnostics.temperature_change_rms_kelvin <= config.temperature_tolerance_kelvin
        );
        assert!(
            result
                .diagnostics
                .precipitation_change_rms_kg_per_m2_per_day
                <= config.precipitation_tolerance_kg_per_m2_per_day
        );
        assert!(result.diagnostics.cover_fraction_change_rms <= config.cover_fraction_tolerance);
        assert!(
            result
                .diagnostics
                .maximum_radiative_balance_error_watts_per_square_meter
                < 1.0,
            "{}",
            result
                .diagnostics
                .maximum_radiative_balance_error_watts_per_square_meter
        );
        assert!(
            result
                .diagnostics
                .moisture_mass_balance_error_kg_per_m2
                .abs()
                <= 1.0e-10
        );
    }

    #[test]
    fn hard_iteration_limit_is_reported() {
        let mut config = config();
        config.maximum_iterations = 1;
        config.albedo_tolerance = 0.0;
        assert_eq!(
            run(32, 16.0, config),
            Err(ClimateCouplingError::IterationLimit)
        );
    }
}
