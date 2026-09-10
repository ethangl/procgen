use crate::{FieldSummary, SeafloorAge, StageInputError};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaseElevationConfig {
    pub continental_base: f32,
    /// Elevation of age-zero oceanic crust at a ridge.
    pub ridge_elevation: f32,
    /// Minimum elevation reached by sufficiently old oceanic crust.
    pub deep_ocean_elevation: f32,
    /// Seafloor age in evolution steps at which oceanic crust reaches the
    /// deep-ocean floor. Age spans one step, for crust born at a ridge during
    /// the run, to the prior's hop age plus the whole run for crust that
    /// predates it, so this belongs near the top of that span: below it the
    /// whole ocean sits on the deep floor and only crust made during the run
    /// carries any gradient.
    pub cooling_age: usize,
}

impl Default for BaseElevationConfig {
    fn default() -> Self {
        Self {
            continental_base: 0.65,
            ridge_elevation: 0.30,
            deep_ocean_elevation: 0.08,
            cooling_age: 40,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BaseElevationDiagnostics {
    pub summary: FieldSummary,
    pub oceanic: FieldSummary,
    pub oceanic_cell_count: usize,
    pub continental_cell_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BaseElevation {
    pub cell_elevations: Vec<f32>,
    pub diagnostics: BaseElevationDiagnostics,
}

impl BaseElevation {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), StageInputError> {
        if self.cell_elevations.len() != mesh.cell_count() {
            return Err(StageInputError::BaseElevation);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseElevationError {
    InvalidConfig,
}

impl fmt::Display for BaseElevationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str(
                "base elevations must be finite and between 0 and 1, ridge elevation must not be below deep-ocean elevation, and cooling age must be positive",
            ),
        }
    }
}

impl std::error::Error for BaseElevationError {}

/// Derives normalized per-cell base elevation from the independent final
/// seafloor-age field.
///
/// `None` retains the configured continental base. `Some(age)` follows a
/// square-root cooling curve from ridge elevation to deep-ocean elevation.
/// Ages at or above `cooling_age` use the deep floor. Crust that predates the
/// run carries the prior's age plus the steps the run took, so an old ocean
/// reaches the deep floor and crust born at a ridge during the run does not.
pub fn derive_base_elevation(
    seafloor_age: &SeafloorAge,
    config: BaseElevationConfig,
) -> Result<BaseElevation, BaseElevationError> {
    validate_config(config)?;

    let cell_elevations: Vec<_> = seafloor_age
        .cell_ages
        .iter()
        .map(|age| match *age {
            None => config.continental_base,
            Some(age) if age >= config.cooling_age => config.deep_ocean_elevation,
            Some(age) => {
                let progress = (age as f32 / config.cooling_age as f32).sqrt();
                config.ridge_elevation
                    + (config.deep_ocean_elevation - config.ridge_elevation) * progress
            }
        })
        .collect();
    let oceanic_elevations: Vec<_> = seafloor_age
        .cell_ages
        .iter()
        .zip(&cell_elevations)
        .filter_map(|(age, &elevation)| age.map(|_| elevation))
        .collect();
    let oceanic_cell_count = oceanic_elevations.len();
    let diagnostics = BaseElevationDiagnostics {
        summary: FieldSummary::from_values(&cell_elevations),
        oceanic: FieldSummary::from_values(&oceanic_elevations),
        oceanic_cell_count,
        continental_cell_count: cell_elevations.len() - oceanic_cell_count,
    };
    Ok(BaseElevation {
        cell_elevations,
        diagnostics,
    })
}

fn validate_config(config: BaseElevationConfig) -> Result<(), BaseElevationError> {
    let elevations = [
        config.continental_base,
        config.ridge_elevation,
        config.deep_ocean_elevation,
    ];
    if elevations
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        || config.ridge_elevation < config.deep_ocean_elevation
        || config.cooling_age == 0
    {
        return Err(BaseElevationError::InvalidConfig);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{final_state_fixture, fingerprint, reference_evolution_config};
    use crate::{SeafloorAgeDiagnostics, derive_seafloor_age};

    #[test]
    fn base_elevation_is_deterministic_and_has_a_stable_fingerprint() {
        let (mesh, _, evolution) = final_state_fixture();
        let age = derive_seafloor_age(&mesh, &evolution, reference_evolution_config().step_count)
            .unwrap();
        let config = BaseElevationConfig::default();
        let first = derive_base_elevation(&age, config).unwrap();

        assert_eq!(first, derive_base_elevation(&age, config).unwrap());
        assert_eq!(
            first.diagnostics.oceanic_cell_count + first.diagnostics.continental_cell_count,
            age.cell_ages.len()
        );
        assert_eq!(
            fingerprint(
                first
                    .cell_elevations
                    .iter()
                    .map(|value| value.to_bits() as u64)
            ),
            15_914_748_563_604_684_315
        );
    }

    #[test]
    fn age_option_selects_continental_base_or_oceanic_cooling_curve() {
        let age = SeafloorAge {
            cell_ages: vec![None, Some(0), Some(2), Some(8), Some(20)],
            diagnostics: SeafloorAgeDiagnostics::default(),
        };
        let config = BaseElevationConfig {
            continental_base: 0.7,
            ridge_elevation: 0.3,
            deep_ocean_elevation: 0.1,
            cooling_age: 8,
        };

        let base = derive_base_elevation(&age, config).unwrap();
        assert_eq!(base.cell_elevations[0], config.continental_base);
        assert_eq!(base.cell_elevations[1], config.ridge_elevation);
        assert!((base.cell_elevations[2] - 0.2).abs() < f32::EPSILON);
        assert_eq!(base.cell_elevations[3], config.deep_ocean_elevation);
        assert_eq!(base.cell_elevations[4], config.deep_ocean_elevation);
        assert_eq!(base.diagnostics.oceanic_cell_count, 4);
        assert_eq!(base.diagnostics.continental_cell_count, 1);
    }

    #[test]
    fn crust_older_than_the_cooling_age_sits_on_the_deep_floor() {
        let age = SeafloorAge {
            cell_ages: vec![Some(13); 32],
            diagnostics: SeafloorAgeDiagnostics::default(),
        };
        let config = BaseElevationConfig {
            cooling_age: 8,
            ..Default::default()
        };
        let base = derive_base_elevation(&age, config).unwrap();

        assert!(
            base.cell_elevations
                .iter()
                .all(|&value| value == config.deep_ocean_elevation)
        );
        assert_eq!(base.diagnostics.oceanic_cell_count, 32);
    }

    #[test]
    fn rejects_invalid_configuration() {
        let age = SeafloorAge {
            cell_ages: vec![None, Some(0)],
            diagnostics: SeafloorAgeDiagnostics::default(),
        };
        assert_eq!(
            derive_base_elevation(
                &age,
                BaseElevationConfig {
                    cooling_age: 0,
                    ..Default::default()
                }
            ),
            Err(BaseElevationError::InvalidConfig)
        );
    }
}
