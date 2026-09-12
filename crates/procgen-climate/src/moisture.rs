use crate::{
    AreaWeightedSummary, ClimateOutputError, SECONDS_PER_DAY,
    advection::{Route, advect, build_routes},
    validate_range,
};
use procgen_core::Vec3;
use procgen_planet::{Planet, PlanetValidationError};
use procgen_sphere_mesh::{SphereMesh, default_hop_length, mean_cell_width};
use procgen_tectonics::{ElevationField, land_elevation_meters};
use std::{fmt, ops::RangeInclusive};

pub const MOISTURE_STEP_COUNT_RANGE: RangeInclusive<usize> = 1..=4_096;
/// A second to a month. It bounds a step the mesh derives rather than one a
/// caller types, and the coarsest mesh the viewer offers takes the whole run
/// in one step, so the top is the longest run rather than the week it was when
/// the step itself was configured.
pub const MOISTURE_STEP_SECONDS_RANGE: RangeInclusive<f64> = 1.0..=2_592_000.0;
/// From one step of the default mesh to a decade.
pub const MOISTURE_SIMULATED_DAYS_RANGE: RangeInclusive<f64> = 0.25..=3_650.0;

/// Steps the default mesh takes. Transport is advective, so a step may not
/// carry moisture further than a cell: a mesh with cells half as wide needs
/// steps half as long, and twice as many of them to cover the same weather.
/// The count therefore goes as the reciprocal of the cell width, and this is
/// what it was measured at on the default mesh.
const DEFAULT_MESH_STEP_COUNT: f64 = 120.0;
pub const MOISTURE_CAPACITY_RANGE: RangeInclusive<f64> = 0.0..=10_000.0;
pub const REFERENCE_TEMPERATURE_KELVIN_RANGE: RangeInclusive<f64> = 0.0..=10_000.0;
pub const MOISTURE_RATE_RANGE: RangeInclusive<f64> = 0.0..=1.0;
pub const TEMPERATURE_SENSITIVITY_RANGE: RangeInclusive<f64> = 0.0..=1.0;
pub const OROGRAPHIC_COEFFICIENT_RANGE: RangeInclusive<f64> = 0.0..=1.0;
pub const TRANSPORT_FRACTION_RANGE: RangeInclusive<f64> = 0.0..=1.0;

/// How long a mesh's transport runs for, and in how many pieces.
///
/// The pieces are the mesh's business and the length is the config's, so a
/// finer mesh resolves the same weather rather than less of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoistureSchedule {
    pub step_count: usize,
    pub step_seconds: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoistureTransportConfig {
    /// Days of weather the transport simulates. The step count follows from
    /// the mesh and the seconds a step stands for follow from both, so this
    /// is the whole of what a caller states about the run's length.
    pub simulated_days: f64,
    /// Column water capacity at the reference temperature, in kg/m2.
    pub reference_capacity_kg_per_m2: f64,
    pub reference_temperature_kelvin: f64,
    /// Exponential capacity response per kelvin.
    pub capacity_temperature_sensitivity_per_kelvin: f64,
    pub minimum_capacity_kg_per_m2: f64,
    pub maximum_capacity_kg_per_m2: f64,
    /// Ocean relaxation rate toward local moisture capacity.
    pub ocean_evaporation_rate_per_second: f64,
    /// Background conversion of airborne moisture to rainfall.
    pub rainfall_rate_per_second: f64,
    /// Conversion rate per meter of positive terrain ascent.
    pub orographic_coefficient_per_meter: f64,
    /// Hard bound on the fraction removed orographically in one step.
    pub maximum_orographic_fraction_per_step: f64,
    /// CFL-style bound on the humidity exported from a cell in one step.
    ///
    /// It, and not the wind, is what sets how far moisture reaches inland at
    /// the defaults: the clamp binds over most of the world, so a stronger
    /// wind moves the same water. That is a separate problem from how a run
    /// is sliced and this slice does not address it.
    pub maximum_transport_fraction_per_step: f64,
}

/// The longest run a mesh of `cell_count` cells can simulate, in days: the
/// steps it derives, each standing for the longest the solver will take.
///
/// A coarse mesh derives few steps, so a month shared over them puts more in
/// each than the explicit solver's per-step fractions mean anything for. The
/// viewer bounds its slider by this the way it bounds a step duration by
/// [`procgen_tectonics::maximum_step_duration`].
pub fn maximum_simulated_days(cell_count: usize) -> f64 {
    let steps = MoistureTransportConfig::EARTHLIKE
        .schedule(cell_count)
        .step_count as f64;
    steps * MOISTURE_STEP_SECONDS_RANGE.end() / SECONDS_PER_DAY
}

impl MoistureTransportConfig {
    /// The schedule a mesh of `cell_count` cells runs this config on.
    ///
    /// The step count scales with the reciprocal of the cell width, because a
    /// step may not carry moisture past a cell; the seconds a step stands for
    /// are then the simulated days shared out over them. On the default mesh
    /// that is the 120 steps of six hours this was measured at.
    pub fn schedule(&self, cell_count: usize) -> MoistureSchedule {
        let width_ratio =
            f64::from(default_hop_length()) / f64::from(mean_cell_width(1.0, cell_count));
        let step_count = (DEFAULT_MESH_STEP_COUNT * width_ratio).round().max(1.0);
        MoistureSchedule {
            step_count: step_count as usize,
            step_seconds: self.simulated_days * SECONDS_PER_DAY / step_count,
        }
    }
}

impl MoistureTransportConfig {
    /// Convenient Earth-like choices. The solver contains no planet-specific values.
    pub const EARTHLIKE: Self = Self {
        simulated_days: 30.0,
        reference_capacity_kg_per_m2: 25.0,
        reference_temperature_kelvin: 288.0,
        capacity_temperature_sensitivity_per_kelvin: 0.07,
        minimum_capacity_kg_per_m2: 0.2,
        maximum_capacity_kg_per_m2: 100.0,
        ocean_evaporation_rate_per_second: 1.5e-6,
        rainfall_rate_per_second: 8.0e-7,
        orographic_coefficient_per_meter: 2.0e-4,
        maximum_orographic_fraction_per_step: 0.35,
        maximum_transport_fraction_per_step: 0.5,
    };
}

#[derive(Clone, Copy, Debug)]
pub struct MoistureTransportInputs<'a> {
    pub planet: Planet,
    pub selected_temperature_kelvin: &'a [f32],
    pub final_elevation: ElevationField<'a>,
    pub cell_wind_meters_per_second: &'a [Vec3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoistureTransportDiagnostics {
    pub humidity_kg_per_m2: AreaWeightedSummary,
    pub moisture_capacity_kg_per_m2: AreaWeightedSummary,
    pub evaporation_kg_per_m2_per_day: AreaWeightedSummary,
    pub precipitation_kg_per_m2_per_day: AreaWeightedSummary,
    pub condensation_kg_per_m2_per_day: AreaWeightedSummary,
    pub orographic_precipitation_kg_per_m2_per_day: AreaWeightedSummary,
    pub simulated_days: f64,
    /// What the mesh made of the configured run. It is a fact about this
    /// result rather than a config value echoed back: the config states the
    /// days and the mesh decides the slicing.
    pub schedule: MoistureSchedule,
    pub ocean_cell_count: usize,
    pub precipitating_cell_count: usize,
    pub orographic_cell_count: usize,
    pub maximum_orographic_fraction_per_step: f64,
    /// Area-weighted evaporation minus final humidity and precipitation.
    pub mass_balance_error_kg_per_m2: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MoistureTransport {
    pub cell_humidity_kg_per_m2: Vec<f32>,
    pub cell_moisture_capacity_kg_per_m2: Vec<f32>,
    pub cell_evaporation_kg_per_m2_per_day: Vec<f32>,
    pub cell_precipitation_kg_per_m2_per_day: Vec<f32>,
    pub cell_condensation_kg_per_m2_per_day: Vec<f32>,
    pub cell_orographic_precipitation_kg_per_m2_per_day: Vec<f32>,
    pub diagnostics: MoistureTransportDiagnostics,
}

impl MoistureTransport {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), ClimateOutputError> {
        let cells = mesh.cell_count();
        if self.cell_humidity_kg_per_m2.len() != cells
            || self.cell_moisture_capacity_kg_per_m2.len() != cells
            || self.cell_evaporation_kg_per_m2_per_day.len() != cells
            || self.cell_precipitation_kg_per_m2_per_day.len() != cells
            || self.cell_condensation_kg_per_m2_per_day.len() != cells
            || self.cell_orographic_precipitation_kg_per_m2_per_day.len() != cells
        {
            return Err(ClimateOutputError::MoistureTransport);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoistureTransportError {
    Planet(PlanetValidationError),
    TemperatureCells,
    ElevationCells,
    WindCells,
    Temperature,
    Elevation,
    Wind,
    SimulatedDays,
    StepCount,
    StepSeconds,
    Capacity,
    ReferenceTemperature,
    TemperatureSensitivity,
    EvaporationRate,
    RainfallRate,
    OrographicCoefficient,
    OrographicFraction,
    TransportFraction,
    NumericalRange,
}

impl fmt::Display for MoistureTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use MoistureTransportError as Error;
        match self {
            Error::Planet(error) => error.fmt(formatter),
            Error::TemperatureCells => formatter.write_str("temperature must match mesh cells"),
            Error::ElevationCells => formatter.write_str("elevation must match mesh cells"),
            Error::WindCells => formatter.write_str("wind must match mesh cells"),
            Error::Temperature => formatter.write_str("temperature must be finite and nonnegative"),
            Error::Elevation => formatter.write_str("elevation must contain only finite values"),
            Error::Wind => formatter.write_str("wind must contain only finite vectors"),
            Error::SimulatedDays => {
                formatter.write_str("simulated days is outside its supported range")
            }
            Error::StepCount => formatter.write_str(
                "the step count this mesh derives is outside the solver's supported range",
            ),
            Error::StepSeconds => formatter.write_str(
                "the step duration this mesh derives is outside the solver's supported range",
            ),
            Error::Capacity => formatter.write_str("moisture capacity parameters are invalid"),
            Error::ReferenceTemperature => {
                formatter.write_str("reference temperature is outside its supported range")
            }
            Error::TemperatureSensitivity => {
                formatter.write_str("capacity temperature sensitivity is invalid")
            }
            Error::EvaporationRate => formatter.write_str("ocean evaporation rate is invalid"),
            Error::RainfallRate => formatter.write_str("rainfall rate is invalid"),
            Error::OrographicCoefficient => {
                formatter.write_str("orographic coefficient is invalid")
            }
            Error::OrographicFraction => {
                formatter.write_str("maximum orographic fraction must be in [0, 1]")
            }
            Error::TransportFraction => {
                formatter.write_str("maximum transport fraction must be in [0, 1]")
            }
            Error::NumericalRange => {
                formatter.write_str("moisture transport is outside the finite f32 output range")
            }
        }
    }
}

impl std::error::Error for MoistureTransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Planet(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PlanetValidationError> for MoistureTransportError {
    fn from(error: PlanetValidationError) -> Self {
        Self::Planet(error)
    }
}

struct CellModel {
    areas: Vec<f64>,
    capacity_mass: Vec<f64>,
    capacity_kg_per_m2: Vec<f32>,
    evaporation_fraction: Vec<f64>,
    orographic_fraction: Vec<f64>,
    ocean_cell_count: usize,
    maximum_orographic_fraction: f64,
}

impl CellModel {
    fn new(
        mesh: &SphereMesh,
        inputs: MoistureTransportInputs<'_>,
        config: MoistureTransportConfig,
        schedule: MoistureSchedule,
    ) -> Self {
        let areas = mesh
            .cell_areas
            .iter()
            .map(|&area| f64::from(area))
            .collect::<Vec<_>>();
        let elevation_meters = inputs
            .final_elevation
            .cell_elevations
            .iter()
            .map(|&elevation| {
                land_elevation_meters(
                    elevation,
                    inputs.final_elevation.sea_level,
                    inputs.planet.maximum_land_elevation_meters,
                ) as f32
            })
            .collect::<Vec<_>>();
        let elevation_gradients = mesh.cell_gradients(&elevation_meters);
        let ocean_evaporation_fraction =
            1.0 - (-config.ocean_evaporation_rate_per_second * schedule.step_seconds).exp();

        let mut capacity_mass = Vec::with_capacity(mesh.cell_count());
        let mut capacity_kg_per_m2 = Vec::with_capacity(mesh.cell_count());
        let mut evaporation_fraction = Vec::with_capacity(mesh.cell_count());
        let mut orographic_fraction = Vec::with_capacity(mesh.cell_count());
        let mut ocean_cell_count = 0;
        for cell in 0..mesh.cell_count() {
            let capacity = (config.reference_capacity_kg_per_m2
                * (config.capacity_temperature_sensitivity_per_kelvin
                    * (f64::from(inputs.selected_temperature_kelvin[cell])
                        - config.reference_temperature_kelvin))
                    .exp())
            .clamp(
                config.minimum_capacity_kg_per_m2,
                config.maximum_capacity_kg_per_m2,
            );
            capacity_mass.push(capacity * areas[cell]);
            capacity_kg_per_m2.push(capacity as f32);

            let land = inputs.final_elevation.is_land(cell);
            evaporation_fraction.push(if land {
                0.0
            } else {
                ocean_cell_count += 1;
                ocean_evaporation_fraction
            });
            let ascent_meters_per_second = if land {
                f64::from(inputs.cell_wind_meters_per_second[cell].dot(elevation_gradients[cell]))
                    .max(0.0)
                    / inputs.planet.radius_meters
            } else {
                0.0
            };
            orographic_fraction.push(
                (1.0 - (-config.orographic_coefficient_per_meter
                    * ascent_meters_per_second
                    * schedule.step_seconds)
                    .exp())
                .min(config.maximum_orographic_fraction_per_step),
            );
        }
        let maximum_orographic_fraction = orographic_fraction.iter().copied().fold(0.0, f64::max);
        Self {
            areas,
            capacity_mass,
            capacity_kg_per_m2,
            evaporation_fraction,
            orographic_fraction,
            ocean_cell_count,
            maximum_orographic_fraction,
        }
    }
}

struct WaterBudget {
    humidity_mass: Vec<f64>,
    transport_scratch: Vec<f64>,
    evaporation_mass: Vec<f64>,
    precipitation_mass: Vec<f64>,
    condensation_mass: Vec<f64>,
    orographic_mass: Vec<f64>,
}

impl WaterBudget {
    fn new(cell_count: usize) -> Self {
        Self {
            humidity_mass: vec![0.0; cell_count],
            transport_scratch: vec![0.0; cell_count],
            evaporation_mass: vec![0.0; cell_count],
            precipitation_mass: vec![0.0; cell_count],
            condensation_mass: vec![0.0; cell_count],
            orographic_mass: vec![0.0; cell_count],
        }
    }

    fn evaporate(&mut self, model: &CellModel) {
        for cell in 0..self.humidity_mass.len() {
            let evaporated = (model.capacity_mass[cell] - self.humidity_mass[cell]).max(0.0)
                * model.evaporation_fraction[cell];
            self.humidity_mass[cell] += evaporated;
            self.evaporation_mass[cell] += evaporated;
        }
    }

    fn transport(&mut self, routes: &[Route]) {
        advect(routes, &mut self.humidity_mass, &mut self.transport_scratch);
    }

    fn precipitate(&mut self, model: &CellModel, rainfall_fraction: f64) {
        for cell in 0..self.humidity_mass.len() {
            let condensed = (self.humidity_mass[cell] - model.capacity_mass[cell]).max(0.0);
            let after_condensation = self.humidity_mass[cell] - condensed;
            let rainfall = after_condensation * rainfall_fraction;
            let orographic = (after_condensation - rainfall) * model.orographic_fraction[cell];
            self.humidity_mass[cell] -= condensed + rainfall + orographic;
            self.condensation_mass[cell] += condensed;
            self.orographic_mass[cell] += orographic;
            self.precipitation_mass[cell] += condensed + rainfall + orographic;
        }
    }

    fn finish(
        self,
        mesh: &SphereMesh,
        model: CellModel,
        schedule: MoistureSchedule,
    ) -> Result<MoistureTransport, MoistureTransportError> {
        let humidity = columns(&self.humidity_mass, &model.areas, 1.0);
        let simulated_days = schedule.step_count as f64 * schedule.step_seconds / SECONDS_PER_DAY;
        let evaporation = columns(&self.evaporation_mass, &model.areas, 1.0 / simulated_days);
        let precipitation = columns(&self.precipitation_mass, &model.areas, 1.0 / simulated_days);
        let condensation = columns(&self.condensation_mass, &model.areas, 1.0 / simulated_days);
        let orographic = columns(&self.orographic_mass, &model.areas, 1.0 / simulated_days);
        let mass_balance_error_kg_per_m2 = (self.evaporation_mass.iter().sum::<f64>()
            - self.humidity_mass.iter().sum::<f64>()
            - self.precipitation_mass.iter().sum::<f64>())
            / mesh.total_area();
        let diagnostics = MoistureTransportDiagnostics {
            humidity_kg_per_m2: AreaWeightedSummary::from_field(mesh, &humidity),
            moisture_capacity_kg_per_m2: AreaWeightedSummary::from_field(
                mesh,
                &model.capacity_kg_per_m2,
            ),
            evaporation_kg_per_m2_per_day: AreaWeightedSummary::from_field(mesh, &evaporation),
            precipitation_kg_per_m2_per_day: AreaWeightedSummary::from_field(mesh, &precipitation),
            condensation_kg_per_m2_per_day: AreaWeightedSummary::from_field(mesh, &condensation),
            orographic_precipitation_kg_per_m2_per_day: AreaWeightedSummary::from_field(
                mesh,
                &orographic,
            ),
            simulated_days,
            schedule,
            ocean_cell_count: model.ocean_cell_count,
            precipitating_cell_count: precipitation.iter().filter(|&&value| value > 0.0).count(),
            orographic_cell_count: orographic.iter().filter(|&&value| value > 0.0).count(),
            maximum_orographic_fraction_per_step: model.maximum_orographic_fraction,
            mass_balance_error_kg_per_m2,
        };
        let summaries = [
            diagnostics.humidity_kg_per_m2,
            diagnostics.moisture_capacity_kg_per_m2,
            diagnostics.evaporation_kg_per_m2_per_day,
            diagnostics.precipitation_kg_per_m2_per_day,
            diagnostics.condensation_kg_per_m2_per_day,
            diagnostics.orographic_precipitation_kg_per_m2_per_day,
        ];
        if !summaries.iter().all(|summary| summary.is_finite())
            || !mass_balance_error_kg_per_m2.is_finite()
            || [
                &humidity,
                &model.capacity_kg_per_m2,
                &evaporation,
                &precipitation,
                &condensation,
                &orographic,
            ]
            .into_iter()
            .flatten()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(MoistureTransportError::NumericalRange);
        }
        Ok(MoistureTransport {
            cell_humidity_kg_per_m2: humidity,
            cell_moisture_capacity_kg_per_m2: model.capacity_kg_per_m2,
            cell_evaporation_kg_per_m2_per_day: evaporation,
            cell_precipitation_kg_per_m2_per_day: precipitation,
            cell_condensation_kg_per_m2_per_day: condensation,
            cell_orographic_precipitation_kg_per_m2_per_day: orographic,
            diagnostics,
        })
    }
}

pub fn derive_moisture_transport(
    mesh: &SphereMesh,
    inputs: MoistureTransportInputs<'_>,
    config: MoistureTransportConfig,
) -> Result<MoistureTransport, MoistureTransportError> {
    let schedule = validate(mesh, inputs, config)?;
    let model = CellModel::new(mesh, inputs, config, schedule);
    let routes = build_routes(
        mesh,
        inputs.cell_wind_meters_per_second,
        inputs.planet.radius_meters,
        schedule.step_seconds,
        config.maximum_transport_fraction_per_step,
    );

    let rainfall_fraction = 1.0 - (-config.rainfall_rate_per_second * schedule.step_seconds).exp();
    let mut budget = WaterBudget::new(mesh.cell_count());
    for _ in 0..schedule.step_count {
        budget.evaporate(&model);
        budget.transport(&routes);
        budget.precipitate(&model, rainfall_fraction);
    }
    budget.finish(mesh, model, schedule)
}

fn columns(mass: &[f64], areas: &[f64], scale: f64) -> Vec<f32> {
    mass.iter()
        .zip(areas)
        .map(|(&mass, &area)| (mass / area * scale) as f32)
        .collect()
}

fn validate(
    mesh: &SphereMesh,
    inputs: MoistureTransportInputs<'_>,
    config: MoistureTransportConfig,
) -> Result<MoistureSchedule, MoistureTransportError> {
    inputs.planet.validate()?;
    if inputs.selected_temperature_kelvin.len() != mesh.cell_count() {
        return Err(MoistureTransportError::TemperatureCells);
    }
    if inputs.final_elevation.cell_elevations.len() != mesh.cell_count() {
        return Err(MoistureTransportError::ElevationCells);
    }
    if inputs.cell_wind_meters_per_second.len() != mesh.cell_count() {
        return Err(MoistureTransportError::WindCells);
    }
    if inputs
        .selected_temperature_kelvin
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(MoistureTransportError::Temperature);
    }
    if inputs
        .final_elevation
        .cell_elevations
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(MoistureTransportError::Elevation);
    }
    if inputs
        .cell_wind_meters_per_second
        .iter()
        .any(|wind| !wind.x.is_finite() || !wind.y.is_finite() || !wind.z.is_finite())
    {
        return Err(MoistureTransportError::Wind);
    }
    validate_range(
        config.simulated_days,
        &MOISTURE_SIMULATED_DAYS_RANGE,
        MoistureTransportError::SimulatedDays,
    )?;
    // The schedule is derived, so these two bound the mesh rather than the
    // config: a mesh fine enough to ask for more steps than the solver will
    // take is rejected rather than run at a coarser schedule than it needs.
    let schedule = config.schedule(mesh.cell_count());
    validate_range(
        schedule.step_count,
        &MOISTURE_STEP_COUNT_RANGE,
        MoistureTransportError::StepCount,
    )?;
    validate_range(
        schedule.step_seconds,
        &MOISTURE_STEP_SECONDS_RANGE,
        MoistureTransportError::StepSeconds,
    )?;
    for capacity in [
        config.reference_capacity_kg_per_m2,
        config.minimum_capacity_kg_per_m2,
        config.maximum_capacity_kg_per_m2,
    ] {
        validate_range(
            capacity,
            &MOISTURE_CAPACITY_RANGE,
            MoistureTransportError::Capacity,
        )?;
    }
    if config.minimum_capacity_kg_per_m2 > config.maximum_capacity_kg_per_m2 {
        return Err(MoistureTransportError::Capacity);
    }
    validate_range(
        config.reference_temperature_kelvin,
        &REFERENCE_TEMPERATURE_KELVIN_RANGE,
        MoistureTransportError::ReferenceTemperature,
    )?;
    validate_range(
        config.capacity_temperature_sensitivity_per_kelvin,
        &TEMPERATURE_SENSITIVITY_RANGE,
        MoistureTransportError::TemperatureSensitivity,
    )?;
    validate_range(
        config.ocean_evaporation_rate_per_second,
        &MOISTURE_RATE_RANGE,
        MoistureTransportError::EvaporationRate,
    )?;
    validate_range(
        config.rainfall_rate_per_second,
        &MOISTURE_RATE_RANGE,
        MoistureTransportError::RainfallRate,
    )?;
    validate_range(
        config.orographic_coefficient_per_meter,
        &OROGRAPHIC_COEFFICIENT_RANGE,
        MoistureTransportError::OrographicCoefficient,
    )?;
    validate_range(
        config.maximum_orographic_fraction_per_step,
        &TRANSPORT_FRACTION_RANGE,
        MoistureTransportError::OrographicFraction,
    )?;
    validate_range(
        config.maximum_transport_fraction_per_step,
        &TRANSPORT_FRACTION_RANGE,
        MoistureTransportError::TransportFraction,
    )?;
    Ok(schedule)
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::DEFAULT_CELL_COUNT;
    use procgen_sphere_mesh::SphericalDelaunay;

    /// Lends a synthetic field against the datum the tectonic pipeline
    /// defaults to, which these elevations are written against.
    fn elevation_field(cell_elevations: &[f32]) -> ElevationField<'_> {
        ElevationField {
            cell_elevations,
            sea_level: procgen_tectonics::CoarseElevationConfig::default().sea_level,
        }
    }

    /// The Earth-like preset at the default mesh's six-hour step, however many
    /// steps `cell_count` derives.
    ///
    /// The orographic constants were tuned against that step, and a mesh this
    /// coarse derives so few steps that a month in each washes the barrier's
    /// contribution out. Shortening the run keeps the step.
    fn earthlike_step_on(cell_count: usize) -> MoistureTransportConfig {
        let earthlike = MoistureTransportConfig::EARTHLIKE;
        let steps = earthlike.schedule(cell_count).step_count as f64;
        let default_steps = earthlike.schedule(DEFAULT_CELL_COUNT).step_count as f64;
        MoistureTransportConfig {
            simulated_days: earthlike.simulated_days * steps / default_steps,
            ..earthlike
        }
    }

    fn mesh(count: usize) -> SphereMesh {
        let points = fibonacci_sphere(FibonacciConfig::new(count)).unwrap();
        let delaunay = SphericalDelaunay::build(points).unwrap();
        SphereMesh::from_delaunay(&delaunay, 1.0).unwrap()
    }

    fn run(
        mesh: &SphereMesh,
        temperature: &[f32],
        elevations: &[f32],
        wind: &[Vec3],
        config: MoistureTransportConfig,
    ) -> MoistureTransport {
        derive_moisture_transport(
            mesh,
            MoistureTransportInputs {
                planet: Planet::EARTH,
                selected_temperature_kelvin: temperature,
                final_elevation: elevation_field(elevations),
                cell_wind_meters_per_second: wind,
            },
            config,
        )
        .unwrap()
    }

    fn eastward_wind(mesh: &SphereMesh, speed: f32) -> Vec<Vec3> {
        mesh.cell_centers
            .iter()
            .map(|&point| {
                let normal = point.normalized();
                let east = Vec3::new(0.0, 1.0, 0.0).cross(normal).normalized();
                east * speed
            })
            .collect()
    }

    #[test]
    fn validates_inputs_and_configuration_boundaries() {
        let mesh = mesh(32);
        let temperature = vec![288.0; mesh.cell_count()];
        let elevation = vec![0.2; mesh.cell_count()];
        let wind = vec![Vec3::ZERO; mesh.cell_count()];
        let error = |planet, temperature: &[f32], elevations: &[f32], wind: &[Vec3], config| {
            derive_moisture_transport(
                &mesh,
                MoistureTransportInputs {
                    planet,
                    selected_temperature_kelvin: temperature,
                    final_elevation: elevation_field(elevations),
                    cell_wind_meters_per_second: wind,
                },
                config,
            )
            .unwrap_err()
        };

        let mut planet = Planet::EARTH;
        planet.maximum_land_elevation_meters = -1.0;
        assert_eq!(
            error(
                planet,
                &temperature,
                &elevation,
                &wind,
                MoistureTransportConfig::EARTHLIKE
            ),
            MoistureTransportError::Planet(PlanetValidationError::MaximumLandElevation)
        );
        assert_eq!(
            error(
                Planet::EARTH,
                &temperature[..31],
                &elevation,
                &wind,
                MoistureTransportConfig::EARTHLIKE
            ),
            MoistureTransportError::TemperatureCells
        );
        let mut invalid_temperature = temperature.clone();
        invalid_temperature[0] = f32::NAN;
        assert_eq!(
            error(
                Planet::EARTH,
                &invalid_temperature,
                &elevation,
                &wind,
                MoistureTransportConfig::EARTHLIKE
            ),
            MoistureTransportError::Temperature
        );
        let cases = [
            (
                MoistureTransportConfig {
                    simulated_days: 0.0,
                    ..MoistureTransportConfig::EARTHLIKE
                },
                MoistureTransportError::SimulatedDays,
            ),
            (
                MoistureTransportConfig {
                    simulated_days: f64::NAN,
                    ..MoistureTransportConfig::EARTHLIKE
                },
                MoistureTransportError::SimulatedDays,
            ),
            (
                // A decade over the three steps this mesh derives is years
                // in each one, which the solver will not take.
                MoistureTransportConfig {
                    simulated_days: *MOISTURE_SIMULATED_DAYS_RANGE.end(),
                    ..MoistureTransportConfig::EARTHLIKE
                },
                MoistureTransportError::StepSeconds,
            ),
            (
                MoistureTransportConfig {
                    reference_capacity_kg_per_m2: -1.0,
                    ..MoistureTransportConfig::EARTHLIKE
                },
                MoistureTransportError::Capacity,
            ),
            (
                MoistureTransportConfig {
                    minimum_capacity_kg_per_m2: 50.0,
                    maximum_capacity_kg_per_m2: 25.0,
                    ..MoistureTransportConfig::EARTHLIKE
                },
                MoistureTransportError::Capacity,
            ),
            (
                MoistureTransportConfig {
                    reference_temperature_kelvin: f64::INFINITY,
                    ..MoistureTransportConfig::EARTHLIKE
                },
                MoistureTransportError::ReferenceTemperature,
            ),
            (
                MoistureTransportConfig {
                    capacity_temperature_sensitivity_per_kelvin: f64::NAN,
                    ..MoistureTransportConfig::EARTHLIKE
                },
                MoistureTransportError::TemperatureSensitivity,
            ),
            (
                MoistureTransportConfig {
                    ocean_evaporation_rate_per_second: f64::NAN,
                    ..MoistureTransportConfig::EARTHLIKE
                },
                MoistureTransportError::EvaporationRate,
            ),
            (
                MoistureTransportConfig {
                    rainfall_rate_per_second: f64::NAN,
                    ..MoistureTransportConfig::EARTHLIKE
                },
                MoistureTransportError::RainfallRate,
            ),
            (
                MoistureTransportConfig {
                    orographic_coefficient_per_meter: f64::NAN,
                    ..MoistureTransportConfig::EARTHLIKE
                },
                MoistureTransportError::OrographicCoefficient,
            ),
            (
                MoistureTransportConfig {
                    maximum_orographic_fraction_per_step: f64::NAN,
                    ..MoistureTransportConfig::EARTHLIKE
                },
                MoistureTransportError::OrographicFraction,
            ),
            (
                MoistureTransportConfig {
                    maximum_transport_fraction_per_step: f64::NAN,
                    ..MoistureTransportConfig::EARTHLIKE
                },
                MoistureTransportError::TransportFraction,
            ),
        ];
        for (config, expected) in cases {
            assert_eq!(
                error(Planet::EARTH, &temperature, &elevation, &wind, config),
                expected
            );
        }
    }

    /// The run is the days and the mesh decides the slicing, so a finer mesh
    /// resolves the same weather rather than less of it.
    #[test]
    fn the_schedule_is_the_days_over_the_steps_the_mesh_needs() {
        let config = MoistureTransportConfig::EARTHLIKE;
        assert_eq!(
            config.schedule(DEFAULT_CELL_COUNT),
            MoistureSchedule {
                step_count: 120,
                step_seconds: 21_600.0,
            }
        );
        // Four times the cells halves the cell width, so the same month takes
        // twice the steps and each stands for half as long.
        assert_eq!(
            config.schedule(4 * DEFAULT_CELL_COUNT),
            MoistureSchedule {
                step_count: 240,
                step_seconds: 10_800.0,
            }
        );
    }

    #[test]
    fn moisture_transport_is_deterministic() {
        let mesh = mesh(512);
        let temperature = mesh
            .cell_centers
            .iter()
            .map(|point| 285.0 + 12.0 * point.y)
            .collect::<Vec<_>>();
        let elevation = mesh
            .cell_centers
            .iter()
            .map(|point| 0.5 + 0.25 * point.x)
            .collect::<Vec<_>>();
        let wind = eastward_wind(&mesh, 20.0);
        let first = run(
            &mesh,
            &temperature,
            &elevation,
            &wind,
            MoistureTransportConfig::EARTHLIKE,
        );
        let second = run(
            &mesh,
            &temperature,
            &elevation,
            &wind,
            MoistureTransportConfig::EARTHLIKE,
        );
        assert_eq!(first, second);
        // Water leaves the ocean, some of it falls, and the rest is still aloft
        // when the run ends, so rain is positive and stays under evaporation.
        let precipitation = first
            .diagnostics
            .precipitation_kg_per_m2_per_day
            .area_weighted_mean;
        let evaporation = first
            .diagnostics
            .evaporation_kg_per_m2_per_day
            .area_weighted_mean;
        assert!(
            0.0 < precipitation && precipitation < evaporation,
            "{precipitation} kg/m2/day of rain against {evaporation} evaporated"
        );
    }

    #[test]
    fn water_budget_is_conserved() {
        let mesh = mesh(512);
        let temperature = vec![288.0; mesh.cell_count()];
        let elevation = mesh
            .cell_centers
            .iter()
            .map(|point| 0.5 + 0.2 * point.x)
            .collect::<Vec<_>>();
        let result = run(
            &mesh,
            &temperature,
            &elevation,
            &eastward_wind(&mesh, 30.0),
            MoistureTransportConfig::EARTHLIKE,
        );
        assert!(result.diagnostics.mass_balance_error_kg_per_m2.abs() <= 1.0e-10);
    }

    #[test]
    fn dry_world_has_no_humidity_or_precipitation() {
        let mesh = mesh(128);
        let temperature = vec![300.0; mesh.cell_count()];
        let elevation = vec![0.8; mesh.cell_count()];
        let result = run(
            &mesh,
            &temperature,
            &elevation,
            &eastward_wind(&mesh, 30.0),
            MoistureTransportConfig::EARTHLIKE,
        );
        assert_eq!(result.diagnostics.ocean_cell_count, 0);
        assert!(
            result
                .cell_humidity_kg_per_m2
                .iter()
                .all(|&value| value == 0.0)
        );
        assert!(
            result
                .cell_precipitation_kg_per_m2_per_day
                .iter()
                .all(|&value| value == 0.0)
        );
    }

    #[test]
    fn all_ocean_world_closes_its_evaporation_and_rainfall_budget() {
        let mesh = mesh(128);
        let temperature = vec![295.0; mesh.cell_count()];
        let elevation = vec![0.2; mesh.cell_count()];
        let result = run(
            &mesh,
            &temperature,
            &elevation,
            &eastward_wind(&mesh, 20.0),
            MoistureTransportConfig::EARTHLIKE,
        );
        assert_eq!(result.diagnostics.ocean_cell_count, mesh.cell_count());
        assert!(
            result
                .diagnostics
                .evaporation_kg_per_m2_per_day
                .area_weighted_mean
                > 0.0
        );
        assert!(
            result
                .diagnostics
                .precipitation_kg_per_m2_per_day
                .area_weighted_mean
                > 0.0
        );
        assert!(result.diagnostics.mass_balance_error_kg_per_m2.abs() <= 1.0e-10);
    }

    #[test]
    fn zero_wind_does_not_move_ocean_moisture_onto_land() {
        let mesh = mesh(256);
        let temperature = vec![288.0; mesh.cell_count()];
        let elevation = mesh
            .cell_centers
            .iter()
            .map(|point| if point.x < 0.0 { 0.2 } else { 0.8 })
            .collect::<Vec<_>>();
        let result = run(
            &mesh,
            &temperature,
            &elevation,
            &vec![Vec3::ZERO; mesh.cell_count()],
            MoistureTransportConfig::EARTHLIKE,
        );
        let elevation = elevation_field(&elevation);
        for cell in 0..mesh.cell_count() {
            if elevation.is_land(cell) {
                assert_eq!(result.cell_humidity_kg_per_m2[cell], 0.0);
                assert_eq!(result.cell_precipitation_kg_per_m2_per_day[cell], 0.0);
            }
        }
    }

    #[test]
    fn terrain_barrier_adds_bounded_windward_precipitation() {
        let mesh = mesh(2_048);
        let temperature = vec![295.0; mesh.cell_count()];
        let elevation = mesh
            .cell_centers
            .iter()
            .map(|point| {
                if point.z < -0.25 {
                    0.2
                } else if point.z < 0.15 {
                    0.58
                } else if point.z < 0.35 {
                    0.9
                } else {
                    0.58
                }
            })
            .collect::<Vec<_>>();
        let wind = mesh
            .cell_centers
            .iter()
            .map(|&point| {
                let normal = point.normalized();
                let northward = (Vec3::new(0.0, 0.0, 1.0) - normal * normal.z).normalized();
                northward * 35.0
            })
            .collect::<Vec<_>>();
        let result = run(
            &mesh,
            &temperature,
            &elevation,
            &wind,
            earthlike_step_on(2_048),
        );
        let without_orography = run(
            &mesh,
            &temperature,
            &elevation,
            &wind,
            MoistureTransportConfig {
                orographic_coefficient_per_meter: 0.0,
                ..earthlike_step_on(2_048)
            },
        );
        let barrier_masked = |values: &[f32]| {
            values
                .iter()
                .enumerate()
                .map(|(cell, &value)| {
                    if (0.15..0.35).contains(&mesh.cell_centers[cell].z) {
                        value
                    } else {
                        0.0
                    }
                })
                .collect::<Vec<_>>()
        };
        let barrier_orographic_mean = mesh.area_weighted_mean(&barrier_masked(
            &result.cell_orographic_precipitation_kg_per_m2_per_day,
        ));
        let barrier_precipitation_mean = mesh.area_weighted_mean(&barrier_masked(
            &result.cell_precipitation_kg_per_m2_per_day,
        ));
        let baseline_precipitation_mean = mesh.area_weighted_mean(&barrier_masked(
            &without_orography.cell_precipitation_kg_per_m2_per_day,
        ));
        assert!(barrier_orographic_mean > 0.0);
        assert!(barrier_precipitation_mean > baseline_precipitation_mean);
        assert!(result.diagnostics.orographic_cell_count > 0);
        assert!(
            result.diagnostics.maximum_orographic_fraction_per_step
                <= earthlike_step_on(2_048).maximum_orographic_fraction_per_step
        );
    }
}
