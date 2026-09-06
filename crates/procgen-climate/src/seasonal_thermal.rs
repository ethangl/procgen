use crate::{
    AreaWeightedSummary, RadiativeEquilibriumError, SECONDS_PER_DAY, SolarForcingConfig,
    SolarForcingError, Surface,
    orbit::{OrbitalSampler, daily_mean_at, orbital_state, selected_sample_index},
    radiative_equilibrium::{RadiativeEquilibriumModel, validate_albedo_field},
    validate_range,
};
use procgen_planet::{Planet, PlanetValidationError};
use procgen_sphere_mesh::SphereMesh;
use std::{fmt, ops::RangeInclusive};

pub const THERMAL_CAPACITY_RANGE: RangeInclusive<f64> = 0.0..=1.0e12;
pub const ORBITAL_PERIOD_DAYS_RANGE: RangeInclusive<f64> = 0.01..=1.0e6;
const MINIMUM_POSITIVE_THERMAL_CAPACITY: f64 = 1.0;
const FIXED_POINT_ITERATION_LIMIT: usize = 48;
const FIXED_POINT_TOLERANCE_KELVIN: f64 = 1.0e-7;
const ENERGY_STEP_ITERATION_LIMIT: usize = 48;
const ENERGY_STEP_RELATIVE_TOLERANCE: f64 = 1.0e-12;

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
}

impl SeasonalThermalConfig {
    /// Convenient mixed-surface values, not hidden assumptions of the stage.
    pub const EARTHLIKE: Self = Self {
        land_heat_capacity: 5.0e7,
        ocean_heat_capacity: 4.0e8,
        orbital_period_days: 365.256_363_004,
    };

    fn heat_capacity(self, surface: Surface) -> f64 {
        match surface {
            Surface::Land => self.land_heat_capacity,
            Surface::Ocean => self.ocean_heat_capacity,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SeasonalThermalInputs<'a> {
    pub planet: Planet,
    pub solar_forcing: SolarForcingConfig,
    pub emissivity: f64,
    pub final_elevation: &'a [f32],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceThermalDiagnostics {
    pub cell_count: usize,
    pub selected_area_weighted_mean_kelvin: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeasonalThermalDiagnostics {
    pub selected_phase: AreaWeightedSummary,
    pub annual_mean: AreaWeightedSummary,
    pub annual_minimum: AreaWeightedSummary,
    pub annual_maximum: AreaWeightedSummary,
    pub annual_amplitude: AreaWeightedSummary,
    pub land: SurfaceThermalDiagnostics,
    pub ocean: SurfaceThermalDiagnostics,
    /// Largest absolute closure error after advancing the solved initial state
    /// through one complete orbit.
    pub maximum_periodic_closure_error_kelvin: f64,
    /// Largest number of Newton fixed-point refinements used by any cell.
    pub maximum_fixed_point_iterations: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeasonalThermalResponse {
    pub selected_temperature_kelvin: Vec<f32>,
    /// Cell-major representative temperatures at each uniform interval midpoint.
    /// Integrated cycles linearly interpolate their boundary states.
    pub annual_temperature_samples_kelvin: Vec<f32>,
    pub annual_sample_count: usize,
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
    SolarForcing(SolarForcingError),
    ElevationCells,
    Elevation,
    HeatCapacity(Surface),
    OrbitalPeriod,
    PeriodicSteadyState,
    NumericalRange,
}

impl fmt::Display for SeasonalThermalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Planet(error) => error.fmt(formatter),
            Self::Radiation(error) => error.fmt(formatter),
            Self::SolarForcing(error) => error.fmt(formatter),
            Self::ElevationCells => formatter.write_str("elevation must match the mesh cells"),
            Self::Elevation => formatter.write_str("elevation must contain only finite values"),
            Self::HeatCapacity(surface) => write!(
                formatter,
                "{surface} heat capacity must be zero or between {} and {} J/m2/K",
                MINIMUM_POSITIVE_THERMAL_CAPACITY,
                THERMAL_CAPACITY_RANGE.end()
            ),
            Self::OrbitalPeriod => write!(
                formatter,
                "orbital period must be finite and between {} and {} days",
                ORBITAL_PERIOD_DAYS_RANGE.start(),
                ORBITAL_PERIOD_DAYS_RANGE.end()
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
            Self::SolarForcing(error) => Some(error),
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

impl From<SolarForcingError> for SeasonalThermalError {
    fn from(error: SolarForcingError) -> Self {
        Self::SolarForcing(error)
    }
}

#[derive(Clone, Copy)]
struct CellCycle {
    selected: f64,
    mean: f64,
    minimum: f64,
    maximum: f64,
    closure_error: f64,
    fixed_point_iterations: usize,
}

#[derive(Clone, Copy)]
struct PeriodicCycle {
    selected: f64,
    closure_error: f64,
    fixed_point_iterations: usize,
}

impl CellCycle {
    fn from_orbit(
        orbit: &[f64],
        selected: f64,
        closure_error: f64,
        fixed_point_iterations: usize,
    ) -> Self {
        Self {
            selected,
            mean: orbit.iter().sum::<f64>() / orbit.len() as f64,
            minimum: orbit.iter().copied().fold(f64::INFINITY, f64::min),
            maximum: orbit.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            closure_error,
            fixed_point_iterations,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct SurfaceAggregate {
    cell_count: usize,
    selected_weighted_sum: f64,
    area: f64,
}

impl SurfaceAggregate {
    fn add(&mut self, selected: f32, area: f32) {
        self.cell_count += 1;
        self.selected_weighted_sum += f64::from(selected) * f64::from(area);
        self.area += f64::from(area);
    }

    fn diagnostics(self) -> SurfaceThermalDiagnostics {
        SurfaceThermalDiagnostics {
            cell_count: self.cell_count,
            selected_area_weighted_mean_kelvin: if self.area > 0.0 {
                Some(self.selected_weighted_sum / self.area)
            } else {
                None
            },
        }
    }
}

struct ResponseAccumulator {
    selected: Vec<f32>,
    annual_samples: Vec<f32>,
    annual_sample_count: usize,
    means: Vec<f32>,
    minima: Vec<f32>,
    maxima: Vec<f32>,
    amplitudes: Vec<f32>,
    land: SurfaceAggregate,
    ocean: SurfaceAggregate,
    maximum_closure_error: f64,
    maximum_fixed_point_iterations: usize,
}

impl ResponseAccumulator {
    fn new(cell_count: usize, annual_sample_count: usize) -> Self {
        Self {
            selected: Vec::with_capacity(cell_count),
            annual_samples: Vec::with_capacity(cell_count * annual_sample_count),
            annual_sample_count,
            means: Vec::with_capacity(cell_count),
            minima: Vec::with_capacity(cell_count),
            maxima: Vec::with_capacity(cell_count),
            amplitudes: Vec::with_capacity(cell_count),
            land: Default::default(),
            ocean: Default::default(),
            maximum_closure_error: 0.0,
            maximum_fixed_point_iterations: 0,
        }
    }

    fn push(
        &mut self,
        cycle: CellCycle,
        annual_samples: &[f64],
        surface: Surface,
        area: f32,
    ) -> Result<(), SeasonalThermalError> {
        let amplitude = cycle.maximum - cycle.minimum;
        let values = [
            cycle.selected,
            cycle.mean,
            cycle.minimum,
            cycle.maximum,
            amplitude,
        ];
        if values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0 || *value > f64::from(f32::MAX))
        {
            return Err(SeasonalThermalError::NumericalRange);
        }

        let selected = cycle.selected as f32;
        self.selected.push(selected);
        self.annual_samples
            .extend(annual_samples.iter().map(|&value| value as f32));
        self.means.push(cycle.mean as f32);
        self.minima.push(cycle.minimum as f32);
        self.maxima.push(cycle.maximum as f32);
        self.amplitudes.push(amplitude as f32);
        match surface {
            Surface::Land => self.land.add(selected, area),
            Surface::Ocean => self.ocean.add(selected, area),
        }
        self.maximum_closure_error = self.maximum_closure_error.max(cycle.closure_error);
        self.maximum_fixed_point_iterations = self
            .maximum_fixed_point_iterations
            .max(cycle.fixed_point_iterations);
        Ok(())
    }

    fn finish(self, mesh: &SphereMesh) -> SeasonalThermalResponse {
        let diagnostics = SeasonalThermalDiagnostics {
            selected_phase: AreaWeightedSummary::from_field(mesh, &self.selected),
            annual_mean: AreaWeightedSummary::from_field(mesh, &self.means),
            annual_minimum: AreaWeightedSummary::from_field(mesh, &self.minima),
            annual_maximum: AreaWeightedSummary::from_field(mesh, &self.maxima),
            annual_amplitude: AreaWeightedSummary::from_field(mesh, &self.amplitudes),
            land: self.land.diagnostics(),
            ocean: self.ocean.diagnostics(),
            maximum_periodic_closure_error_kelvin: self.maximum_closure_error,
            maximum_fixed_point_iterations: self.maximum_fixed_point_iterations,
        };
        SeasonalThermalResponse {
            selected_temperature_kelvin: self.selected,
            annual_temperature_samples_kelvin: self.annual_samples,
            annual_sample_count: self.annual_sample_count,
            annual_mean_temperature_kelvin: self.means,
            annual_minimum_temperature_kelvin: self.minima,
            annual_maximum_temperature_kelvin: self.maxima,
            annual_amplitude_kelvin: self.amplitudes,
            diagnostics,
        }
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
    cell_albedo: &[f32],
) -> Result<SeasonalThermalResponse, SeasonalThermalError> {
    let SeasonalThermalInputs {
        planet,
        solar_forcing,
        emissivity,
        final_elevation,
    } = inputs;
    planet.validate()?;
    solar_forcing.validate()?;
    validate_albedo_field(mesh, cell_albedo)?;
    let radiation = RadiativeEquilibriumModel::new(emissivity)?;
    validate_inputs(mesh, final_elevation, config)?;

    let sample_count = solar_forcing.annual_sample_count;
    let step_seconds = config.orbital_period_days * SECONDS_PER_DAY / sample_count as f64;
    let sampler = OrbitalSampler::new(planet, sample_count);
    let selected_phase = solar_forcing.orbital_phase.rem_euclid(1.0);
    let selected_state = orbital_state(planet, selected_phase);
    let mut output = ResponseAccumulator::new(mesh.cell_count(), sample_count);
    let mut targets = Vec::with_capacity(sample_count);
    let mut endpoints = Vec::with_capacity(sample_count + 1);
    let mut samples = Vec::with_capacity(sample_count);

    for (((&elevation, &center), &area), &albedo) in final_elevation
        .iter()
        .zip(&mesh.cell_centers)
        .zip(&mesh.cell_areas)
        .zip(cell_albedo)
    {
        let surface = Surface::from_elevation(elevation);
        let heat_capacity = config.heat_capacity(surface);
        let latitude_sine = f64::from(center.y / mesh.radius);
        let target = |state| {
            radiation.temperature_kelvin(
                daily_mean_at(latitude_sine, state).watts_per_square_meter,
                f64::from(albedo),
            )
        };
        targets.clear();
        targets.extend(sampler.midpoint_states().iter().copied().map(target));
        let selected_target = target(selected_state);
        // Zero capacity is the documented exact radiative response; positive
        // capacity samples the integrated periodic cycle.
        let (cycle, annual_samples) = if heat_capacity == 0.0 {
            (
                CellCycle::from_orbit(&targets, selected_target, 0.0, 0),
                targets.as_slice(),
            )
        } else {
            let coefficient = step_seconds * radiation.emission_coefficient() / heat_capacity;
            let solution =
                solve_periodic_cycle(&targets, selected_phase, coefficient, &mut endpoints)?;
            samples.clear();
            samples.extend(
                endpoints
                    .windows(2)
                    .map(|interval| (interval[0] + interval[1]) * 0.5),
            );
            let cycle = CellCycle::from_orbit(
                &samples,
                solution.selected,
                solution.closure_error,
                solution.fixed_point_iterations,
            );
            (cycle, samples.as_slice())
        };
        output.push(cycle, annual_samples, surface, area)?;
    }
    Ok(output.finish(mesh))
}

fn validate_inputs(
    mesh: &SphereMesh,
    elevations: &[f32],
    config: SeasonalThermalConfig,
) -> Result<(), SeasonalThermalError> {
    if elevations.len() != mesh.cell_count() {
        return Err(SeasonalThermalError::ElevationCells);
    }
    if elevations.iter().any(|value| !value.is_finite()) {
        return Err(SeasonalThermalError::Elevation);
    }
    if !valid_heat_capacity(config.land_heat_capacity) {
        return Err(SeasonalThermalError::HeatCapacity(Surface::Land));
    }
    if !valid_heat_capacity(config.ocean_heat_capacity) {
        return Err(SeasonalThermalError::HeatCapacity(Surface::Ocean));
    }
    validate_range(
        config.orbital_period_days,
        &ORBITAL_PERIOD_DAYS_RANGE,
        SeasonalThermalError::OrbitalPeriod,
    )?;
    Ok(())
}

fn valid_heat_capacity(value: f64) -> bool {
    value == 0.0
        || (MINIMUM_POSITIVE_THERMAL_CAPACITY..=*THERMAL_CAPACITY_RANGE.end()).contains(&value)
}

fn solve_periodic_cycle(
    targets: &[f64],
    selected_phase: f64,
    coefficient: f64,
    endpoints: &mut Vec<f64>,
) -> Result<PeriodicCycle, SeasonalThermalError> {
    let minimum = targets.iter().copied().fold(f64::INFINITY, f64::min);
    let maximum = targets.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if minimum == maximum {
        endpoints.clear();
        endpoints.resize(targets.len() + 1, minimum);
        return Ok(PeriodicCycle {
            selected: minimum,
            closure_error: 0.0,
            fixed_point_iterations: 0,
        });
    }
    let mut initial = targets.iter().sum::<f64>() / targets.len() as f64;
    for iteration in 1..=FIXED_POINT_ITERATION_LIMIT {
        let derivative = integrate_cycle(initial, targets, coefficient, endpoints);
        let end = *endpoints.last().expect("an orbit always has endpoints");
        let residual = end - initial;
        if residual.abs() <= FIXED_POINT_TOLERANCE_KELVIN {
            return Ok(PeriodicCycle {
                selected: sample_cycle(endpoints, selected_phase),
                closure_error: residual.abs(),
                fixed_point_iterations: iteration,
            });
        }
        let slope = derivative - 1.0;
        if !slope.is_finite() || slope.abs() < f64::EPSILON {
            break;
        }
        initial = (initial - residual / slope).clamp(minimum, maximum);
    }
    Err(SeasonalThermalError::PeriodicSteadyState)
}

fn sample_cycle(endpoints: &[f64], phase: f64) -> f64 {
    let sample_count = endpoints.len() - 1;
    let position = phase.rem_euclid(1.0) * sample_count as f64;
    let lower = selected_sample_index(phase, sample_count);
    let fraction = position - position.floor();
    endpoints[lower] + (endpoints[lower + 1] - endpoints[lower]) * fraction
}

fn integrate_cycle(
    initial: f64,
    targets: &[f64],
    coefficient: f64,
    endpoints: &mut Vec<f64>,
) -> f64 {
    let mut temperature = initial;
    let mut derivative = 1.0;
    endpoints.clear();
    endpoints.push(temperature);
    for &equilibrium in targets {
        temperature = implicit_energy_step(temperature, equilibrium, coefficient);
        endpoints.push(temperature);
        derivative /= 1.0 + 4.0 * coefficient * temperature.powi(3);
    }
    derivative
}

fn implicit_energy_step(temperature: f64, equilibrium: f64, coefficient: f64) -> f64 {
    let rhs = temperature + coefficient * equilibrium.powi(4);
    let mut lower = temperature.min(equilibrium);
    let mut upper = temperature.max(equilibrium);
    let mut result = temperature;
    for _ in 0..ENERGY_STEP_ITERATION_LIMIT {
        let residual = result + coefficient * result.powi(4) - rhs;
        if residual.abs() <= ENERGY_STEP_RELATIVE_TOLERANCE * rhs.abs().max(1.0) {
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
        derive_with_samples(mesh, elevations, phase, 96, config)
    }

    fn derive_with_samples(
        mesh: &SphereMesh,
        elevations: &[f32],
        phase: f64,
        sample_count: usize,
        config: SeasonalThermalConfig,
    ) -> SeasonalThermalResponse {
        let albedo = vec![0.3; mesh.cell_count()];
        derive_seasonal_thermal_response(
            mesh,
            SeasonalThermalInputs {
                planet: Planet::EARTH,
                solar_forcing: SolarForcingConfig {
                    orbital_phase: phase,
                    annual_sample_count: sample_count,
                },
                emissivity: 1.0,
                final_elevation: elevations,
            },
            config,
            &albedo,
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
    fn cycle_sampling_guards_a_phase_rounded_to_one() {
        let wrapped = (-f64::MIN_POSITIVE).rem_euclid(1.0);
        assert_eq!(wrapped, 1.0);
        assert_eq!(sample_cycle(&[10.0, 20.0, 10.0], wrapped), 10.0);
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
            crate::RadiativeEquilibriumConfig::EARTHLIKE,
            &vec![0.3; mesh.cell_count()],
        )
        .unwrap();
        for (&thermal, &radiative) in thermal
            .selected_temperature_kelvin
            .iter()
            .zip(&radiative.daily_effective_temperature_kelvin)
        {
            assert!((thermal - radiative).abs() <= 1.0e-4);
        }
        let sampler = OrbitalSampler::new(Planet::EARTH, 96);
        let radiation = RadiativeEquilibriumModel::new(1.0).unwrap();
        let latitude_sine = f64::from(mesh.cell_centers[0].y / mesh.radius);
        let expected_midpoint = radiation.temperature_kelvin(
            daily_mean_at(latitude_sine, sampler.midpoint_states()[0]).watts_per_square_meter,
            0.3,
        );
        assert!(
            (f64::from(thermal.annual_temperature_samples_kelvin[0]) - expected_midpoint).abs()
                <= 1.0e-4
        );
    }

    #[test]
    fn inertial_annual_samples_interpolate_interval_midpoints() {
        let targets = [250.0, 280.0, 300.0, 265.0];
        let mut endpoints = Vec::new();
        let solution = solve_periodic_cycle(&targets, 0.3, 1.0e-8, &mut endpoints).unwrap();
        let samples = endpoints
            .windows(2)
            .map(|interval| (interval[0] + interval[1]) * 0.5)
            .collect::<Vec<_>>();
        let cycle = CellCycle::from_orbit(
            &samples,
            solution.selected,
            solution.closure_error,
            solution.fixed_point_iterations,
        );

        assert_eq!(samples.len(), targets.len());
        for (sample, interval) in samples.iter().zip(endpoints.windows(2)) {
            assert!((sample - (interval[0] + interval[1]) * 0.5).abs() <= f64::EPSILON);
        }
        assert!(
            (cycle.mean - samples.iter().sum::<f64>() / samples.len() as f64).abs() <= f64::EPSILON
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
            ..SeasonalThermalConfig::EARTHLIKE
        };
        let inertial = SeasonalThermalConfig {
            land_heat_capacity: 2.0e8,
            ocean_heat_capacity: 2.0e8,
            ..SeasonalThermalConfig::EARTHLIKE
        };
        let zero_values = (0..48)
            .map(|sample| {
                derive_with_samples(&mesh, &elevations, sample as f64 / 48.0, 48, zero)
                    .selected_temperature_kelvin[cell]
            })
            .collect::<Vec<_>>();
        let inertial_values = (0..48)
            .map(|sample| {
                derive_with_samples(&mesh, &elevations, sample as f64 / 48.0, 48, inertial)
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
        elevations[0] = 0.500_1;
        let mixed = derive(&mesh, &elevations, 0.25, SeasonalThermalConfig::EARTHLIKE);
        let all_land = derive(
            &mesh,
            &vec![0.500_1; mesh.cell_count()],
            0.25,
            SeasonalThermalConfig::EARTHLIKE,
        );
        let all_ocean = derive(
            &mesh,
            &vec![0.5; mesh.cell_count()],
            0.25,
            SeasonalThermalConfig::EARTHLIKE,
        );
        assert_eq!(all_ocean.diagnostics.land.cell_count, 0);
        assert_eq!(mixed.diagnostics.land.cell_count, 1);
        assert_eq!(mixed.diagnostics.ocean.cell_count, mesh.cell_count() - 1);
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
                .land
                .selected_area_weighted_mean_kelvin
                .is_some()
        );
        assert!(
            mixed
                .diagnostics
                .ocean
                .selected_area_weighted_mean_kelvin
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
                    solar_forcing: SolarForcingConfig::default(),
                    emissivity: 1.0,
                    final_elevation: &elevations,
                },
                config,
                &vec![0.3; mesh.cell_count()],
            )
        };
        assert!(matches!(
            invalid(SeasonalThermalConfig {
                land_heat_capacity: -1.0,
                ..SeasonalThermalConfig::EARTHLIKE
            }),
            Err(SeasonalThermalError::HeatCapacity(Surface::Land))
        ));
        assert!(matches!(
            invalid(SeasonalThermalConfig {
                land_heat_capacity: 0.5,
                ..SeasonalThermalConfig::EARTHLIKE
            }),
            Err(SeasonalThermalError::HeatCapacity(Surface::Land))
        ));
        assert!(matches!(
            invalid(SeasonalThermalConfig {
                ocean_heat_capacity: f64::NAN,
                ..SeasonalThermalConfig::EARTHLIKE
            }),
            Err(SeasonalThermalError::HeatCapacity(Surface::Ocean))
        ));
        assert!(matches!(
            invalid(SeasonalThermalConfig {
                orbital_period_days: 0.0,
                ..SeasonalThermalConfig::EARTHLIKE
            }),
            Err(SeasonalThermalError::OrbitalPeriod)
        ));
        assert!(matches!(
            derive_seasonal_thermal_response(
                &mesh,
                SeasonalThermalInputs {
                    planet: Planet::EARTH,
                    solar_forcing: SolarForcingConfig {
                        orbital_phase: 0.0,
                        annual_sample_count: 3,
                    },
                    emissivity: 1.0,
                    final_elevation: &elevations,
                },
                SeasonalThermalConfig::EARTHLIKE,
                &vec![0.3; mesh.cell_count()],
            ),
            Err(SeasonalThermalError::SolarForcing(
                SolarForcingError::AnnualSampleCount
            ))
        ));

        let extreme = derive_with_samples(
            &mesh,
            &elevations,
            -10_000.25,
            *crate::ANNUAL_SAMPLE_RANGE.start(),
            SeasonalThermalConfig {
                land_heat_capacity: *THERMAL_CAPACITY_RANGE.end(),
                ocean_heat_capacity: 0.0,
                orbital_period_days: *ORBITAL_PERIOD_DAYS_RANGE.start(),
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

        let opposite_extreme = derive_with_samples(
            &mesh,
            &elevations,
            0.75,
            *crate::ANNUAL_SAMPLE_RANGE.start(),
            SeasonalThermalConfig {
                land_heat_capacity: MINIMUM_POSITIVE_THERMAL_CAPACITY,
                ocean_heat_capacity: *THERMAL_CAPACITY_RANGE.end(),
                orbital_period_days: *ORBITAL_PERIOD_DAYS_RANGE.end(),
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
