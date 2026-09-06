use crate::{AreaWeightedSummary, SolarForcing, SolarForcingError};
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
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadiativeEquilibriumDiagnostics {
    /// Daily effective-temperature statistics in kelvin.
    pub daily: AreaWeightedSummary,
    /// Annual effective-temperature statistics in kelvin.
    pub annual: AreaWeightedSummary,
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
    SolarForcing(SolarForcingError),
    NumericalRange,
}

impl fmt::Display for RadiativeEquilibriumError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Albedo => formatter.write_str("albedo must be finite and between 0 and 1"),
            Self::Emissivity => {
                formatter.write_str("emissivity must be finite, greater than 0, and at most 1")
            }
            Self::SolarForcing(error) => error.fmt(formatter),
            Self::NumericalRange => formatter.write_str(
                "radiative-equilibrium temperature is outside the finite f32 output range",
            ),
        }
    }
}

impl std::error::Error for RadiativeEquilibriumError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SolarForcing(error) => Some(error),
            _ => None,
        }
    }
}

impl From<SolarForcingError> for RadiativeEquilibriumError {
    fn from(error: SolarForcingError) -> Self {
        Self::SolarForcing(error)
    }
}

/// Derives instantaneous radiative equilibrium from each daily-mean forcing
/// value and equilibrium with the annual-mean forcing for each annual value.
pub fn derive_radiative_equilibrium_temperature(
    mesh: &SphereMesh,
    forcing: &SolarForcing,
    config: RadiativeEquilibriumConfig,
) -> Result<RadiativeEquilibriumTemperature, RadiativeEquilibriumError> {
    validate_config(config)?;
    forcing.validate(mesh)?;
    let radiation_scale = (1.0 - config.albedo) / (config.emissivity * STEFAN_BOLTZMANN_CONSTANT);
    if !radiation_scale.is_finite() {
        return Err(RadiativeEquilibriumError::NumericalRange);
    }

    let daily_effective_temperature_kelvin =
        temperatures(&forcing.daily_mean_insolation, radiation_scale);
    let annual_effective_temperature_kelvin =
        temperatures(&forcing.annual_mean_insolation, radiation_scale);
    let daily = AreaWeightedSummary::from_field(mesh, &daily_effective_temperature_kelvin);
    let annual = AreaWeightedSummary::from_field(mesh, &annual_effective_temperature_kelvin);
    let maximum_kelvin = daily.maximum.max(annual.maximum);
    if !maximum_kelvin.is_finite() {
        return Err(RadiativeEquilibriumError::NumericalRange);
    }

    Ok(RadiativeEquilibriumTemperature {
        diagnostics: RadiativeEquilibriumDiagnostics { daily, annual },
        daily_effective_temperature_kelvin,
        annual_effective_temperature_kelvin,
    })
}

pub(crate) fn validate_config(
    config: RadiativeEquilibriumConfig,
) -> Result<(), RadiativeEquilibriumError> {
    if !config.albedo.is_finite() || !(0.0..=1.0).contains(&config.albedo) {
        return Err(RadiativeEquilibriumError::Albedo);
    }
    if !(config.emissivity.is_finite() && 0.0 < config.emissivity && config.emissivity <= 1.0) {
        return Err(RadiativeEquilibriumError::Emissivity);
    }
    Ok(())
}

pub(crate) fn effective_temperature_kelvin(
    insolation: f64,
    config: RadiativeEquilibriumConfig,
) -> f64 {
    let radiation_scale = (1.0 - config.albedo) / (config.emissivity * STEFAN_BOLTZMANN_CONSTANT);
    (insolation * radiation_scale).sqrt().sqrt()
}

fn temperatures(insolation: &[f32], radiation_scale: f64) -> Vec<f32> {
    insolation
        .iter()
        .map(|&value| (f64::from(value) * radiation_scale).sqrt().sqrt() as f32)
        .collect()
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
                    RadiativeEquilibriumConfig {
                        albedo,
                        emissivity: 1.0,
                    },
                ),
                Err(RadiativeEquilibriumError::Albedo)
            );
        }
        for emissivity in [f64::NAN, 0.0, -0.01, 1.01] {
            assert_eq!(
                derive_radiative_equilibrium_temperature(
                    &mesh,
                    &forcing,
                    RadiativeEquilibriumConfig {
                        albedo: 0.3,
                        emissivity,
                    },
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
            Err(RadiativeEquilibriumError::SolarForcing(
                SolarForcingError::Cells
            ))
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
                Err(RadiativeEquilibriumError::SolarForcing(
                    SolarForcingError::Insolation
                ))
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
            AreaWeightedSummary::default()
        );
        assert_eq!(
            temperatures.diagnostics.annual,
            AreaWeightedSummary::default()
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

        let baseline = derive(RadiativeEquilibriumConfig {
            albedo: 0.3,
            emissivity: 1.0,
        });
        let darker = derive(RadiativeEquilibriumConfig {
            albedo: 0.1,
            emissivity: 1.0,
        });
        let lower_emissivity = derive(RadiativeEquilibriumConfig {
            albedo: 0.3,
            emissivity: 0.5,
        });
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
            RadiativeEquilibriumConfig {
                albedo: 1.0,
                emissivity: f64::MIN_POSITIVE,
            },
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
                RadiativeEquilibriumConfig {
                    albedo: 0.0,
                    emissivity: f64::MIN_POSITIVE,
                },
            ),
            Err(RadiativeEquilibriumError::NumericalRange)
        );
    }

    #[test]
    fn earthlike_preset_has_known_stefan_boltzmann_response() {
        assert_eq!(RadiativeEquilibriumConfig::EARTHLIKE.albedo, 0.3);
        assert_eq!(RadiativeEquilibriumConfig::EARTHLIKE.emissivity, 1.0);
        let temperature = (240.0_f64 / STEFAN_BOLTZMANN_CONSTANT).sqrt().sqrt();
        assert!((temperature - 255.0).abs() < 0.1);
    }
}
