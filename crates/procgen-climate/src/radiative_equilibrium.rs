use crate::SolarForcing;
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

/// Stefan-Boltzmann constant in SI units.
pub const STEFAN_BOLTZMANN_CONSTANT: f64 = 5.670_374_419e-8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadiativeEquilibriumConfig {
    /// Uniform fraction of incident shortwave radiation reflected to space.
    pub albedo: f64,
    /// Uniform longwave emissivity used by the Stefan-Boltzmann emission law.
    pub emissivity: f64,
}

impl RadiativeEquilibriumConfig {
    /// A convenient preset, not an implicit assumption of the stage.
    pub const EARTHLIKE: Self = Self {
        albedo: 0.3,
        emissivity: 1.0,
    };

    pub const fn new(albedo: f64, emissivity: f64) -> Self {
        Self { albedo, emissivity }
    }
}

impl Default for RadiativeEquilibriumConfig {
    fn default() -> Self {
        Self::EARTHLIKE
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TemperatureSummary {
    pub minimum_kelvin: f32,
    pub maximum_kelvin: f32,
    pub area_weighted_mean_kelvin: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadiativeEquilibriumDiagnostics {
    pub daily: TemperatureSummary,
    pub annual: TemperatureSummary,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RadiativeEquilibriumTemperature {
    pub daily_effective_temperature_kelvin: Vec<f32>,
    pub annual_effective_temperature_kelvin: Vec<f32>,
    pub diagnostics: RadiativeEquilibriumDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadiativeEquilibriumError {
    Albedo,
    Emissivity,
    CellCount {
        expected: usize,
        daily: usize,
        annual: usize,
    },
    Insolation,
    NumericalRange,
}

impl fmt::Display for RadiativeEquilibriumError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Albedo => formatter.write_str("albedo must be finite and between 0 and 1"),
            Self::Emissivity => {
                formatter.write_str("emissivity must be finite, greater than 0, and at most 1")
            }
            Self::CellCount {
                expected,
                daily,
                annual,
            } => write!(
                formatter,
                "solar-forcing field lengths must match the mesh cell count ({expected}); got {daily} daily and {annual} annual values"
            ),
            Self::Insolation => {
                formatter.write_str("solar-forcing fields must contain finite nonnegative values")
            }
            Self::NumericalRange => formatter.write_str(
                "radiative-equilibrium temperature is outside the finite f32 output range",
            ),
        }
    }
}

impl std::error::Error for RadiativeEquilibriumError {}

/// Derives instantaneous radiative equilibrium from each daily-mean forcing
/// value and equilibrium with the annual-mean forcing for each annual value.
pub fn derive_radiative_equilibrium_temperature(
    mesh: &SphereMesh,
    forcing: &SolarForcing,
    config: RadiativeEquilibriumConfig,
) -> Result<RadiativeEquilibriumTemperature, RadiativeEquilibriumError> {
    validate_inputs(mesh, forcing, config)?;
    let radiation_scale = (1.0 - config.albedo) / (config.emissivity * STEFAN_BOLTZMANN_CONSTANT);
    if !radiation_scale.is_finite() {
        return Err(RadiativeEquilibriumError::NumericalRange);
    }

    let daily_effective_temperature_kelvin =
        temperatures(&forcing.daily_mean_insolation, radiation_scale)?;
    let annual_effective_temperature_kelvin =
        temperatures(&forcing.annual_mean_insolation, radiation_scale)?;

    Ok(RadiativeEquilibriumTemperature {
        diagnostics: RadiativeEquilibriumDiagnostics {
            daily: summarize(mesh, &daily_effective_temperature_kelvin),
            annual: summarize(mesh, &annual_effective_temperature_kelvin),
        },
        daily_effective_temperature_kelvin,
        annual_effective_temperature_kelvin,
    })
}

fn validate_inputs(
    mesh: &SphereMesh,
    forcing: &SolarForcing,
    config: RadiativeEquilibriumConfig,
) -> Result<(), RadiativeEquilibriumError> {
    if !config.albedo.is_finite() || !(0.0..=1.0).contains(&config.albedo) {
        return Err(RadiativeEquilibriumError::Albedo);
    }
    if !(config.emissivity.is_finite() && 0.0 < config.emissivity && config.emissivity <= 1.0) {
        return Err(RadiativeEquilibriumError::Emissivity);
    }
    let expected = mesh.cell_count();
    let daily = forcing.daily_mean_insolation.len();
    let annual = forcing.annual_mean_insolation.len();
    if daily != expected || annual != expected {
        return Err(RadiativeEquilibriumError::CellCount {
            expected,
            daily,
            annual,
        });
    }
    if forcing
        .daily_mean_insolation
        .iter()
        .chain(&forcing.annual_mean_insolation)
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(RadiativeEquilibriumError::Insolation);
    }
    Ok(())
}

fn temperatures(
    insolation: &[f32],
    radiation_scale: f64,
) -> Result<Vec<f32>, RadiativeEquilibriumError> {
    insolation
        .iter()
        .map(|&value| {
            let temperature = (f64::from(value) * radiation_scale).sqrt().sqrt();
            if temperature <= f64::from(f32::MAX) {
                Ok(temperature as f32)
            } else {
                Err(RadiativeEquilibriumError::NumericalRange)
            }
        })
        .collect()
}

fn summarize(mesh: &SphereMesh, values: &[f32]) -> TemperatureSummary {
    TemperatureSummary {
        minimum_kelvin: values.iter().copied().fold(f32::INFINITY, f32::min),
        maximum_kelvin: values.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        area_weighted_mean_kelvin: mesh.area_weighted_mean(values),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SolarForcingConfig, derive_solar_forcing};
    use procgen_planet::Planet;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::SphericalDelaunay;

    fn mesh(count: usize) -> SphereMesh {
        let points = fibonacci_sphere(FibonacciConfig::new(count)).unwrap();
        let delaunay = SphericalDelaunay::build(points).unwrap();
        SphereMesh::from_delaunay(&delaunay, 1.0).unwrap()
    }

    fn forcing(mesh: &SphereMesh) -> SolarForcing {
        derive_solar_forcing(mesh, Planet::EARTH, SolarForcingConfig::default()).unwrap()
    }

    #[test]
    fn repeated_derivation_is_exactly_deterministic() {
        let mesh = mesh(256);
        let forcing = forcing(&mesh);
        let first = derive_radiative_equilibrium_temperature(
            &mesh,
            &forcing,
            RadiativeEquilibriumConfig::EARTHLIKE,
        )
        .unwrap();
        let second = derive_radiative_equilibrium_temperature(
            &mesh,
            &forcing,
            RadiativeEquilibriumConfig::EARTHLIKE,
        )
        .unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn validates_parameters_field_lengths_and_values() {
        let mesh = mesh(32);
        let forcing = forcing(&mesh);
        for albedo in [f64::NAN, -0.01, 1.01] {
            assert_eq!(
                derive_radiative_equilibrium_temperature(
                    &mesh,
                    &forcing,
                    RadiativeEquilibriumConfig::new(albedo, 1.0),
                ),
                Err(RadiativeEquilibriumError::Albedo)
            );
        }
        for emissivity in [f64::NAN, 0.0, -0.01, 1.01] {
            assert_eq!(
                derive_radiative_equilibrium_temperature(
                    &mesh,
                    &forcing,
                    RadiativeEquilibriumConfig::new(0.3, emissivity),
                ),
                Err(RadiativeEquilibriumError::Emissivity)
            );
        }

        let mut wrong_length = forcing.clone();
        wrong_length.daily_mean_insolation.pop();
        assert!(matches!(
            derive_radiative_equilibrium_temperature(
                &mesh,
                &wrong_length,
                RadiativeEquilibriumConfig::EARTHLIKE,
            ),
            Err(RadiativeEquilibriumError::CellCount { .. })
        ));

        for value in [f32::NAN, -1.0] {
            let mut invalid_value = forcing.clone();
            invalid_value.annual_mean_insolation[0] = value;
            assert_eq!(
                derive_radiative_equilibrium_temperature(
                    &mesh,
                    &invalid_value,
                    RadiativeEquilibriumConfig::EARTHLIKE,
                ),
                Err(RadiativeEquilibriumError::Insolation)
            );
        }
    }

    #[test]
    fn zero_insolation_is_absolute_zero() {
        let mesh = mesh(32);
        let mut forcing = forcing(&mesh);
        forcing.daily_mean_insolation.fill(0.0);
        forcing.annual_mean_insolation.fill(0.0);

        let temperatures = derive_radiative_equilibrium_temperature(
            &mesh,
            &forcing,
            RadiativeEquilibriumConfig::EARTHLIKE,
        )
        .unwrap();

        assert!(
            temperatures
                .daily_effective_temperature_kelvin
                .iter()
                .all(|&value| value == 0.0)
        );
        assert!(
            temperatures
                .annual_effective_temperature_kelvin
                .iter()
                .all(|&value| value == 0.0)
        );
        assert_eq!(
            temperatures.diagnostics.daily,
            TemperatureSummary::default()
        );
        assert_eq!(
            temperatures.diagnostics.annual,
            TemperatureSummary::default()
        );
    }

    #[test]
    fn temperature_is_monotonic_in_forcing_albedo_and_emissivity() {
        let mesh = mesh(32);
        let mut forcing = forcing(&mesh);
        forcing.daily_mean_insolation.fill(100.0);
        forcing.annual_mean_insolation.fill(400.0);
        let derive =
            |config| derive_radiative_equilibrium_temperature(&mesh, &forcing, config).unwrap();

        let baseline = derive(RadiativeEquilibriumConfig::new(0.3, 1.0));
        let darker = derive(RadiativeEquilibriumConfig::new(0.1, 1.0));
        let lower_emissivity = derive(RadiativeEquilibriumConfig::new(0.3, 0.5));
        assert!(
            baseline.annual_effective_temperature_kelvin[0]
                > baseline.daily_effective_temperature_kelvin[0]
        );
        assert!(
            darker.daily_effective_temperature_kelvin[0]
                > baseline.daily_effective_temperature_kelvin[0]
        );
        assert!(
            lower_emissivity.daily_effective_temperature_kelvin[0]
                > baseline.daily_effective_temperature_kelvin[0]
        );
    }

    #[test]
    fn handles_extreme_valid_parameters_and_rejects_unrepresentable_output() {
        let mesh = mesh(32);
        let forcing = forcing(&mesh);
        let reflective = derive_radiative_equilibrium_temperature(
            &mesh,
            &forcing,
            RadiativeEquilibriumConfig::new(1.0, f64::MIN_POSITIVE),
        )
        .unwrap();
        assert!(
            reflective
                .daily_effective_temperature_kelvin
                .iter()
                .all(|&value| value == 0.0)
        );
        assert!(
            reflective
                .annual_effective_temperature_kelvin
                .iter()
                .all(|&value| value == 0.0)
        );

        assert_eq!(
            derive_radiative_equilibrium_temperature(
                &mesh,
                &forcing,
                RadiativeEquilibriumConfig::new(0.0, f64::MIN_POSITIVE),
            ),
            Err(RadiativeEquilibriumError::NumericalRange)
        );
    }

    #[test]
    fn earthlike_is_an_explicit_preset_with_known_stefan_boltzmann_response() {
        assert_eq!(
            RadiativeEquilibriumConfig::default(),
            RadiativeEquilibriumConfig::EARTHLIKE
        );
        let temperature = (240.0_f64 / STEFAN_BOLTZMANN_CONSTANT).sqrt().sqrt();
        assert!((temperature - 255.0).abs() < 0.1);
    }
}
