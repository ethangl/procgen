use crate::{
    ANNUAL_SAMPLE_RANGE, AreaWeightedSummary, ORBITAL_PERIOD_DAYS_RANGE, Surface,
    orbit::selected_sample_index, validate_range,
};
use procgen_sphere_mesh::SphereMesh;
use procgen_tectonics::is_land;
use std::{fmt, ops::RangeInclusive};

pub const CRYOSPHERE_ITERATION_LIMIT_RANGE: RangeInclusive<usize> = 1..=4_096;
pub const CRYOSPHERE_TEMPERATURE_RANGE: RangeInclusive<f64> = 0.0..=10_000.0;
pub const CRYOSPHERE_MASS_RANGE: RangeInclusive<f64> = 0.0..=1.0e9;
pub const CRYOSPHERE_RATE_RANGE: RangeInclusive<f64> = 0.0..=10_000.0;
pub const CRYOSPHERE_FRACTION_RATE_RANGE: RangeInclusive<f64> = 0.0..=1.0;
pub const CRYOSPHERE_CLOSURE_TOLERANCE_RANGE: RangeInclusive<f64> = 0.0..=1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CryosphereConfig {
    pub maximum_iterations: usize,
    pub closure_tolerance: f64,
    pub snowfall_temperature_kelvin: f64,
    pub melt_temperature_kelvin: f64,
    /// Snow water equivalent producing visually complete seasonal cover.
    pub full_snow_cover_kg_per_m2: f64,
    /// Bounded seasonal reservoir; overflow supplies perennial land ice.
    pub seasonal_snow_capacity_kg_per_m2: f64,
    pub snow_melt_kg_per_m2_per_kelvin_day: f64,
    pub land_ice_melt_kg_per_m2_per_kelvin_day: f64,
    pub sea_ice_growth_fraction_per_kelvin_day: f64,
    pub sea_ice_melt_fraction_per_kelvin_day: f64,
}

impl CryosphereConfig {
    /// Convenient Earth-like choices; the solver itself has no fixed latitude rules.
    pub const EARTHLIKE: Self = Self {
        maximum_iterations: 512,
        closure_tolerance: 1.0e-6,
        snowfall_temperature_kelvin: 274.15,
        melt_temperature_kelvin: 273.15,
        full_snow_cover_kg_per_m2: 25.0,
        seasonal_snow_capacity_kg_per_m2: 200.0,
        snow_melt_kg_per_m2_per_kelvin_day: 3.0,
        land_ice_melt_kg_per_m2_per_kelvin_day: 1.0,
        sea_ice_growth_fraction_per_kelvin_day: 0.005,
        sea_ice_melt_fraction_per_kelvin_day: 0.01,
    };
}

#[derive(Clone, Copy, Debug)]
pub struct CryosphereInputs<'a> {
    /// Cell-major temperatures for uniform intervals over one annual cycle.
    pub annual_temperature_samples_kelvin: &'a [f32],
    pub annual_sample_count: usize,
    pub selected_orbital_phase: f64,
    pub orbital_period_days: f64,
    /// Existing precipitation climatology, held constant within the annual cycle.
    pub precipitation_kg_per_m2_per_day: &'a [f32],
    /// The existing sea-level predicate applied to this field is the ocean mask.
    pub final_elevation: &'a [f32],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CryosphereDiagnostics {
    pub selected_snowfall_kg_per_m2_per_day: AreaWeightedSummary,
    pub selected_melt_kg_per_m2_per_day: AreaWeightedSummary,
    pub selected_snow_cover_fraction: AreaWeightedSummary,
    pub land_ice_cover_fraction: AreaWeightedSummary,
    pub selected_sea_ice_cover_fraction: AreaWeightedSummary,
    pub annual_snowfall_kg_per_m2: AreaWeightedSummary,
    pub annual_snow_melt_kg_per_m2: AreaWeightedSummary,
    pub annual_land_ice_accumulation_kg_per_m2: AreaWeightedSummary,
    pub annual_land_ice_ablation_kg_per_m2: AreaWeightedSummary,
    pub annual_sea_ice_growth_fraction: AreaWeightedSummary,
    pub annual_sea_ice_melt_fraction: AreaWeightedSummary,
    pub land_cell_count: usize,
    pub ocean_cell_count: usize,
    pub snow_covered_cell_count: usize,
    pub land_ice_cell_count: usize,
    pub sea_ice_cell_count: usize,
    pub maximum_iterations_used: usize,
    pub maximum_snow_closure_error_kg_per_m2: f64,
    pub maximum_sea_ice_closure_error: f64,
    pub snow_mass_balance_error_kg_per_m2: f64,
    pub land_ice_mass_balance_kg_per_m2: f64,
    pub sea_ice_cover_balance_error: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cryosphere {
    pub cell_snowfall_kg_per_m2_per_day: Vec<f32>,
    pub cell_melt_kg_per_m2_per_day: Vec<f32>,
    pub cell_snow_cover_fraction: Vec<f32>,
    pub cell_land_ice_cover_fraction: Vec<f32>,
    pub cell_sea_ice_cover_fraction: Vec<f32>,
    pub diagnostics: CryosphereDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CryosphereError {
    TemperatureCells,
    Temperature,
    SampleCount,
    SelectedPhase,
    OrbitalPeriod,
    PrecipitationCells,
    Precipitation,
    ElevationCells,
    Elevation,
    IterationLimit,
    ClosureTolerance,
    TemperatureThreshold,
    SnowCapacity,
    MeltRate,
    SeaIceRate,
    PeriodicSteadyState,
    NumericalRange,
}

impl fmt::Display for CryosphereError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use CryosphereError as Error;
        match self {
            Error::TemperatureCells => {
                formatter.write_str("annual temperatures must match cells and samples")
            }
            Error::Temperature => {
                formatter.write_str("temperatures must be finite and nonnegative")
            }
            Error::SampleCount => formatter.write_str("annual sample count must be positive"),
            Error::SelectedPhase => formatter.write_str("selected orbital phase must be finite"),
            Error::OrbitalPeriod => {
                formatter.write_str("orbital period must be finite and positive")
            }
            Error::PrecipitationCells => formatter.write_str("precipitation must match mesh cells"),
            Error::Precipitation => {
                formatter.write_str("precipitation must be finite and nonnegative")
            }
            Error::ElevationCells => formatter.write_str("elevation must match mesh cells"),
            Error::Elevation => formatter.write_str("elevation must contain only finite values"),
            Error::IterationLimit => {
                formatter.write_str("cryosphere iteration limit is outside its supported range")
            }
            Error::ClosureTolerance => {
                formatter.write_str("cryosphere closure tolerance is invalid")
            }
            Error::TemperatureThreshold => {
                formatter.write_str("cryosphere temperature threshold is invalid")
            }
            Error::SnowCapacity => formatter.write_str("cryosphere snow capacity is invalid"),
            Error::MeltRate => formatter.write_str("cryosphere melt rate is invalid"),
            Error::SeaIceRate => formatter.write_str("cryosphere sea-ice rate is invalid"),
            Error::PeriodicSteadyState => {
                formatter.write_str("cryosphere did not reach a periodic annual cycle")
            }
            Error::NumericalRange => {
                formatter.write_str("cryosphere is outside the finite f32 output range")
            }
        }
    }
}

impl std::error::Error for CryosphereError {}

#[derive(Clone, Copy, Default)]
struct LandSnapshot {
    snowfall_rate: f64,
    snow_melt_rate: f64,
    land_ice_ablation_potential_rate: f64,
    snow: f64,
}

#[derive(Clone, Copy, Default)]
struct LandCycle {
    final_snow: f64,
    snowfall: f64,
    snow_melt: f64,
    land_ice_accumulation: f64,
    land_ice_ablation_potential: f64,
    selected: LandSnapshot,
}

#[derive(Clone, Copy, Default)]
struct OceanSnapshot {
    snowfall_rate: f64,
    sea_ice: f64,
}

#[derive(Clone, Copy, Default)]
struct OceanCycle {
    final_sea_ice: f64,
    snowfall: f64,
    sea_ice_growth: f64,
    sea_ice_melt: f64,
    selected: OceanSnapshot,
}

#[derive(Clone, Copy)]
struct CellResult {
    surface: Surface,
    snowfall_rate: f64,
    melt_rate: f64,
    snow_cover: f64,
    land_ice_cover: f64,
    sea_ice_cover: f64,
    snowfall: f64,
    snow_melt: f64,
    land_ice_accumulation: f64,
    land_ice_ablation: f64,
    sea_ice_growth: f64,
    sea_ice_melt: f64,
    iterations_used: usize,
    snow_closure: f64,
    sea_ice_closure: f64,
    snow_balance_error: f64,
    sea_ice_balance_error: f64,
}

impl CellResult {
    fn validate(self) -> Result<Self, CryosphereError> {
        if [
            self.snowfall_rate,
            self.melt_rate,
            self.snow_cover,
            self.land_ice_cover,
            self.sea_ice_cover,
            self.snowfall,
            self.snow_melt,
            self.land_ice_accumulation,
            self.land_ice_ablation,
            self.sea_ice_growth,
            self.sea_ice_melt,
            self.snow_closure,
            self.sea_ice_closure,
        ]
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0 || *value > f64::from(f32::MAX))
            || !self.snow_balance_error.is_finite()
            || !self.sea_ice_balance_error.is_finite()
        {
            return Err(CryosphereError::NumericalRange);
        }
        Ok(self)
    }
}

pub fn derive_cryosphere(
    mesh: &SphereMesh,
    inputs: CryosphereInputs<'_>,
    config: CryosphereConfig,
) -> Result<Cryosphere, CryosphereError> {
    validate(mesh, inputs, config)?;
    let selected_sample =
        selected_sample_index(inputs.selected_orbital_phase, inputs.annual_sample_count);
    let step_days = inputs.orbital_period_days / inputs.annual_sample_count as f64;
    let mut results = Vec::with_capacity(mesh.cell_count());
    for cell in 0..mesh.cell_count() {
        let temperatures = &inputs.annual_temperature_samples_kelvin
            [cell * inputs.annual_sample_count..(cell + 1) * inputs.annual_sample_count];
        let precipitation = f64::from(inputs.precipitation_kg_per_m2_per_day[cell]);
        let surface = if is_land(inputs.final_elevation[cell]) {
            Surface::Land
        } else {
            Surface::Ocean
        };
        results.push(match surface {
            Surface::Land => solve_land_cell(
                temperatures,
                precipitation,
                selected_sample,
                step_days,
                config,
            )?,
            Surface::Ocean => solve_ocean_cell(
                temperatures,
                precipitation,
                selected_sample,
                step_days,
                config,
            )?,
        });
    }
    Ok(finish(mesh, &results))
}

fn solve_land_cell(
    temperatures: &[f32],
    precipitation_rate: f64,
    selected_sample: usize,
    step_days: f64,
    config: CryosphereConfig,
) -> Result<CellResult, CryosphereError> {
    let (initial_snow, iterations_used) =
        solve_periodic_reservoir(config.seasonal_snow_capacity_kg_per_m2, config, |initial| {
            land_cycle(
                initial,
                temperatures,
                precipitation_rate,
                selected_sample,
                step_days,
                config,
            )
            .final_snow
        })?;
    let cycle = land_cycle(
        initial_snow,
        temperatures,
        precipitation_rate,
        selected_sample,
        step_days,
        config,
    );
    let land_ice_cover = if cycle.land_ice_accumulation == 0.0 {
        0.0
    } else if cycle.land_ice_ablation_potential == 0.0 {
        1.0
    } else {
        (cycle.land_ice_accumulation / cycle.land_ice_ablation_potential).min(1.0)
    };
    let land_ice_ablation = cycle.land_ice_ablation_potential * land_ice_cover;
    CellResult {
        surface: Surface::Land,
        snowfall_rate: cycle.selected.snowfall_rate,
        melt_rate: cycle.selected.snow_melt_rate
            + cycle.selected.land_ice_ablation_potential_rate * land_ice_cover,
        snow_cover: (cycle.selected.snow / config.full_snow_cover_kg_per_m2).min(1.0),
        land_ice_cover,
        sea_ice_cover: 0.0,
        snowfall: cycle.snowfall,
        snow_melt: cycle.snow_melt,
        land_ice_accumulation: cycle.land_ice_accumulation,
        land_ice_ablation,
        sea_ice_growth: 0.0,
        sea_ice_melt: 0.0,
        iterations_used,
        snow_closure: (cycle.final_snow - initial_snow).abs(),
        sea_ice_closure: 0.0,
        snow_balance_error: cycle.snowfall
            - cycle.snow_melt
            - cycle.land_ice_accumulation
            - (cycle.final_snow - initial_snow),
        sea_ice_balance_error: 0.0,
    }
    .validate()
}

fn solve_ocean_cell(
    temperatures: &[f32],
    precipitation_rate: f64,
    selected_sample: usize,
    step_days: f64,
    config: CryosphereConfig,
) -> Result<CellResult, CryosphereError> {
    let (initial_sea_ice, iterations_used) = solve_periodic_reservoir(1.0, config, |initial| {
        ocean_cycle(
            initial,
            temperatures,
            precipitation_rate,
            selected_sample,
            step_days,
            config,
        )
        .final_sea_ice
    })?;
    let cycle = ocean_cycle(
        initial_sea_ice,
        temperatures,
        precipitation_rate,
        selected_sample,
        step_days,
        config,
    );
    CellResult {
        surface: Surface::Ocean,
        snowfall_rate: cycle.selected.snowfall_rate,
        melt_rate: 0.0,
        snow_cover: 0.0,
        land_ice_cover: 0.0,
        sea_ice_cover: cycle.selected.sea_ice,
        snowfall: cycle.snowfall,
        snow_melt: 0.0,
        land_ice_accumulation: 0.0,
        land_ice_ablation: 0.0,
        sea_ice_growth: cycle.sea_ice_growth,
        sea_ice_melt: cycle.sea_ice_melt,
        iterations_used,
        snow_closure: 0.0,
        sea_ice_closure: (cycle.final_sea_ice - initial_sea_ice).abs(),
        snow_balance_error: 0.0,
        sea_ice_balance_error: cycle.sea_ice_growth
            - cycle.sea_ice_melt
            - (cycle.final_sea_ice - initial_sea_ice),
    }
    .validate()
}

/// Bisects an initial reservoir in `[0, capacity]` until one complete annual
/// cycle returns to that initial value.
fn solve_periodic_reservoir(
    capacity: f64,
    config: CryosphereConfig,
    cycle: impl Fn(f64) -> f64,
) -> Result<(f64, usize), CryosphereError> {
    let residual = |initial| cycle(initial) - initial;
    let mut lower = 0.0;
    let mut upper = capacity;
    let lower_residual = residual(lower);
    if lower_residual.abs() <= config.closure_tolerance {
        return Ok((lower, 1));
    }
    let upper_residual = residual(upper);
    if upper_residual.abs() <= config.closure_tolerance {
        return Ok((upper, 1));
    }
    debug_assert!(lower_residual >= 0.0, "an empty reservoir cannot lose mass");
    debug_assert!(
        upper_residual <= 0.0,
        "a full bounded reservoir cannot gain mass"
    );

    for iteration in 1..=config.maximum_iterations {
        let midpoint = (lower + upper) * 0.5;
        let midpoint_residual = residual(midpoint);
        if midpoint_residual.abs() <= config.closure_tolerance {
            return Ok((midpoint, iteration));
        }
        if midpoint_residual > 0.0 {
            lower = midpoint;
        } else {
            upper = midpoint;
        }
    }
    Err(CryosphereError::PeriodicSteadyState)
}

fn land_cycle(
    initial_snow: f64,
    temperatures: &[f32],
    precipitation_rate: f64,
    selected_sample: usize,
    step_days: f64,
    config: CryosphereConfig,
) -> LandCycle {
    let mut snow = initial_snow;
    let mut cycle = LandCycle::default();
    for (sample, &temperature) in temperatures.iter().enumerate() {
        let temperature = f64::from(temperature);
        let snowfall_rate = snowfall_rate(temperature, precipitation_rate, config);
        let snowfall = snowfall_rate * step_days;
        cycle.snowfall += snowfall;
        snow += snowfall;

        let warm_degree_days = (temperature - config.melt_temperature_kelvin).max(0.0) * step_days;
        let snow_melt = snow.min(config.snow_melt_kg_per_m2_per_kelvin_day * warm_degree_days);
        snow -= snow_melt;
        cycle.snow_melt += snow_melt;
        let snow_degree_days = if config.snow_melt_kg_per_m2_per_kelvin_day > 0.0 {
            snow_melt / config.snow_melt_kg_per_m2_per_kelvin_day
        } else {
            0.0
        };
        let land_ice_ablation_potential = (warm_degree_days - snow_degree_days).max(0.0)
            * config.land_ice_melt_kg_per_m2_per_kelvin_day;
        cycle.land_ice_ablation_potential += land_ice_ablation_potential;

        let accumulation = (snow - config.seasonal_snow_capacity_kg_per_m2).max(0.0);
        snow -= accumulation;
        cycle.land_ice_accumulation += accumulation;
        if sample == selected_sample {
            cycle.selected = LandSnapshot {
                snowfall_rate,
                snow_melt_rate: snow_melt / step_days,
                land_ice_ablation_potential_rate: land_ice_ablation_potential / step_days,
                snow,
            };
        }
    }
    cycle.final_snow = snow;
    cycle
}

fn ocean_cycle(
    initial_sea_ice: f64,
    temperatures: &[f32],
    precipitation_rate: f64,
    selected_sample: usize,
    step_days: f64,
    config: CryosphereConfig,
) -> OceanCycle {
    let mut sea_ice = initial_sea_ice;
    let mut cycle = OceanCycle::default();
    for (sample, &temperature) in temperatures.iter().enumerate() {
        let temperature = f64::from(temperature);
        let snowfall_rate = snowfall_rate(temperature, precipitation_rate, config);
        cycle.snowfall += snowfall_rate * step_days;
        let growth = (config.melt_temperature_kelvin - temperature).max(0.0)
            * config.sea_ice_growth_fraction_per_kelvin_day
            * step_days;
        let actual_growth = growth.min(1.0 - sea_ice);
        sea_ice += actual_growth;
        cycle.sea_ice_growth += actual_growth;
        let melt = (temperature - config.melt_temperature_kelvin).max(0.0)
            * config.sea_ice_melt_fraction_per_kelvin_day
            * step_days;
        let actual_melt = melt.min(sea_ice);
        sea_ice -= actual_melt;
        cycle.sea_ice_melt += actual_melt;
        if sample == selected_sample {
            cycle.selected = OceanSnapshot {
                snowfall_rate,
                sea_ice,
            };
        }
    }
    cycle.final_sea_ice = sea_ice;
    cycle
}

fn snowfall_rate(temperature: f64, precipitation_rate: f64, config: CryosphereConfig) -> f64 {
    if temperature <= config.snowfall_temperature_kelvin {
        precipitation_rate
    } else {
        0.0
    }
}

fn finish(mesh: &SphereMesh, results: &[CellResult]) -> Cryosphere {
    let field = |select: fn(&CellResult) -> f64| {
        results
            .iter()
            .map(|value| select(value) as f32)
            .collect::<Vec<_>>()
    };
    let snowfall_rate = field(|value| value.snowfall_rate);
    let melt_rate = field(|value| value.melt_rate);
    let snow_cover = field(|value| value.snow_cover);
    let land_ice_cover = field(|value| value.land_ice_cover);
    let sea_ice_cover = field(|value| value.sea_ice_cover);
    let annual_snowfall = field(|value| value.snowfall);
    let annual_snow_melt = field(|value| value.snow_melt);
    let annual_land_ice_accumulation = field(|value| value.land_ice_accumulation);
    let annual_land_ice_ablation = field(|value| value.land_ice_ablation);
    let annual_land_ice_balance =
        field(|value| value.land_ice_accumulation - value.land_ice_ablation);
    let annual_sea_ice_growth = field(|value| value.sea_ice_growth);
    let annual_sea_ice_melt = field(|value| value.sea_ice_melt);
    let snow_balance_errors = field(|value| value.snow_balance_error);
    let sea_ice_balance_errors = field(|value| value.sea_ice_balance_error);
    let summary = |values: &[f32]| AreaWeightedSummary::from_field(mesh, values);
    let diagnostics = CryosphereDiagnostics {
        selected_snowfall_kg_per_m2_per_day: summary(&snowfall_rate),
        selected_melt_kg_per_m2_per_day: summary(&melt_rate),
        selected_snow_cover_fraction: summary(&snow_cover),
        land_ice_cover_fraction: summary(&land_ice_cover),
        selected_sea_ice_cover_fraction: summary(&sea_ice_cover),
        annual_snowfall_kg_per_m2: summary(&annual_snowfall),
        annual_snow_melt_kg_per_m2: summary(&annual_snow_melt),
        annual_land_ice_accumulation_kg_per_m2: summary(&annual_land_ice_accumulation),
        annual_land_ice_ablation_kg_per_m2: summary(&annual_land_ice_ablation),
        annual_sea_ice_growth_fraction: summary(&annual_sea_ice_growth),
        annual_sea_ice_melt_fraction: summary(&annual_sea_ice_melt),
        land_cell_count: results
            .iter()
            .filter(|value| value.surface == Surface::Land)
            .count(),
        ocean_cell_count: results
            .iter()
            .filter(|value| value.surface == Surface::Ocean)
            .count(),
        snow_covered_cell_count: snow_cover.iter().filter(|&&value| value > 0.0).count(),
        land_ice_cell_count: land_ice_cover.iter().filter(|&&value| value > 0.0).count(),
        sea_ice_cell_count: sea_ice_cover.iter().filter(|&&value| value > 0.0).count(),
        maximum_iterations_used: results
            .iter()
            .map(|value| value.iterations_used)
            .max()
            .unwrap_or(0),
        maximum_snow_closure_error_kg_per_m2: results
            .iter()
            .map(|value| value.snow_closure)
            .fold(0.0, f64::max),
        maximum_sea_ice_closure_error: results
            .iter()
            .map(|value| value.sea_ice_closure)
            .fold(0.0, f64::max),
        snow_mass_balance_error_kg_per_m2: mesh.area_weighted_mean(&snow_balance_errors),
        land_ice_mass_balance_kg_per_m2: mesh.area_weighted_mean(&annual_land_ice_balance),
        sea_ice_cover_balance_error: mesh.area_weighted_mean(&sea_ice_balance_errors),
    };
    Cryosphere {
        cell_snowfall_kg_per_m2_per_day: snowfall_rate,
        cell_melt_kg_per_m2_per_day: melt_rate,
        cell_snow_cover_fraction: snow_cover,
        cell_land_ice_cover_fraction: land_ice_cover,
        cell_sea_ice_cover_fraction: sea_ice_cover,
        diagnostics,
    }
}

fn validate(
    mesh: &SphereMesh,
    inputs: CryosphereInputs<'_>,
    config: CryosphereConfig,
) -> Result<(), CryosphereError> {
    if !ANNUAL_SAMPLE_RANGE.contains(&inputs.annual_sample_count) {
        return Err(CryosphereError::SampleCount);
    }
    let expected_temperature_count = mesh
        .cell_count()
        .checked_mul(inputs.annual_sample_count)
        .ok_or(CryosphereError::TemperatureCells)?;
    if inputs.annual_temperature_samples_kelvin.len() != expected_temperature_count {
        return Err(CryosphereError::TemperatureCells);
    }
    if inputs
        .annual_temperature_samples_kelvin
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(CryosphereError::Temperature);
    }
    if !inputs.selected_orbital_phase.is_finite() {
        return Err(CryosphereError::SelectedPhase);
    }
    validate_range(
        inputs.orbital_period_days,
        &ORBITAL_PERIOD_DAYS_RANGE,
        CryosphereError::OrbitalPeriod,
    )?;
    if inputs.precipitation_kg_per_m2_per_day.len() != mesh.cell_count() {
        return Err(CryosphereError::PrecipitationCells);
    }
    if inputs
        .precipitation_kg_per_m2_per_day
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(CryosphereError::Precipitation);
    }
    if inputs.final_elevation.len() != mesh.cell_count() {
        return Err(CryosphereError::ElevationCells);
    }
    if inputs
        .final_elevation
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(CryosphereError::Elevation);
    }
    validate_range(
        config.maximum_iterations,
        &CRYOSPHERE_ITERATION_LIMIT_RANGE,
        CryosphereError::IterationLimit,
    )?;
    validate_range(
        config.closure_tolerance,
        &CRYOSPHERE_CLOSURE_TOLERANCE_RANGE,
        CryosphereError::ClosureTolerance,
    )?;
    for temperature in [
        config.snowfall_temperature_kelvin,
        config.melt_temperature_kelvin,
    ] {
        validate_range(
            temperature,
            &CRYOSPHERE_TEMPERATURE_RANGE,
            CryosphereError::TemperatureThreshold,
        )?;
    }
    for mass in [
        config.full_snow_cover_kg_per_m2,
        config.seasonal_snow_capacity_kg_per_m2,
    ] {
        validate_range(mass, &CRYOSPHERE_MASS_RANGE, CryosphereError::SnowCapacity)?;
    }
    if config.full_snow_cover_kg_per_m2 <= 0.0
        || config.seasonal_snow_capacity_kg_per_m2 < config.full_snow_cover_kg_per_m2
    {
        return Err(CryosphereError::SnowCapacity);
    }
    for rate in [
        config.snow_melt_kg_per_m2_per_kelvin_day,
        config.land_ice_melt_kg_per_m2_per_kelvin_day,
    ] {
        validate_range(rate, &CRYOSPHERE_RATE_RANGE, CryosphereError::MeltRate)?;
    }
    for rate in [
        config.sea_ice_growth_fraction_per_kelvin_day,
        config.sea_ice_melt_fraction_per_kelvin_day,
    ] {
        validate_range(
            rate,
            &CRYOSPHERE_FRACTION_RATE_RANGE,
            CryosphereError::SeaIceRate,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_core::fingerprint;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::SphericalDelaunay;

    fn mesh() -> SphereMesh {
        let points = fibonacci_sphere(FibonacciConfig::new(64)).unwrap();
        let delaunay = SphericalDelaunay::build(points).unwrap();
        SphereMesh::from_delaunay(&delaunay, 1.0).unwrap()
    }

    fn run(
        mesh: &SphereMesh,
        temperatures: Vec<f32>,
        precipitation: Vec<f32>,
        elevation: Vec<f32>,
        phase: f64,
    ) -> Cryosphere {
        derive_cryosphere(
            mesh,
            CryosphereInputs {
                annual_temperature_samples_kelvin: &temperatures,
                annual_sample_count: temperatures.len() / mesh.cell_count(),
                selected_orbital_phase: phase,
                orbital_period_days: 360.0,
                precipitation_kg_per_m2_per_day: &precipitation,
                final_elevation: &elevation,
            },
            CryosphereConfig::EARTHLIKE,
        )
        .unwrap()
    }

    #[test]
    fn is_deterministic() {
        let mesh = mesh();
        let temperatures = vec![268.0; mesh.cell_count() * 12];
        let precipitation = vec![1.0; mesh.cell_count()];
        let elevation = (0..mesh.cell_count())
            .map(|cell| if cell % 2 == 0 { 0.7 } else { 0.3 })
            .collect::<Vec<_>>();
        let first = run(
            &mesh,
            temperatures.clone(),
            precipitation.clone(),
            elevation.clone(),
            0.25,
        );
        let second = run(&mesh, temperatures, precipitation, elevation, 0.25);
        assert_eq!(first, second);
        assert_eq!(
            fingerprint(
                first
                    .cell_snow_cover_fraction
                    .iter()
                    .chain(&first.cell_land_ice_cover_fraction)
                    .chain(&first.cell_sea_ice_cover_fraction)
                    .map(|value| u64::from(value.to_bits()))
            ),
            16_094_499_875_263_205_925
        );
    }

    #[test]
    fn warm_world_has_no_cryosphere() {
        let mesh = mesh();
        let result = run(
            &mesh,
            vec![300.0; mesh.cell_count() * 12],
            vec![2.0; mesh.cell_count()],
            vec![0.7; mesh.cell_count()],
            0.0,
        );
        assert_eq!(result.diagnostics.snow_covered_cell_count, 0);
        assert_eq!(result.diagnostics.land_ice_cell_count, 0);
        assert_eq!(result.diagnostics.sea_ice_cell_count, 0);
    }

    #[test]
    fn frozen_world_grows_land_and_sea_ice() {
        let mesh = mesh();
        let elevation = (0..mesh.cell_count())
            .map(|cell| if cell % 2 == 0 { 0.7 } else { 0.3 })
            .collect::<Vec<_>>();
        let result = run(
            &mesh,
            vec![250.0; mesh.cell_count() * 12],
            vec![1.0; mesh.cell_count()],
            elevation,
            0.5,
        );
        assert!(result.diagnostics.land_ice_cell_count > 0);
        assert!(result.diagnostics.sea_ice_cell_count > 0);
        assert_eq!(result.diagnostics.land_ice_cover_fraction.maximum, 1.0);
        assert_eq!(
            result.diagnostics.selected_sea_ice_cover_fraction.maximum,
            1.0
        );
        assert!(result.diagnostics.snow_mass_balance_error_kg_per_m2.abs() <= 1.0e-10);
        assert!(result.diagnostics.sea_ice_cover_balance_error.abs() <= 1.0e-10);
        assert!(result.diagnostics.land_ice_mass_balance_kg_per_m2 > 0.0);
    }

    #[test]
    fn dry_frozen_land_does_not_create_ice() {
        let mesh = mesh();
        let result = run(
            &mesh,
            vec![250.0; mesh.cell_count() * 12],
            vec![0.0; mesh.cell_count()],
            vec![0.7; mesh.cell_count()],
            0.0,
        );
        assert_eq!(result.diagnostics.snow_covered_cell_count, 0);
        assert_eq!(result.diagnostics.land_ice_cell_count, 0);
    }

    #[test]
    fn land_ice_melt_does_not_require_a_nonzero_snow_melt_factor() {
        let result = solve_land_cell(
            &[250.0, 290.0, 250.0, 290.0],
            1.0,
            1,
            90.0,
            CryosphereConfig {
                snow_melt_kg_per_m2_per_kelvin_day: 0.0,
                land_ice_melt_kg_per_m2_per_kelvin_day: 1.0,
                ..CryosphereConfig::EARTHLIKE
            },
        )
        .unwrap();

        assert!(result.land_ice_accumulation > 0.0);
        assert!(result.land_ice_ablation > 0.0);
        assert!(result.land_ice_cover > 0.0);
    }

    #[test]
    fn hemispheres_have_opposite_selected_snow_seasons() {
        let mesh = mesh();
        let samples = 12;
        let mut temperatures = Vec::with_capacity(mesh.cell_count() * samples);
        for center in &mesh.cell_centers {
            for sample in 0..samples {
                let seasonal =
                    (sample as f64 / samples as f64 * std::f64::consts::TAU).cos() as f32;
                temperatures.push(273.0 - center.y.signum() * seasonal * 15.0);
            }
        }
        let result = run(
            &mesh,
            temperatures,
            vec![1.0; mesh.cell_count()],
            vec![0.7; mesh.cell_count()],
            0.0,
        );
        let north = mesh
            .cell_centers
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.y.total_cmp(&b.1.y))
            .unwrap()
            .0;
        let south = mesh
            .cell_centers
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.y.total_cmp(&b.1.y))
            .unwrap()
            .0;
        assert!(
            result.cell_snowfall_kg_per_m2_per_day[north]
                > result.cell_snowfall_kg_per_m2_per_day[south]
        );
        assert!(result.cell_snow_cover_fraction[north] > result.cell_snow_cover_fraction[south]);
    }

    #[test]
    fn cold_nonpolar_land_can_form_ice() {
        let mesh = mesh();
        let nonpolar = mesh
            .cell_centers
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.y.abs().total_cmp(&b.1.y.abs()))
            .unwrap()
            .0;
        let mut temperatures = vec![300.0; mesh.cell_count() * 12];
        temperatures[nonpolar * 12..(nonpolar + 1) * 12].fill(250.0);
        let result = run(
            &mesh,
            temperatures,
            vec![1.0; mesh.cell_count()],
            vec![0.7; mesh.cell_count()],
            0.0,
        );
        assert_eq!(result.diagnostics.land_ice_cell_count, 1);
        assert_eq!(result.cell_land_ice_cover_fraction[nonpolar], 1.0);
    }
}
