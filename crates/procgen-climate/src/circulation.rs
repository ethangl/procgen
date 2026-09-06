use crate::AreaWeightedSummary;
use procgen_core::Vec3;
use procgen_planet::{Planet, PlanetValidationError};
use procgen_sphere_mesh::SphereMesh;
use std::{f64::consts::TAU, fmt, ops::RangeInclusive};

pub const DRAG_RATE_RANGE: RangeInclusive<f64> = 1.0e-8..=1.0;
pub const MAXIMUM_WIND_SPEED_RANGE: RangeInclusive<f64> = 0.0..=10_000.0;
pub const TERRAIN_STEERING_RANGE: RangeInclusive<f64> = 0.0..=1.0;
pub const CALM_WIND_SPEED_METERS_PER_SECOND: f32 = 1.0e-6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtmosphericCirculationConfig {
    /// Linear near-surface momentum damping rate in s^-1.
    pub surface_drag_per_second: f64,
    /// Fraction of the upslope wind component removed, in [0, 1].
    pub terrain_steering: f64,
    /// Numerical and modeling safety cap applied after drag, Coriolis, and
    /// terrain steering have been evaluated.
    pub maximum_wind_speed_meters_per_second: f64,
}

impl AtmosphericCirculationConfig {
    /// Convenient Earth-like values. The solver itself contains no
    /// latitude-band or planet-specific constants.
    pub const EARTHLIKE: Self = Self {
        surface_drag_per_second: 1.5e-5,
        terrain_steering: 0.65,
        maximum_wind_speed_meters_per_second: 100.0,
    };
}

#[derive(Clone, Copy, Debug)]
pub struct AtmosphericCirculationInputs<'a> {
    pub planet: Planet,
    pub selected_temperature_kelvin: &'a [f32],
    pub final_elevation: &'a [f32],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtmosphericCirculationDiagnostics {
    pub wind_speed_meters_per_second: AreaWeightedSummary,
    pub temperature_gradient_kelvin_per_radian: AreaWeightedSummary,
    pub pressure_gradient_acceleration_meters_per_second_squared: AreaWeightedSummary,
    pub coriolis_parameter_per_second: AreaWeightedSummary,
    pub terrain_steering_fraction: AreaWeightedSummary,
    pub calm_cell_count: usize,
    pub terrain_steered_cell_count: usize,
    pub speed_capped_cell_count: usize,
    pub maximum_tangency_error_meters_per_second: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AtmosphericCirculation {
    pub cell_wind_meters_per_second: Vec<Vec3>,
    pub cell_wind_speed_meters_per_second: Vec<f32>,
    pub cell_temperature_gradient_kelvin_per_radian: Vec<f32>,
    pub cell_pressure_gradient_acceleration_meters_per_second_squared: Vec<f32>,
    pub cell_coriolis_parameter_per_second: Vec<f32>,
    pub cell_terrain_steering_fraction: Vec<f32>,
    pub diagnostics: AtmosphericCirculationDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtmosphericCirculationError {
    Planet(PlanetValidationError),
    TemperatureCells,
    ElevationCells,
    Temperature,
    Elevation,
    SurfaceDrag,
    TerrainSteering,
    MaximumWindSpeed,
    NumericalRange,
}

impl fmt::Display for AtmosphericCirculationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Planet(error) => error.fmt(formatter),
            Self::TemperatureCells => formatter.write_str("temperature must match the mesh cells"),
            Self::ElevationCells => formatter.write_str("elevation must match the mesh cells"),
            Self::Temperature => formatter.write_str("temperature must be finite and nonnegative"),
            Self::Elevation => formatter.write_str("elevation must contain only finite values"),
            Self::SurfaceDrag => write!(
                formatter,
                "surface drag must be finite and between {} and {} s^-1",
                DRAG_RATE_RANGE.start(),
                DRAG_RATE_RANGE.end()
            ),
            Self::TerrainSteering => {
                formatter.write_str("terrain steering must be finite and in [0, 1]")
            }
            Self::MaximumWindSpeed => write!(
                formatter,
                "maximum wind speed must be finite and between {} and {} m/s",
                MAXIMUM_WIND_SPEED_RANGE.start(),
                MAXIMUM_WIND_SPEED_RANGE.end()
            ),
            Self::NumericalRange => formatter
                .write_str("atmospheric circulation is outside the finite f32 output range"),
        }
    }
}

impl std::error::Error for AtmosphericCirculationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Planet(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PlanetValidationError> for AtmosphericCirculationError {
    fn from(error: PlanetValidationError) -> Self {
        Self::Planet(error)
    }
}

#[derive(Clone, Copy)]
struct CellPhysics {
    pressure_acceleration_scale: f32,
    rotation_rate: f64,
    surface_drag_per_second: f64,
    terrain_steering: f32,
    maximum_wind_speed_meters_per_second: f32,
}

#[derive(Clone, Copy)]
struct CellCirculation {
    wind: Vec3,
    speed: f32,
    temperature_gradient: f32,
    pressure_acceleration: f32,
    coriolis: f32,
    terrain_steering_fraction: f32,
    terrain_steered: bool,
    speed_capped: bool,
    tangency_error: f32,
}

impl CellCirculation {
    fn is_finite(self) -> bool {
        self.wind.x.is_finite()
            && self.wind.y.is_finite()
            && self.wind.z.is_finite()
            && self.speed.is_finite()
            && self.temperature_gradient.is_finite()
            && self.pressure_acceleration.is_finite()
            && self.coriolis.is_finite()
            && self.terrain_steering_fraction.is_finite()
            && self.tangency_error.is_finite()
    }
}

struct CirculationAccumulator {
    winds: Vec<Vec3>,
    speeds: Vec<f32>,
    temperature_gradients: Vec<f32>,
    pressure_accelerations: Vec<f32>,
    coriolis_parameters: Vec<f32>,
    terrain_steering_fractions: Vec<f32>,
    terrain_steered_cell_count: usize,
    speed_capped_cell_count: usize,
    maximum_tangency_error: f32,
}

impl CirculationAccumulator {
    fn new(cell_count: usize) -> Self {
        Self {
            winds: Vec::with_capacity(cell_count),
            speeds: Vec::with_capacity(cell_count),
            temperature_gradients: Vec::with_capacity(cell_count),
            pressure_accelerations: Vec::with_capacity(cell_count),
            coriolis_parameters: Vec::with_capacity(cell_count),
            terrain_steering_fractions: Vec::with_capacity(cell_count),
            terrain_steered_cell_count: 0,
            speed_capped_cell_count: 0,
            maximum_tangency_error: 0.0,
        }
    }

    fn push(&mut self, cell: CellCirculation) -> Result<(), AtmosphericCirculationError> {
        if !cell.is_finite() {
            return Err(AtmosphericCirculationError::NumericalRange);
        }
        self.winds.push(cell.wind);
        self.speeds.push(cell.speed);
        self.temperature_gradients.push(cell.temperature_gradient);
        self.pressure_accelerations.push(cell.pressure_acceleration);
        self.coriolis_parameters.push(cell.coriolis);
        self.terrain_steering_fractions
            .push(cell.terrain_steering_fraction);
        self.terrain_steered_cell_count += usize::from(cell.terrain_steered);
        self.speed_capped_cell_count += usize::from(cell.speed_capped);
        self.maximum_tangency_error = self.maximum_tangency_error.max(cell.tangency_error);
        Ok(())
    }

    fn finish(self, mesh: &SphereMesh) -> AtmosphericCirculation {
        let diagnostics = AtmosphericCirculationDiagnostics {
            wind_speed_meters_per_second: AreaWeightedSummary::from_field(mesh, &self.speeds),
            temperature_gradient_kelvin_per_radian: AreaWeightedSummary::from_field(
                mesh,
                &self.temperature_gradients,
            ),
            pressure_gradient_acceleration_meters_per_second_squared:
                AreaWeightedSummary::from_field(mesh, &self.pressure_accelerations),
            coriolis_parameter_per_second: AreaWeightedSummary::from_field(
                mesh,
                &self.coriolis_parameters,
            ),
            terrain_steering_fraction: AreaWeightedSummary::from_field(
                mesh,
                &self.terrain_steering_fractions,
            ),
            calm_cell_count: self
                .speeds
                .iter()
                .filter(|&&speed| speed <= CALM_WIND_SPEED_METERS_PER_SECOND)
                .count(),
            terrain_steered_cell_count: self.terrain_steered_cell_count,
            speed_capped_cell_count: self.speed_capped_cell_count,
            maximum_tangency_error_meters_per_second: self.maximum_tangency_error,
        };
        AtmosphericCirculation {
            cell_wind_meters_per_second: self.winds,
            cell_wind_speed_meters_per_second: self.speeds,
            cell_temperature_gradient_kelvin_per_radian: self.temperature_gradients,
            cell_pressure_gradient_acceleration_meters_per_second_squared: self
                .pressure_accelerations,
            cell_coriolis_parameter_per_second: self.coriolis_parameters,
            cell_terrain_steering_fraction: self.terrain_steering_fractions,
            diagnostics,
        }
    }
}

pub fn derive_atmospheric_circulation(
    mesh: &SphereMesh,
    inputs: AtmosphericCirculationInputs<'_>,
    config: AtmosphericCirculationConfig,
) -> Result<AtmosphericCirculation, AtmosphericCirculationError> {
    validate(mesh, inputs, config)?;

    let rotation_period = inputs.planet.sidereal_rotation_period_seconds;
    let rotation_rate = if rotation_period == 0.0 {
        0.0
    } else {
        TAU / rotation_period
    };
    let physics = CellPhysics {
        pressure_acceleration_scale: (inputs
            .planet
            .atmospheric_specific_gas_constant_joules_per_kilogram_kelvin
            / inputs.planet.radius_meters) as f32,
        rotation_rate,
        surface_drag_per_second: config.surface_drag_per_second,
        terrain_steering: config.terrain_steering as f32,
        maximum_wind_speed_meters_per_second: config.maximum_wind_speed_meters_per_second as f32,
    };
    let temperature_gradients = mesh.cell_gradients(inputs.selected_temperature_kelvin);
    let elevation_gradients = mesh.cell_gradients(inputs.final_elevation);
    let mut accumulator = CirculationAccumulator::new(mesh.cell_count());

    for cell in 0..mesh.cell_count() {
        accumulator.push(solve_cell(
            mesh.cell_centers[cell].normalized(),
            temperature_gradients[cell],
            elevation_gradients[cell],
            physics,
        ))?;
    }
    Ok(accumulator.finish(mesh))
}

fn solve_cell(
    normal: Vec3,
    temperature_gradient: Vec3,
    elevation_gradient: Vec3,
    physics: CellPhysics,
) -> CellCirculation {
    let temperature_gradient_magnitude = temperature_gradient.length();
    let acceleration = temperature_gradient * physics.pressure_acceleration_scale;
    let coriolis = 2.0 * physics.rotation_rate * f64::from(normal.y);
    let drag = physics.surface_drag_per_second;
    let denominator = drag * drag + coriolis * coriolis;
    let mut wind = acceleration * (drag / denominator) as f32
        - normal.cross(acceleration) * (coriolis / denominator) as f32;

    let uphill = elevation_gradient.normalized();
    let upslope = wind.dot(uphill).max(0.0);
    let removed_speed = upslope * physics.terrain_steering;
    wind = wind - uphill * removed_speed;
    let terrain_steered = removed_speed > 0.0;
    let terrain_steering_fraction = if terrain_steered {
        physics.terrain_steering
    } else {
        0.0
    };

    let steered_speed = wind.length();
    let balance_speed_capped = steered_speed > physics.maximum_wind_speed_meters_per_second;
    if balance_speed_capped {
        wind = wind * (physics.maximum_wind_speed_meters_per_second / steered_speed);
    }
    wind = wind - normal * wind.dot(normal);
    let projected_speed = wind.length();
    let projection_speed_capped = projected_speed > physics.maximum_wind_speed_meters_per_second;
    if projection_speed_capped {
        wind = wind * (physics.maximum_wind_speed_meters_per_second / projected_speed);
    }
    let speed = projected_speed.min(physics.maximum_wind_speed_meters_per_second);

    CellCirculation {
        wind,
        speed,
        temperature_gradient: temperature_gradient_magnitude,
        pressure_acceleration: acceleration.length(),
        coriolis: coriolis as f32,
        terrain_steering_fraction,
        terrain_steered,
        speed_capped: balance_speed_capped || projection_speed_capped,
        tangency_error: wind.dot(normal).abs(),
    }
}

fn validate(
    mesh: &SphereMesh,
    inputs: AtmosphericCirculationInputs<'_>,
    config: AtmosphericCirculationConfig,
) -> Result<(), AtmosphericCirculationError> {
    inputs.planet.validate()?;
    if inputs.selected_temperature_kelvin.len() != mesh.cell_count() {
        return Err(AtmosphericCirculationError::TemperatureCells);
    }
    if inputs.final_elevation.len() != mesh.cell_count() {
        return Err(AtmosphericCirculationError::ElevationCells);
    }
    if inputs
        .selected_temperature_kelvin
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(AtmosphericCirculationError::Temperature);
    }
    if inputs
        .final_elevation
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(AtmosphericCirculationError::Elevation);
    }
    if !config.surface_drag_per_second.is_finite()
        || !DRAG_RATE_RANGE.contains(&config.surface_drag_per_second)
    {
        return Err(AtmosphericCirculationError::SurfaceDrag);
    }
    if !config.terrain_steering.is_finite()
        || !TERRAIN_STEERING_RANGE.contains(&config.terrain_steering)
    {
        return Err(AtmosphericCirculationError::TerrainSteering);
    }
    if !config.maximum_wind_speed_meters_per_second.is_finite()
        || !MAXIMUM_WIND_SPEED_RANGE.contains(&config.maximum_wind_speed_meters_per_second)
    {
        return Err(AtmosphericCirculationError::MaximumWindSpeed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_core::fingerprint;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::SphericalDelaunay;

    fn mesh(count: usize) -> SphereMesh {
        let points = fibonacci_sphere(FibonacciConfig::new(count)).unwrap();
        let delaunay = SphericalDelaunay::build(points).unwrap();
        SphereMesh::from_delaunay(&delaunay, 1.0).unwrap()
    }

    fn inputs<'a>(
        temperature: &'a [f32],
        elevation: &'a [f32],
    ) -> AtmosphericCirculationInputs<'a> {
        AtmosphericCirculationInputs {
            planet: Planet::EARTH,
            selected_temperature_kelvin: temperature,
            final_elevation: elevation,
        }
    }

    fn config() -> AtmosphericCirculationConfig {
        AtmosphericCirculationConfig::EARTHLIKE
    }

    #[test]
    fn circulation_is_deterministic() {
        let mesh = mesh(512);
        let temperature = mesh
            .cell_centers
            .iter()
            .map(|point| 280.0 + 35.0 * point.y)
            .collect::<Vec<_>>();
        let elevation = mesh
            .cell_centers
            .iter()
            .map(|point| 0.5 + 0.2 * point.x)
            .collect::<Vec<_>>();
        let first =
            derive_atmospheric_circulation(&mesh, inputs(&temperature, &elevation), config())
                .unwrap();
        let second =
            derive_atmospheric_circulation(&mesh, inputs(&temperature, &elevation), config())
                .unwrap();
        assert_eq!(first, second);
        let hash = fingerprint(first.cell_wind_meters_per_second.iter().flat_map(|wind| {
            [
                u64::from(wind.x.to_bits()),
                u64::from(wind.y.to_bits()),
                u64::from(wind.z.to_bits()),
            ]
        }));
        assert_eq!(hash, 6_107_040_015_668_640_167);
    }

    #[test]
    fn uniform_temperature_produces_calm_air() {
        let mesh = mesh(128);
        let temperature = vec![280.0; mesh.cell_count()];
        let elevation = vec![0.5; mesh.cell_count()];
        let result =
            derive_atmospheric_circulation(&mesh, inputs(&temperature, &elevation), config())
                .unwrap();
        assert!(
            result
                .cell_wind_meters_per_second
                .iter()
                .all(|wind| *wind == Vec3::ZERO)
        );
        assert_eq!(result.diagnostics.calm_cell_count, mesh.cell_count());
    }

    #[test]
    fn zero_rotation_follows_the_temperature_gradient_without_deflection() {
        let mesh = mesh(512);
        let temperature = mesh
            .cell_centers
            .iter()
            .map(|point| 280.0 + 20.0 * point.y)
            .collect::<Vec<_>>();
        let elevation = vec![0.0; mesh.cell_count()];
        let mut input = inputs(&temperature, &elevation);
        input.planet.sidereal_rotation_period_seconds = 0.0;
        let result = derive_atmospheric_circulation(&mesh, input, config()).unwrap();
        let gradients = mesh.cell_gradients(&temperature);
        for (cell, wind) in result.cell_wind_meters_per_second.iter().enumerate() {
            let gradient = gradients[cell];
            assert!(wind.cross(gradient).length() <= 1.0e-3 * wind.length().max(1.0));
            assert!(wind.dot(gradient) >= -1.0e-6);
            assert_eq!(result.cell_coriolis_parameter_per_second[cell], 0.0);
        }
    }

    #[test]
    fn hemispheres_deflect_an_axisymmetric_gradient_with_matching_zonal_signs() {
        let mesh = mesh(2_048);
        let temperature = mesh
            .cell_centers
            .iter()
            .map(|point| 310.0 - 55.0 * point.y.abs())
            .collect::<Vec<_>>();
        let elevation = vec![0.0; mesh.cell_count()];
        let result =
            derive_atmospheric_circulation(&mesh, inputs(&temperature, &elevation), config())
                .unwrap();
        let mut north = 0.0;
        let mut south = 0.0;
        let mut north_count = 0;
        let mut south_count = 0;
        for (cell, &wind) in result.cell_wind_meters_per_second.iter().enumerate() {
            let normal = mesh.cell_centers[cell].normalized();
            let east = Vec3::new(0.0, 1.0, 0.0).cross(normal).normalized();
            if normal.y > 0.2 {
                north += wind.dot(east);
                north_count += 1;
            }
            if normal.y < -0.2 {
                south += wind.dot(east);
                south_count += 1;
            }
        }
        let north = north / north_count as f32;
        let south = south / south_count as f32;
        assert!(north * south > 0.0);
        assert!((north.abs() - south.abs()).abs() <= 0.08 * north.abs().max(south.abs()));
    }

    #[test]
    fn planet_validation_is_chained_through_one_error_variant() {
        let mesh = mesh(32);
        let temperature = vec![280.0; mesh.cell_count()];
        let elevation = vec![0.0; mesh.cell_count()];
        let mut input = inputs(&temperature, &elevation);
        input.planet.radius_meters = 0.0;
        let error = derive_atmospheric_circulation(&mesh, input, config()).unwrap_err();
        assert_eq!(
            error,
            AtmosphericCirculationError::Planet(PlanetValidationError::Radius)
        );
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn coriolis_is_zero_at_the_equator_and_extreme_at_the_poles() {
        let mesh = mesh(2_048);
        let temperature = mesh
            .cell_centers
            .iter()
            .map(|point| 290.0 + 20.0 * point.x)
            .collect::<Vec<_>>();
        let elevation = vec![0.0; mesh.cell_count()];
        let result =
            derive_atmospheric_circulation(&mesh, inputs(&temperature, &elevation), config())
                .unwrap();
        let equator = (0..mesh.cell_count())
            .min_by(|&a, &b| {
                mesh.cell_centers[a]
                    .y
                    .abs()
                    .total_cmp(&mesh.cell_centers[b].y.abs())
            })
            .unwrap();
        let pole = (0..mesh.cell_count())
            .max_by(|&a, &b| {
                mesh.cell_centers[a]
                    .y
                    .abs()
                    .total_cmp(&mesh.cell_centers[b].y.abs())
            })
            .unwrap();
        assert!(result.cell_coriolis_parameter_per_second[equator].abs() < 1.0e-7);
        assert!(result.cell_coriolis_parameter_per_second[pole].abs() > 1.4e-4);
        assert!(
            result.cell_wind_meters_per_second[equator]
                .length()
                .is_finite()
        );
        assert!(
            result.cell_wind_meters_per_second[pole]
                .length()
                .is_finite()
        );
    }

    #[test]
    fn every_wind_vector_is_tangent() {
        let mesh = mesh(512);
        let temperature = mesh
            .cell_centers
            .iter()
            .map(|point| 280.0 + 30.0 * point.x - 15.0 * point.z)
            .collect::<Vec<_>>();
        let elevation = mesh
            .cell_centers
            .iter()
            .map(|point| point.y)
            .collect::<Vec<_>>();
        let result =
            derive_atmospheric_circulation(&mesh, inputs(&temperature, &elevation), config())
                .unwrap();
        for (cell, wind) in result.cell_wind_meters_per_second.iter().enumerate() {
            assert!(wind.dot(mesh.cell_centers[cell].normalized()).abs() <= 2.0e-5);
        }
    }

    #[test]
    fn terrain_steering_only_reduces_the_upslope_component() {
        let mesh = mesh(512);
        let temperature = mesh
            .cell_centers
            .iter()
            .map(|point| 280.0 + 30.0 * point.x)
            .collect::<Vec<_>>();
        let elevation = temperature.clone();
        let mut input = inputs(&temperature, &elevation);
        input.planet.sidereal_rotation_period_seconds = 0.0;
        let unsteered = derive_atmospheric_circulation(
            &mesh,
            input,
            AtmosphericCirculationConfig {
                terrain_steering: 0.0,
                ..config()
            },
        )
        .unwrap();
        let steered = derive_atmospheric_circulation(
            &mesh,
            input,
            AtmosphericCirculationConfig {
                terrain_steering: 1.0,
                ..config()
            },
        )
        .unwrap();
        assert!(
            steered
                .cell_wind_speed_meters_per_second
                .iter()
                .zip(&unsteered.cell_wind_speed_meters_per_second)
                .all(|(steered, unsteered)| steered <= unsteered)
        );
        assert!(steered.diagnostics.terrain_steered_cell_count > 0);
        assert!(
            steered
                .cell_wind_speed_meters_per_second
                .iter()
                .all(|speed| *speed <= 1.0e-3)
        );
    }

    #[test]
    fn extreme_valid_parameters_remain_finite_and_bounded() {
        let mesh = mesh(128);
        let temperature = mesh
            .cell_centers
            .iter()
            .map(|point| if point.x >= 0.0 { 10_000.0 } else { 0.0 })
            .collect::<Vec<_>>();
        let elevation = mesh
            .cell_centers
            .iter()
            .map(|point| if point.z >= 0.0 { 1.0e20 } else { -1.0e20 })
            .collect::<Vec<_>>();
        let mut input = inputs(&temperature, &elevation);
        input.planet.radius_meters = *procgen_planet::PLANET_RADIUS_METERS_RANGE.start();
        input.planet.sidereal_rotation_period_seconds = 1.0;
        input
            .planet
            .atmospheric_specific_gas_constant_joules_per_kilogram_kelvin =
            *procgen_planet::ATMOSPHERIC_SPECIFIC_GAS_CONSTANT_RANGE.end();
        let extreme = AtmosphericCirculationConfig {
            surface_drag_per_second: *DRAG_RATE_RANGE.start(),
            terrain_steering: *TERRAIN_STEERING_RANGE.end(),
            maximum_wind_speed_meters_per_second: *MAXIMUM_WIND_SPEED_RANGE.end(),
        };
        let result = derive_atmospheric_circulation(&mesh, input, extreme).unwrap();
        assert!(
            result
                .cell_wind_meters_per_second
                .iter()
                .all(|wind| wind.x.is_finite()
                    && wind.y.is_finite()
                    && wind.z.is_finite()
                    && wind.length()
                        <= extreme.maximum_wind_speed_meters_per_second as f32 + 1.0e-3)
        );
        assert!(
            result
                .cell_terrain_steering_fraction
                .iter()
                .all(|value| (0.0..=1.0).contains(value))
        );
    }
}
