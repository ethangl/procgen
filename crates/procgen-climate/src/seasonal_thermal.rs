use crate::{
    AreaWeightedSummary, RadiativeEquilibriumConfig, RadiativeEquilibriumError,
    radiative_equilibrium::{effective_temperature_kelvin, validate_config as validate_radiation},
    solar_forcing::{daily_mean_at_latitude_sine, orbital_state},
};
use procgen_planet::{Planet, PlanetValidationError};
use procgen_sphere_mesh::SphereMesh;
use std::{fmt, ops::RangeInclusive};

const SECONDS_PER_DAY: f64 = 86_400.0;
pub const THERMAL_SAMPLE_RANGE: RangeInclusive<usize> = 4..=4_096;
pub const THERMAL_CAPACITY_RANGE: RangeInclusive<f64> = 0.0..=1.0e12;
pub const ORBITAL_PERIOD_DAYS_RANGE: RangeInclusive<f64> = 0.01..=1.0e6;
const MINIMUM_POSITIVE_THERMAL_CAPACITY: f64 = 1.0;
const FIXED_POINT_ITERATION_LIMIT: usize = 48;
const FIXED_POINT_TOLERANCE_KELVIN: f64 = 1.0e-7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeasonalThermalConfig {
    /// Effective surface heat capacity of land in J m^-2 K^-1. Zero follows
    /// radiative equilibrium without lag.
    pub land_heat_capacity: f64,
    /// Effective surface heat capacity of ocean in J m^-2 K^-1. Zero follows
    /// radiative equilibrium without lag.
    pub ocean_heat_capacity: f64,
    /// Orbital period in Earth days. Orbital geometry deliberately does not
    /// infer this from an unmodelled stellar or planetary mass.
    pub orbital_period_days: f64,
    /// Uniform elapsed-time intervals used to integrate one orbit.
    pub sample_count: usize,
}

impl SeasonalThermalConfig {
    /// Convenient mixed-surface values, not hidden assumptions of the stage.
    pub const EARTHLIKE: Self = Self {
        land_heat_capacity: 5.0e7,
        ocean_heat_capacity: 4.0e8,
        orbital_period_days: 365.256_363_004,
        sample_count: 96,
    };
}

#[derive(Clone, Copy, Debug)]
pub struct SeasonalThermalInputs<'a> {
    pub planet: Planet,
    pub selected_orbital_phase: f64,
    pub radiative_equilibrium: RadiativeEquilibriumConfig,
    pub final_elevation: &'a [f32],
    pub sea_level: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeasonalThermalDiagnostics {
    pub selected_phase: AreaWeightedSummary,
    pub annual_mean: AreaWeightedSummary,
    pub annual_minimum: AreaWeightedSummary,
    pub annual_maximum: AreaWeightedSummary,
    pub annual_amplitude: AreaWeightedSummary,
    pub land_cell_count: usize,
    pub ocean_cell_count: usize,
    pub selected_land_area_weighted_mean_kelvin: Option<f64>,
    pub selected_ocean_area_weighted_mean_kelvin: Option<f64>,
    /// Largest absolute closure error after advancing the solved initial state
    /// through one complete orbit.
    pub maximum_periodic_closure_error_kelvin: f64,
    /// Largest number of Newton fixed-point refinements used by any cell.
    pub maximum_fixed_point_iterations: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeasonalThermalResponse {
    pub selected_temperature_kelvin: Vec<f32>,
    pub annual_mean_temperature_kelvin: Vec<f32>,
    pub annual_minimum_temperature_kelvin: Vec<f32>,
    pub annual_maximum_temperature_kelvin: Vec<f32>,
    pub annual_amplitude_kelvin: Vec<f32>,
    pub diagnostics: SeasonalThermalDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeasonalThermalError {
    Planet(PlanetValidationError),
    Radiation(RadiativeEquilibriumError),
    ElevationCells,
    Elevation,
    SeaLevel,
    OrbitalPhase,
    LandHeatCapacity,
    OceanHeatCapacity,
    OrbitalPeriod,
    SampleCount,
    PeriodicSteadyState,
    NumericalRange,
}

impl fmt::Display for SeasonalThermalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Planet(error) => error.fmt(formatter),
            Self::Radiation(error) => error.fmt(formatter),
            Self::ElevationCells => formatter.write_str("elevation must match the mesh cells"),
            Self::Elevation => formatter.write_str("elevation must contain only finite values"),
            Self::SeaLevel => formatter.write_str("sea level must be finite"),
            Self::OrbitalPhase => formatter.write_str("selected orbital phase must be finite"),
            Self::LandHeatCapacity => write!(
                formatter,
                "land heat capacity must be zero or between {} and {} J/m2/K",
                MINIMUM_POSITIVE_THERMAL_CAPACITY,
                THERMAL_CAPACITY_RANGE.end()
            ),
            Self::OceanHeatCapacity => write!(
                formatter,
                "ocean heat capacity must be zero or between {} and {} J/m2/K",
                MINIMUM_POSITIVE_THERMAL_CAPACITY,
                THERMAL_CAPACITY_RANGE.end()
            ),
            Self::OrbitalPeriod => write!(
                formatter,
                "orbital period must be finite and between {} and {} days",
                ORBITAL_PERIOD_DAYS_RANGE.start(),
                ORBITAL_PERIOD_DAYS_RANGE.end()
            ),
            Self::SampleCount => write!(
                formatter,
                "thermal sample count must be between {} and {}",
                THERMAL_SAMPLE_RANGE.start(),
                THERMAL_SAMPLE_RANGE.end()
            ),
            Self::PeriodicSteadyState => {
                formatter.write_str("seasonal response did not reach a periodic steady state")
            }
            Self::NumericalRange => {
                formatter.write_str("seasonal response is outside the finite f32 output range")
            }
        }
    }
}

impl std::error::Error for SeasonalThermalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Planet(error) => Some(error),
            Self::Radiation(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PlanetValidationError> for SeasonalThermalError {
    fn from(error: PlanetValidationError) -> Self {
        Self::Planet(error)
    }
}

impl From<RadiativeEquilibriumError> for SeasonalThermalError {
    fn from(error: RadiativeEquilibriumError) -> Self {
        Self::Radiation(error)
    }
}

/// Solves the isolated surface energy balance `C dT/dt = epsilon sigma
/// (T_eq^4 - T^4)` over a repeating orbit. Midpoint forcing and an implicit
/// emission step keep every update bounded between its previous temperature
/// and the phase-resolved radiative target. Each cell is solved independently.
pub fn derive_seasonal_thermal_response(
    mesh: &SphereMesh,
    inputs: SeasonalThermalInputs<'_>,
    config: SeasonalThermalConfig,
) -> Result<SeasonalThermalResponse, SeasonalThermalError> {
    let SeasonalThermalInputs {
        planet,
        selected_orbital_phase,
        radiative_equilibrium: radiative_config,
        final_elevation,
        sea_level,
    } = inputs;
    planet.validate()?;
    validate_radiation(radiative_config)?;
    validate_inputs(
        mesh,
        selected_orbital_phase,
        final_elevation,
        sea_level,
        config,
    )?;

    let sample_count = config.sample_count;
    let step_seconds = config.orbital_period_days * SECONDS_PER_DAY / sample_count as f64;
    let states = (0..sample_count)
        .map(|sample| orbital_state(planet, (sample as f64 + 0.5) / sample_count as f64))
        .collect::<Vec<_>>();
    let selected_phase = selected_orbital_phase.rem_euclid(1.0);
    let mut selected = Vec::with_capacity(mesh.cell_count());
    let mut means = Vec::with_capacity(mesh.cell_count());
    let mut minima = Vec::with_capacity(mesh.cell_count());
    let mut maxima = Vec::with_capacity(mesh.cell_count());
    let mut amplitudes = Vec::with_capacity(mesh.cell_count());
    let mut land = vec![false; mesh.cell_count()];
    let mut maximum_closure_error = 0.0_f64;
    let mut maximum_iterations = 0;
    let mut targets = Vec::with_capacity(sample_count);
    let mut endpoints = Vec::with_capacity(sample_count + 1);

    for cell in 0..mesh.cell_count() {
        land[cell] = final_elevation[cell] >= sea_level;
        let heat_capacity = if land[cell] {
            config.land_heat_capacity
        } else {
            config.ocean_heat_capacity
        };
        let latitude_sine = f64::from(mesh.cell_centers[cell].y / mesh.radius);
        let target = |state| {
            // Match the public phase-resolved forcing field's f32 contract
            // before applying the shared radiative-equilibrium law.
            let insolation = daily_mean_at_latitude_sine(latitude_sine, state) as f32;
            effective_temperature_kelvin(f64::from(insolation), radiative_config)
        };

        if heat_capacity == 0.0 {
            let mut sum = 0.0;
            let mut minimum = f64::INFINITY;
            let mut maximum = f64::NEG_INFINITY;
            for &state in &states {
                let temperature = target(state);
                sum += temperature;
                minimum = minimum.min(temperature);
                maximum = maximum.max(temperature);
            }
            push_outputs(
                &mut selected,
                &mut means,
                &mut minima,
                &mut maxima,
                &mut amplitudes,
                target(orbital_state(planet, selected_phase)),
                sum / sample_count as f64,
                minimum,
                maximum,
            )?;
            continue;
        }

        targets.clear();
        targets.extend(states.iter().copied().map(target));
        let (initial, iterations) = solve_periodic_initial(
            &targets,
            heat_capacity,
            step_seconds,
            radiative_config.emissivity,
        )?;
        maximum_iterations = maximum_iterations.max(iterations);
        let mut temperature = initial;
        endpoints.clear();
        endpoints.push(temperature);
        for &equilibrium in &targets {
            temperature = implicit_energy_step(
                temperature,
                equilibrium,
                heat_capacity,
                step_seconds,
                radiative_config.emissivity,
            );
            endpoints.push(temperature);
        }
        maximum_closure_error = maximum_closure_error.max((temperature - initial).abs());

        let orbit = &endpoints[..sample_count];
        let mean = orbit.iter().sum::<f64>() / sample_count as f64;
        let minimum = orbit.iter().copied().fold(f64::INFINITY, f64::min);
        let maximum = orbit.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let position = selected_phase * sample_count as f64;
        let lower = position.floor() as usize % sample_count;
        let fraction = position - position.floor();
        let selected_temperature =
            endpoints[lower] + (endpoints[lower + 1] - endpoints[lower]) * fraction;
        push_outputs(
            &mut selected,
            &mut means,
            &mut minima,
            &mut maxima,
            &mut amplitudes,
            selected_temperature,
            mean,
            minimum,
            maximum,
        )?;
    }

    let land_cell_count = land.iter().filter(|&&is_land| is_land).count();
    let ocean_cell_count = land.len() - land_cell_count;
    Ok(SeasonalThermalResponse {
        diagnostics: SeasonalThermalDiagnostics {
            selected_phase: AreaWeightedSummary::from_field(mesh, &selected),
            annual_mean: AreaWeightedSummary::from_field(mesh, &means),
            annual_minimum: AreaWeightedSummary::from_field(mesh, &minima),
            annual_maximum: AreaWeightedSummary::from_field(mesh, &maxima),
            annual_amplitude: AreaWeightedSummary::from_field(mesh, &amplitudes),
            land_cell_count,
            ocean_cell_count,
            selected_land_area_weighted_mean_kelvin: class_mean(mesh, &selected, &land, true),
            selected_ocean_area_weighted_mean_kelvin: class_mean(mesh, &selected, &land, false),
            maximum_periodic_closure_error_kelvin: maximum_closure_error,
            maximum_fixed_point_iterations: maximum_iterations,
        },
        selected_temperature_kelvin: selected,
        annual_mean_temperature_kelvin: means,
        annual_minimum_temperature_kelvin: minima,
        annual_maximum_temperature_kelvin: maxima,
        annual_amplitude_kelvin: amplitudes,
    })
}

fn validate_inputs(
    mesh: &SphereMesh,
    selected_phase: f64,
    elevations: &[f32],
    sea_level: f32,
    config: SeasonalThermalConfig,
) -> Result<(), SeasonalThermalError> {
    if !selected_phase.is_finite() {
        return Err(SeasonalThermalError::OrbitalPhase);
    }
    if elevations.len() != mesh.cell_count() {
        return Err(SeasonalThermalError::ElevationCells);
    }
    if elevations.iter().any(|value| !value.is_finite()) {
        return Err(SeasonalThermalError::Elevation);
    }
    if !sea_level.is_finite() {
        return Err(SeasonalThermalError::SeaLevel);
    }
    if !valid_heat_capacity(config.land_heat_capacity) {
        return Err(SeasonalThermalError::LandHeatCapacity);
    }
    if !valid_heat_capacity(config.ocean_heat_capacity) {
        return Err(SeasonalThermalError::OceanHeatCapacity);
    }
    if !config.orbital_period_days.is_finite()
        || !ORBITAL_PERIOD_DAYS_RANGE.contains(&config.orbital_period_days)
    {
        return Err(SeasonalThermalError::OrbitalPeriod);
    }
    if !THERMAL_SAMPLE_RANGE.contains(&config.sample_count) {
        return Err(SeasonalThermalError::SampleCount);
    }
    Ok(())
}

fn valid_heat_capacity(value: f64) -> bool {
    value == 0.0
        || (value.is_finite()
            && (MINIMUM_POSITIVE_THERMAL_CAPACITY..=*THERMAL_CAPACITY_RANGE.end()).contains(&value))
}

fn solve_periodic_initial(
    targets: &[f64],
    capacity: f64,
    step_seconds: f64,
    emissivity: f64,
) -> Result<(f64, usize), SeasonalThermalError> {
    let minimum = targets.iter().copied().fold(f64::INFINITY, f64::min);
    let maximum = targets.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if minimum == maximum {
        return Ok((minimum, 0));
    }
    let mut initial = targets.iter().sum::<f64>() / targets.len() as f64;
    for iteration in 1..=FIXED_POINT_ITERATION_LIMIT {
        let (end, derivative) =
            advance_orbit_with_derivative(initial, targets, capacity, step_seconds, emissivity);
        let residual = end - initial;
        if residual.abs() <= FIXED_POINT_TOLERANCE_KELVIN {
            return Ok((initial, iteration));
        }
        let slope = derivative - 1.0;
        if !slope.is_finite() || slope.abs() < f64::EPSILON {
            break;
        }
        initial = (initial - residual / slope).clamp(minimum, maximum);
    }
    Err(SeasonalThermalError::PeriodicSteadyState)
}

fn advance_orbit_with_derivative(
    initial: f64,
    targets: &[f64],
    capacity: f64,
    step_seconds: f64,
    emissivity: f64,
) -> (f64, f64) {
    let coefficient = step_seconds * emissivity * crate::STEFAN_BOLTZMANN_CONSTANT / capacity;
    let mut temperature = initial;
    let mut derivative = 1.0;
    for &equilibrium in targets {
        temperature = implicit_energy_step_with_coefficient(temperature, equilibrium, coefficient);
        derivative /= 1.0 + 4.0 * coefficient * temperature.powi(3);
    }
    (temperature, derivative)
}

fn implicit_energy_step(
    temperature: f64,
    equilibrium: f64,
    capacity: f64,
    step_seconds: f64,
    emissivity: f64,
) -> f64 {
    let coefficient = step_seconds * emissivity * crate::STEFAN_BOLTZMANN_CONSTANT / capacity;
    implicit_energy_step_with_coefficient(temperature, equilibrium, coefficient)
}

fn implicit_energy_step_with_coefficient(
    temperature: f64,
    equilibrium: f64,
    coefficient: f64,
) -> f64 {
    let rhs = temperature + coefficient * equilibrium.powi(4);
    let mut lower = temperature.min(equilibrium);
    let mut upper = temperature.max(equilibrium);
    let mut result = temperature;
    for _ in 0..48 {
        let residual = result + coefficient * result.powi(4) - rhs;
        if residual.abs() <= 1.0e-12 * rhs.abs().max(1.0) {
            break;
        }
        if residual > 0.0 {
            upper = result;
        } else {
            lower = result;
        }
        let derivative = 1.0 + 4.0 * coefficient * result.powi(3);
        let next = result - residual / derivative;
        result = if next.is_finite() && next > lower && next < upper {
            next
        } else {
            (lower + upper) * 0.5
        };
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn push_outputs(
    selected: &mut Vec<f32>,
    means: &mut Vec<f32>,
    minima: &mut Vec<f32>,
    maxima: &mut Vec<f32>,
    amplitudes: &mut Vec<f32>,
    selected_value: f64,
    mean: f64,
    minimum: f64,
    maximum: f64,
) -> Result<(), SeasonalThermalError> {
    let values = [selected_value, mean, minimum, maximum, maximum - minimum];
    if values
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0 || *value > f64::from(f32::MAX))
    {
        return Err(SeasonalThermalError::NumericalRange);
    }
    selected.push(selected_value as f32);
    means.push(mean as f32);
    minima.push(minimum as f32);
    maxima.push(maximum as f32);
    amplitudes.push((maximum - minimum) as f32);
    Ok(())
}

fn class_mean(mesh: &SphereMesh, values: &[f32], land: &[bool], class: bool) -> Option<f64> {
    let mut weighted_sum = 0.0;
    let mut area_sum = 0.0;
    for (cell, &value) in values.iter().enumerate() {
        if land[cell] == class {
            let area = f64::from(mesh.cell_areas[cell]);
            weighted_sum += f64::from(value) * area;
            area_sum += area;
        }
    }
    if area_sum > 0.0 {
        Some(weighted_sum / area_sum)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        SolarForcingConfig, derive_radiative_equilibrium_temperature, derive_solar_forcing,
    };
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::SphericalDelaunay;

    fn mesh(count: usize) -> SphereMesh {
        let points = fibonacci_sphere(FibonacciConfig::new(count)).unwrap();
        let delaunay = SphericalDelaunay::build(points).unwrap();
        SphereMesh::from_delaunay(&delaunay, 1.0).unwrap()
    }

    fn derive(
        mesh: &SphereMesh,
        elevations: &[f32],
        phase: f64,
        config: SeasonalThermalConfig,
    ) -> SeasonalThermalResponse {
        derive_seasonal_thermal_response(
            mesh,
            SeasonalThermalInputs {
                planet: Planet::EARTH,
                selected_orbital_phase: phase,
                radiative_equilibrium: RadiativeEquilibriumConfig::EARTHLIKE,
                final_elevation: elevations,
                sea_level: 0.5,
            },
            config,
        )
        .unwrap()
    }

    #[test]
    fn repeated_derivation_is_exactly_deterministic() {
        let mesh = mesh(128);
        let elevations = vec![0.6; mesh.cell_count()];
        let first = derive(&mesh, &elevations, 0.37, SeasonalThermalConfig::EARTHLIKE);
        let second = derive(&mesh, &elevations, 0.37, SeasonalThermalConfig::EARTHLIKE);
        assert_eq!(first, second);
    }

    #[test]
    fn solution_is_a_periodic_steady_state_and_phase_wraps() {
        let mesh = mesh(128);
        let elevations = vec![0.6; mesh.cell_count()];
        let first = derive(&mesh, &elevations, 0.37, SeasonalThermalConfig::EARTHLIKE);
        let next_orbit = derive(&mesh, &elevations, 1.37, SeasonalThermalConfig::EARTHLIKE);
        assert_eq!(first, next_orbit);
        assert!(first.diagnostics.maximum_periodic_closure_error_kelvin <= 1.0e-6);
        assert!(first.diagnostics.maximum_fixed_point_iterations > 0);
    }

    #[test]
    fn zero_inertia_follows_phase_resolved_radiative_temperature() {
        let mesh = mesh(64);
        let elevations = vec![0.6; mesh.cell_count()];
        let config = SeasonalThermalConfig {
            land_heat_capacity: 0.0,
            ocean_heat_capacity: 0.0,
            ..SeasonalThermalConfig::EARTHLIKE
        };
        let thermal = derive(&mesh, &elevations, 0.31, config);
        let forcing = derive_solar_forcing(
            &mesh,
            Planet::EARTH,
            SolarForcingConfig {
                orbital_phase: 0.31,
                annual_sample_count: 96,
            },
        )
        .unwrap();
        let radiative = derive_radiative_equilibrium_temperature(
            &mesh,
            &forcing,
            RadiativeEquilibriumConfig::EARTHLIKE,
        )
        .unwrap();
        assert_eq!(
            thermal.selected_temperature_kelvin,
            radiative.daily_effective_temperature_kelvin
        );
    }

    #[test]
    fn thermal_inertia_lags_and_reduces_the_seasonal_peak() {
        let mesh = mesh(128);
        let cell = mesh
            .cell_centers
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                (left.y - 0.65).abs().total_cmp(&(right.y - 0.65).abs())
            })
            .unwrap()
            .0;
        let elevations = vec![0.6; mesh.cell_count()];
        let zero = SeasonalThermalConfig {
            land_heat_capacity: 0.0,
            ocean_heat_capacity: 0.0,
            sample_count: 48,
            ..SeasonalThermalConfig::EARTHLIKE
        };
        let inertial = SeasonalThermalConfig {
            land_heat_capacity: 2.0e8,
            ocean_heat_capacity: 2.0e8,
            sample_count: 48,
            ..SeasonalThermalConfig::EARTHLIKE
        };
        let zero_values = (0..48)
            .map(|sample| {
                derive(&mesh, &elevations, sample as f64 / 48.0, zero).selected_temperature_kelvin
                    [cell]
            })
            .collect::<Vec<_>>();
        let inertial_values = (0..48)
            .map(|sample| {
                derive(&mesh, &elevations, sample as f64 / 48.0, inertial)
                    .selected_temperature_kelvin[cell]
            })
            .collect::<Vec<_>>();
        let peak = |values: &[f32]| {
            values
                .iter()
                .enumerate()
                .max_by(|(_, left), (_, right)| left.total_cmp(right))
                .unwrap()
                .0
        };
        let zero_peak = peak(&zero_values);
        let inertial_peak = peak(&inertial_values);
        assert!((inertial_peak + 48 - zero_peak) % 48 > 0);
        assert!((inertial_peak + 48 - zero_peak) % 48 < 24);
        assert!(
            inertial_values
                .iter()
                .copied()
                .fold(f32::NEG_INFINITY, f32::max)
                < zero_values
                    .iter()
                    .copied()
                    .fold(f32::NEG_INFINITY, f32::max)
        );
    }

    #[test]
    fn final_elevation_selects_land_and_ocean_inertia() {
        let mesh = mesh(64);
        let mut elevations = vec![0.4; mesh.cell_count()];
        elevations[0] = 0.5;
        let mixed = derive(&mesh, &elevations, 0.25, SeasonalThermalConfig::EARTHLIKE);
        let all_land = derive(
            &mesh,
            &vec![0.5; mesh.cell_count()],
            0.25,
            SeasonalThermalConfig::EARTHLIKE,
        );
        let all_ocean = derive(
            &mesh,
            &vec![0.4; mesh.cell_count()],
            0.25,
            SeasonalThermalConfig::EARTHLIKE,
        );
        assert_eq!(mixed.diagnostics.land_cell_count, 1);
        assert_eq!(mixed.diagnostics.ocean_cell_count, mesh.cell_count() - 1);
        assert_eq!(
            mixed.selected_temperature_kelvin[0],
            all_land.selected_temperature_kelvin[0]
        );
        assert_eq!(
            mixed.selected_temperature_kelvin[1],
            all_ocean.selected_temperature_kelvin[1]
        );
        assert!(
            mixed
                .diagnostics
                .selected_land_area_weighted_mean_kelvin
                .is_some()
        );
        assert!(
            mixed
                .diagnostics
                .selected_ocean_area_weighted_mean_kelvin
                .is_some()
        );
    }

    #[test]
    fn validates_inputs_and_handles_extreme_valid_parameters() {
        let mesh = mesh(32);
        let elevations = vec![0.5; mesh.cell_count()];
        let invalid = |config| {
            derive_seasonal_thermal_response(
                &mesh,
                SeasonalThermalInputs {
                    planet: Planet::EARTH,
                    selected_orbital_phase: 0.0,
                    radiative_equilibrium: RadiativeEquilibriumConfig::EARTHLIKE,
                    final_elevation: &elevations,
                    sea_level: 0.5,
                },
                config,
            )
        };
        assert!(matches!(
            invalid(SeasonalThermalConfig {
                land_heat_capacity: -1.0,
                ..SeasonalThermalConfig::EARTHLIKE
            }),
            Err(SeasonalThermalError::LandHeatCapacity)
        ));
        assert!(matches!(
            invalid(SeasonalThermalConfig {
                land_heat_capacity: 0.5,
                ..SeasonalThermalConfig::EARTHLIKE
            }),
            Err(SeasonalThermalError::LandHeatCapacity)
        ));
        assert!(matches!(
            invalid(SeasonalThermalConfig {
                ocean_heat_capacity: f64::NAN,
                ..SeasonalThermalConfig::EARTHLIKE
            }),
            Err(SeasonalThermalError::OceanHeatCapacity)
        ));
        assert!(matches!(
            invalid(SeasonalThermalConfig {
                orbital_period_days: 0.0,
                ..SeasonalThermalConfig::EARTHLIKE
            }),
            Err(SeasonalThermalError::OrbitalPeriod)
        ));
        assert!(matches!(
            invalid(SeasonalThermalConfig {
                sample_count: 3,
                ..SeasonalThermalConfig::EARTHLIKE
            }),
            Err(SeasonalThermalError::SampleCount)
        ));

        let extreme = derive(
            &mesh,
            &elevations,
            -10_000.25,
            SeasonalThermalConfig {
                land_heat_capacity: *THERMAL_CAPACITY_RANGE.end(),
                ocean_heat_capacity: 0.0,
                orbital_period_days: *ORBITAL_PERIOD_DAYS_RANGE.start(),
                sample_count: *THERMAL_SAMPLE_RANGE.start(),
            },
        );
        assert!(
            extreme
                .selected_temperature_kelvin
                .iter()
                .all(|value| value.is_finite())
        );
        assert!(
            extreme
                .annual_amplitude_kelvin
                .iter()
                .all(|value| *value >= 0.0)
        );

        let opposite_extreme = derive(
            &mesh,
            &elevations,
            0.75,
            SeasonalThermalConfig {
                land_heat_capacity: MINIMUM_POSITIVE_THERMAL_CAPACITY,
                ocean_heat_capacity: *THERMAL_CAPACITY_RANGE.end(),
                orbital_period_days: *ORBITAL_PERIOD_DAYS_RANGE.end(),
                sample_count: *THERMAL_SAMPLE_RANGE.start(),
            },
        );
        assert!(
            opposite_extreme
                .selected_temperature_kelvin
                .iter()
                .all(|value| value.is_finite())
        );
    }
}
