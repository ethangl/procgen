//! Deterministic climate forcing stages over the authoritative spherical mesh.
//!
//! This first slice computes top-of-atmosphere daily-mean insolation only. It
//! has no atmospheric, surface, temperature, transport, or feedback model.

use procgen_planet::{Planet, PlanetValidationError};
use procgen_sphere_mesh::SphereMesh;
use std::{f64::consts::PI, fmt, ops::RangeInclusive};

pub const ANNUAL_SAMPLE_RANGE: RangeInclusive<usize> = 4..=4_096;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolarForcingConfig {
    /// Orbits since periapsis, uniform in elapsed orbital time and wrapped to
    /// the unit interval.
    pub orbital_phase: f64,
    /// Midpoint samples, uniform in elapsed orbital time, for the annual mean.
    pub annual_sample_count: usize,
}

impl Default for SolarForcingConfig {
    fn default() -> Self {
        Self {
            orbital_phase: 0.0,
            annual_sample_count: 96,
        }
    }
}

/// Unit-bearing field statistics kept independent of tectonic field types.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InsolationSummary {
    pub minimum_watts_per_square_meter: f32,
    pub maximum_watts_per_square_meter: f32,
    pub area_weighted_mean_watts_per_square_meter: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolarForcingDiagnostics {
    pub orbital_phase: f64,
    pub orbital_distance_meters: f64,
    pub stellar_flux_watts_per_square_meter: f64,
    pub solar_declination_radians: f64,
    pub polar_night_cell_count: usize,
    pub polar_day_cell_count: usize,
    pub daily_mean: InsolationSummary,
    pub annual_mean: InsolationSummary,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SolarForcing {
    pub daily_mean_insolation: Vec<f32>,
    pub annual_mean_insolation: Vec<f32>,
    pub diagnostics: SolarForcingDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolarForcingError {
    Planet(PlanetValidationError),
    OrbitalPhase,
    AnnualSampleCount,
    NumericalRange,
}

impl fmt::Display for SolarForcingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Planet(error) => error.fmt(formatter),
            Self::OrbitalPhase => formatter.write_str("orbital phase must be finite"),
            Self::AnnualSampleCount => write!(
                formatter,
                "annual sample count must be between {} and {}",
                ANNUAL_SAMPLE_RANGE.start(),
                ANNUAL_SAMPLE_RANGE.end()
            ),
            Self::NumericalRange => formatter.write_str(
                "planet inputs produce solar forcing outside the finite f32 output range",
            ),
        }
    }
}

impl std::error::Error for SolarForcingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Planet(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PlanetValidationError> for SolarForcingError {
    fn from(error: PlanetValidationError) -> Self {
        Self::Planet(error)
    }
}

pub fn derive_solar_forcing(
    mesh: &SphereMesh,
    planet: Planet,
    config: SolarForcingConfig,
) -> Result<SolarForcing, SolarForcingError> {
    planet.validate()?;
    validate_config(config)?;
    let periapsis = orbital_state(planet, 0.0);
    if periapsis.stellar_flux > f64::from(f32::MAX) {
        return Err(SolarForcingError::NumericalRange);
    }

    let orbital_phase = config.orbital_phase.rem_euclid(1.0);
    let phase_state = orbital_state(planet, orbital_phase);
    let latitudes = mesh
        .cell_centers
        .iter()
        .map(|center| SinCos::from_sine(f64::from(center.y / mesh.radius)))
        .collect::<Vec<_>>();
    let daily = latitudes
        .iter()
        .map(|&latitude| daily_mean_at_latitude(latitude, phase_state))
        .collect::<Vec<_>>();
    let polar_night_cell_count = daily
        .iter()
        .filter(|value| value.daylight == Daylight::PolarNight)
        .count();
    let polar_day_cell_count = daily
        .iter()
        .filter(|value| value.daylight == Daylight::PolarDay)
        .count();
    let daily_mean_insolation = daily
        .iter()
        .map(|value| value.watts_per_square_meter as f32)
        .collect::<Vec<_>>();

    let mut annual_sums = vec![0.0_f64; mesh.cell_count()];
    for sample in 0..config.annual_sample_count {
        let phase = (sample as f64 + 0.5) / config.annual_sample_count as f64;
        let state = orbital_state(planet, phase);
        for (cell, &latitude) in latitudes.iter().enumerate() {
            annual_sums[cell] += daily_mean_at_latitude(latitude, state).watts_per_square_meter;
        }
    }
    let sample_reciprocal = 1.0 / config.annual_sample_count as f64;
    let annual_mean_insolation = annual_sums
        .into_iter()
        .map(|sum| (sum * sample_reciprocal) as f32)
        .collect::<Vec<_>>();

    Ok(SolarForcing {
        diagnostics: SolarForcingDiagnostics {
            orbital_phase,
            orbital_distance_meters: phase_state.distance_meters,
            stellar_flux_watts_per_square_meter: phase_state.stellar_flux,
            solar_declination_radians: phase_state.declination,
            polar_night_cell_count,
            polar_day_cell_count,
            daily_mean: summarize(mesh, &daily_mean_insolation),
            annual_mean: summarize(mesh, &annual_mean_insolation),
        },
        daily_mean_insolation,
        annual_mean_insolation,
    })
}

fn validate_config(config: SolarForcingConfig) -> Result<(), SolarForcingError> {
    if !config.orbital_phase.is_finite() {
        return Err(SolarForcingError::OrbitalPhase);
    }
    if !ANNUAL_SAMPLE_RANGE.contains(&config.annual_sample_count) {
        return Err(SolarForcingError::AnnualSampleCount);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct OrbitalState {
    distance_meters: f64,
    stellar_flux: f64,
    declination: f64,
    declination_trig: SinCos,
}

fn orbital_state(planet: Planet, phase: f64) -> OrbitalState {
    let orbit = planet.orbit;
    let mean_anomaly = phase * 2.0 * PI;
    let eccentric_anomaly = solve_kepler(mean_anomaly, orbit.eccentricity);
    let true_anomaly = 2.0
        * ((1.0 + orbit.eccentricity).sqrt() * (eccentric_anomaly * 0.5).sin())
            .atan2((1.0 - orbit.eccentricity).sqrt() * (eccentric_anomaly * 0.5).cos());
    let distance_meters =
        orbit.semi_major_axis_meters * (1.0 - orbit.eccentricity * eccentric_anomaly.cos());
    let stellar_longitude = true_anomaly + orbit.stellar_longitude_at_periapsis_radians;
    let declination = (orbit.obliquity_radians.sin() * stellar_longitude.sin()).asin();
    let stellar_flux = planet.star.luminosity_watts / (4.0 * PI * distance_meters.powi(2));
    OrbitalState {
        distance_meters,
        stellar_flux,
        declination,
        declination_trig: SinCos {
            sine: declination.sin(),
            cosine: declination.cos(),
        },
    }
}

fn solve_kepler(mean_anomaly: f64, eccentricity: f64) -> f64 {
    if mean_anomaly == 0.0 || eccentricity == 0.0 {
        return mean_anomaly;
    }
    let mut lower = 0.0;
    let mut upper = 2.0 * PI;
    for _ in 0..64 {
        let midpoint = (lower + upper) * 0.5;
        let residual = midpoint - eccentricity * midpoint.sin() - mean_anomaly;
        if residual > 0.0 {
            upper = midpoint;
        } else {
            lower = midpoint;
        }
    }
    (lower + upper) * 0.5
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Daylight {
    PolarNight,
    Cycles,
    PolarDay,
}

#[derive(Clone, Copy, Debug)]
struct SinCos {
    sine: f64,
    cosine: f64,
}

impl SinCos {
    fn from_sine(sine: f64) -> Self {
        let sine = sine.clamp(-1.0, 1.0);
        Self {
            sine,
            cosine: (1.0 - sine * sine).sqrt(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DailyMeanInsolation {
    watts_per_square_meter: f64,
    daylight: Daylight,
}

#[derive(Clone, Copy, Debug)]
struct DaylightWindow {
    sunset_hour_angle: f64,
    daylight: Daylight,
}

fn daylight_window(meridional: f64, diurnal: f64) -> DaylightWindow {
    // Cosine of the sunset hour angle: infinite when the sun never crosses
    // the horizon, and NaN only at a pole on the equinox.
    let sunset_argument = -meridional / diurnal;
    let (sunset_hour_angle, daylight) = if sunset_argument >= 1.0 {
        (0.0, Daylight::PolarNight)
    } else if sunset_argument <= -1.0 {
        (PI, Daylight::PolarDay)
    } else if sunset_argument.is_nan() {
        (0.0, Daylight::Cycles)
    } else {
        (sunset_argument.acos(), Daylight::Cycles)
    };
    DaylightWindow {
        sunset_hour_angle,
        daylight,
    }
}

fn daily_mean_at_latitude(latitude: SinCos, state: OrbitalState) -> DailyMeanInsolation {
    let meridional = latitude.sine * state.declination_trig.sine;
    let diurnal = latitude.cosine * state.declination_trig.cosine;
    let daylight = daylight_window(meridional, diurnal);
    let insolation = state.stellar_flux / PI
        * (daylight.sunset_hour_angle * meridional + diurnal * daylight.sunset_hour_angle.sin());
    DailyMeanInsolation {
        watts_per_square_meter: insolation.clamp(0.0, state.stellar_flux),
        daylight: daylight.daylight,
    }
}

fn summarize(mesh: &SphereMesh, values: &[f32]) -> InsolationSummary {
    let minimum = values.iter().copied().fold(f32::INFINITY, f32::min);
    let maximum = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    InsolationSummary {
        minimum_watts_per_square_meter: minimum,
        maximum_watts_per_square_meter: maximum,
        area_weighted_mean_watts_per_square_meter: mesh.area_weighted_mean(values),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_planet::{Orbit, Star};
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::SphericalDelaunay;

    fn mesh(count: usize) -> SphereMesh {
        let points = fibonacci_sphere(FibonacciConfig::new(count)).unwrap();
        let delaunay = SphericalDelaunay::build(points).unwrap();
        SphereMesh::from_delaunay(&delaunay, 1.0).unwrap()
    }

    fn circular_planet(obliquity: f64, stellar_longitude: f64) -> Planet {
        Planet {
            star: Star {
                luminosity_watts: 4.0 * PI,
            },
            orbit: Orbit {
                semi_major_axis_meters: 1.0,
                eccentricity: 0.0,
                obliquity_radians: obliquity,
                stellar_longitude_at_periapsis_radians: stellar_longitude,
            },
        }
    }

    #[test]
    fn zero_obliquity_matches_equatorial_daily_mean_and_global_energy() {
        let mesh = mesh(2_048);
        let forcing = derive_solar_forcing(
            &mesh,
            circular_planet(0.0, 0.0),
            SolarForcingConfig::default(),
        )
        .unwrap();
        let equator = mesh
            .cell_centers
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| left.y.abs().total_cmp(&right.y.abs()))
            .unwrap()
            .0;

        assert!((forcing.daily_mean_insolation[equator] - 1.0 / PI as f32).abs() < 5.0e-4);
        assert!(
            (forcing
                .diagnostics
                .daily_mean
                .area_weighted_mean_watts_per_square_meter
                - 0.25)
                .abs()
                < 2.0e-3
        );
        assert_eq!(
            forcing.daily_mean_insolation,
            forcing.annual_mean_insolation
        );
    }

    #[test]
    fn northern_solstice_has_southern_polar_night_and_northern_polar_day() {
        let mesh = mesh(512);
        let forcing = derive_solar_forcing(
            &mesh,
            circular_planet(23.5_f64.to_radians(), PI * 0.5),
            SolarForcingConfig::default(),
        )
        .unwrap();
        let north = mesh
            .cell_centers
            .iter()
            .position(|center| center.y > 0.99)
            .unwrap();
        let south = mesh
            .cell_centers
            .iter()
            .position(|center| center.y < -0.99)
            .unwrap();

        assert!(forcing.daily_mean_insolation[north] > 0.0);
        assert_eq!(forcing.daily_mean_insolation[south], 0.0);
        assert!(forcing.diagnostics.polar_day_cell_count > 0);
        assert!(forcing.diagnostics.polar_night_cell_count > 0);
    }

    #[test]
    fn repeated_derivation_is_exactly_deterministic_and_bounded() {
        let mesh = mesh(256);
        let config = SolarForcingConfig {
            orbital_phase: 0.371,
            annual_sample_count: 37,
        };
        let first = derive_solar_forcing(&mesh, Planet::EARTH, config).unwrap();
        let second = derive_solar_forcing(&mesh, Planet::EARTH, config).unwrap();

        assert_eq!(first, second);
        assert!(
            first
                .daily_mean_insolation
                .iter()
                .all(|value| value.is_finite()
                    && *value >= 0.0
                    && f64::from(*value) <= first.diagnostics.stellar_flux_watts_per_square_meter)
        );
        let periapsis_flux = orbital_state(Planet::EARTH, 0.0).stellar_flux;
        assert!(
            first
                .annual_mean_insolation
                .iter()
                .all(|value| value.is_finite()
                    && *value >= 0.0
                    && f64::from(*value) <= periapsis_flux)
        );
    }

    #[test]
    fn validates_phase_sample_bounds_and_planet() {
        let mesh = mesh(32);
        let invalid_phase = SolarForcingConfig {
            orbital_phase: f64::NAN,
            ..Default::default()
        };
        assert_eq!(
            derive_solar_forcing(&mesh, Planet::EARTH, invalid_phase),
            Err(SolarForcingError::OrbitalPhase)
        );

        let invalid_samples = SolarForcingConfig {
            annual_sample_count: *ANNUAL_SAMPLE_RANGE.start() - 1,
            ..Default::default()
        };
        assert_eq!(
            derive_solar_forcing(&mesh, Planet::EARTH, invalid_samples),
            Err(SolarForcingError::AnnualSampleCount)
        );

        let mut invalid_planet = Planet::EARTH;
        invalid_planet.orbit.semi_major_axis_meters = 0.0;
        assert_eq!(
            derive_solar_forcing(&mesh, invalid_planet, SolarForcingConfig::default()),
            Err(SolarForcingError::Planet(
                PlanetValidationError::SemiMajorAxis
            ))
        );
    }

    #[test]
    fn exact_poles_are_finite_at_equinox_and_classified_at_solstice() {
        let equinox = orbital_state(circular_planet(0.0, 0.0), 0.0);
        assert_eq!(
            daily_mean_at_latitude(SinCos::from_sine(1.0), equinox),
            DailyMeanInsolation {
                watts_per_square_meter: 0.0,
                daylight: Daylight::Cycles,
            }
        );

        let solstice = orbital_state(circular_planet(23.5_f64.to_radians(), PI * 0.5), 0.0);
        assert_eq!(
            daily_mean_at_latitude(SinCos::from_sine(1.0), solstice).daylight,
            Daylight::PolarDay
        );
        assert_eq!(
            daily_mean_at_latitude(SinCos::from_sine(-1.0), solstice),
            DailyMeanInsolation {
                watts_per_square_meter: 0.0,
                daylight: Daylight::PolarNight,
            }
        );
    }

    #[test]
    fn near_parabolic_orbit_resolves_periapsis_without_iteration_drift() {
        let baseline = circular_planet(0.0, 0.0);
        let planet = Planet {
            orbit: Orbit {
                eccentricity: 0.999_999,
                ..baseline.orbit
            },
            ..baseline
        };
        let state = orbital_state(planet, 0.0);
        assert!((state.distance_meters - 1.0e-6).abs() < 1.0e-15);
        assert!(state.stellar_flux.is_finite());
    }

    #[test]
    fn orbital_phase_is_periodic() {
        let mesh = mesh(64);
        let forcing = |orbital_phase| {
            derive_solar_forcing(
                &mesh,
                Planet::EARTH,
                SolarForcingConfig {
                    orbital_phase,
                    ..Default::default()
                },
            )
            .unwrap()
        };
        assert_eq!(forcing(-0.25), forcing(1.75));
    }

    #[test]
    fn rejects_forcing_that_cannot_fit_the_output_field() {
        let mesh = mesh(32);
        let planet = Planet {
            star: Star {
                luminosity_watts: f64::MAX,
            },
            ..Planet::EARTH
        };
        assert_eq!(
            derive_solar_forcing(&mesh, planet, SolarForcingConfig::default()),
            Err(SolarForcingError::NumericalRange)
        );
    }
}
