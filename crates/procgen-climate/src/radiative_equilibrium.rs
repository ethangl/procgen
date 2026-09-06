use crate::{AreaWeightedSummary, SolarForcing, SolarForcingError, validate_range};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

/// Stefan-Boltzmann constant in SI units.
pub const STEFAN_BOLTZMANN_CONSTANT: f64 = 5.670_374_419e-8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadiativeEquilibriumConfig {
    /// Uniform longwave emissivity used by the Stefan-Boltzmann emission law.
    pub emissivity: f64,
}

impl RadiativeEquilibriumConfig {
    /// A convenient preset, not an implicit assumption of the stage.
    pub const EARTHLIKE: Self = Self { emissivity: 1.0 };
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
            Self::Albedo => formatter
                .write_str("albedo must match mesh cells with finite values between 0 and 1"),
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
    cell_albedo: &[f32],
) -> Result<RadiativeEquilibriumTemperature, RadiativeEquilibriumError> {
    validate_albedo_field(mesh, cell_albedo)?;
    let model = RadiativeEquilibriumModel::new(config.emissivity)?;
    forcing.validate(mesh)?;

    let daily_effective_temperature_kelvin =
        temperatures(&forcing.daily_mean_insolation, cell_albedo, model);
    let annual_effective_temperature_kelvin =
        temperatures(&forcing.annual_mean_insolation, cell_albedo, model);
    let daily = AreaWeightedSummary::from_field(mesh, &daily_effective_temperature_kelvin);
    let annual = AreaWeightedSummary::from_field(mesh, &annual_effective_temperature_kelvin);
    if !daily.is_finite() || !annual.is_finite() {
        return Err(RadiativeEquilibriumError::NumericalRange);
    }

    Ok(RadiativeEquilibriumTemperature {
        diagnostics: RadiativeEquilibriumDiagnostics { daily, annual },
        daily_effective_temperature_kelvin,
        annual_effective_temperature_kelvin,
    })
}

#[derive(Clone, Copy)]
pub(crate) struct RadiativeEquilibriumModel {
    emissivity: f64,
}

impl RadiativeEquilibriumModel {
    pub fn new(emissivity: f64) -> Result<Self, RadiativeEquilibriumError> {
        validate_range(
            emissivity,
            &(0.0..=1.0),
            RadiativeEquilibriumError::Emissivity,
        )?;
        if emissivity == 0.0 {
            return Err(RadiativeEquilibriumError::Emissivity);
        }
        Ok(Self { emissivity })
    }

    pub fn temperature_kelvin(self, insolation: f64, albedo: f64) -> f64 {
        (insolation * (1.0 - albedo) / self.emission_coefficient())
            .sqrt()
            .sqrt()
    }

    pub fn emission_coefficient(self) -> f64 {
        self.emissivity * STEFAN_BOLTZMANN_CONSTANT
    }
}

fn temperatures(
    insolation: &[f32],
    cell_albedo: &[f32],
    model: RadiativeEquilibriumModel,
) -> Vec<f32> {
    insolation
        .iter()
        .zip(cell_albedo)
        .map(|(&value, &albedo)| {
            model.temperature_kelvin(f64::from(value), f64::from(albedo)) as f32
        })
        .collect()
}

pub(crate) fn validate_albedo_field(
    mesh: &SphereMesh,
    cell_albedo: &[f32],
) -> Result<(), RadiativeEquilibriumError> {
    if cell_albedo.len() != mesh.cell_count()
        || cell_albedo
            .iter()
            .any(|&value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return Err(RadiativeEquilibriumError::Albedo);
    }
    Ok(())
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

    fn albedo(mesh: &SphereMesh, value: f32) -> Vec<f32> {
        vec![value; mesh.cell_count()]
    }

    #[test]
    fn repeated_derivation_is_exactly_deterministic() {
        let mesh = mesh(256);
        let forcing = forcing(&mesh);
        let albedo = albedo(&mesh, 0.3);
        let first = derive_radiative_equilibrium_temperature(
            &mesh,
            &forcing,
            RadiativeEquilibriumConfig::EARTHLIKE,
            &albedo,
        )
        .unwrap();
        let second = derive_radiative_equilibrium_temperature(
            &mesh,
            &forcing,
            RadiativeEquilibriumConfig::EARTHLIKE,
            &albedo,
        )
        .unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn validates_parameters_field_lengths_and_values() {
        let mesh = mesh(32);
        let forcing = forcing(&mesh);
        assert_eq!(
            derive_radiative_equilibrium_temperature(
                &mesh,
                &forcing,
                RadiativeEquilibriumConfig::EARTHLIKE,
                &vec![0.3; mesh.cell_count() - 1],
            ),
            Err(RadiativeEquilibriumError::Albedo)
        );
        for value in [f32::NAN, -0.01, 1.01] {
            assert_eq!(
                derive_radiative_equilibrium_temperature(
                    &mesh,
                    &forcing,
                    RadiativeEquilibriumConfig::EARTHLIKE,
                    &albedo(&mesh, value),
                ),
                Err(RadiativeEquilibriumError::Albedo)
            );
        }
        for emissivity in [f64::NAN, 0.0, -0.01, 1.01] {
            assert_eq!(
                derive_radiative_equilibrium_temperature(
                    &mesh,
                    &forcing,
                    RadiativeEquilibriumConfig { emissivity },
                    &albedo(&mesh, 0.3),
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
                &albedo(&mesh, 0.3),
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
                    &albedo(&mesh, 0.3),
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
            &albedo(&mesh, 0.3),
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
        let derive = |config, albedo_value| {
            derive_radiative_equilibrium_temperature(
                &mesh,
                &forcing,
                config,
                &albedo(&mesh, albedo_value),
            )
            .unwrap()
        };

        let baseline = derive(RadiativeEquilibriumConfig { emissivity: 1.0 }, 0.3);
        let darker = derive(RadiativeEquilibriumConfig { emissivity: 1.0 }, 0.1);
        let lower_emissivity = derive(RadiativeEquilibriumConfig { emissivity: 0.5 }, 0.3);
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
                emissivity: f64::MIN_POSITIVE,
            },
            &albedo(&mesh, 1.0),
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
                    emissivity: f64::MIN_POSITIVE,
                },
                &albedo(&mesh, 0.0),
            ),
            Err(RadiativeEquilibriumError::NumericalRange)
        );
    }

    #[test]
    fn earthlike_preset_has_known_stefan_boltzmann_response() {
        assert_eq!(RadiativeEquilibriumConfig::EARTHLIKE.emissivity, 1.0);
        let temperature = (240.0_f64 / STEFAN_BOLTZMANN_CONSTANT).sqrt().sqrt();
        assert!((temperature - 255.0).abs() < 0.1);
    }
}
