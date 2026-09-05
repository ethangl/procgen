use crate::{
    CrustClass, CrustClassification, FieldSummary, PlatePartition, SeafloorAge,
    stage::{StageInputError, validate_ownership_and_crust},
};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaseElevationConfig {
    pub continental_base: f32,
    /// Elevation of age-zero oceanic crust at a ridge.
    pub ridge_elevation: f32,
    /// Minimum elevation reached by sufficiently old oceanic crust.
    pub deep_ocean_elevation: f32,
    /// Seafloor hop age at which oceanic crust reaches the deep-ocean floor.
    pub cooling_age: usize,
}

impl Default for BaseElevationConfig {
    fn default() -> Self {
        Self {
            continental_base: 0.65,
            ridge_elevation: 0.30,
            deep_ocean_elevation: 0.08,
            cooling_age: 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BaseElevationDiagnostics {
    pub summary: FieldSummary,
    pub oceanic: FieldSummary,
    pub continental: FieldSummary,
    pub oceanic_cell_count: usize,
    pub continental_cell_count: usize,
    pub deep_ocean_cell_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BaseElevation {
    pub cell_elevations: Vec<f32>,
    pub diagnostics: BaseElevationDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseElevationError {
    InvalidConfig,
    Input(StageInputError),
    AgeCountMismatch,
    MissingOceanicAge,
}

impl fmt::Display for BaseElevationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str(
                "base elevations must be finite and between 0 and 1, ridge elevation must not be below deep-ocean elevation, and cooling age must be positive",
            ),
            Self::Input(error) => error.fmt(formatter),
            Self::AgeCountMismatch => {
                formatter.write_str("seafloor ages must match the mesh cell count")
            }
            Self::MissingOceanicAge => {
                formatter.write_str("every oceanic cell must have a seafloor age")
            }
        }
    }
}

impl std::error::Error for BaseElevationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            Self::InvalidConfig | Self::AgeCountMismatch | Self::MissingOceanicAge => None,
        }
    }
}

impl From<StageInputError> for BaseElevationError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
    }
}

/// Derives normalized per-cell base elevation from current-owner crust and the
/// independent final seafloor-age field.
///
/// Continental cells retain the configured base. Oceanic cells follow a
/// square-root cooling curve from ridge elevation to deep-ocean elevation.
/// Ages at or above `cooling_age` deterministically use the deep floor. The
/// seafloor-age stage supplies a configured fallback age for ridge-less plates.
pub fn derive_base_elevation(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    seafloor_age: &SeafloorAge,
    config: BaseElevationConfig,
) -> Result<BaseElevation, BaseElevationError> {
    validate_config(config)?;
    validate_ownership_and_crust(mesh, partition, crust)?;
    if seafloor_age.cell_ages.len() != mesh.cell_count() {
        return Err(BaseElevationError::AgeCountMismatch);
    }

    let mut oceanic = Vec::new();
    let mut continental = Vec::new();
    let mut deep_ocean_cell_count = 0;
    let mut cell_elevations = Vec::with_capacity(mesh.cell_count());
    for cell in 0..mesh.cell_count() {
        let elevation = match crust.cell_class(partition, cell) {
            CrustClass::Continental => {
                continental.push(config.continental_base);
                config.continental_base
            }
            CrustClass::Oceanic => {
                let age =
                    seafloor_age.cell_ages[cell].ok_or(BaseElevationError::MissingOceanicAge)?;
                deep_ocean_cell_count += usize::from(age >= config.cooling_age);
                let progress = (age as f32 / config.cooling_age as f32).min(1.0).sqrt();
                let elevation = config.ridge_elevation
                    + (config.deep_ocean_elevation - config.ridge_elevation) * progress;
                oceanic.push(elevation);
                elevation
            }
        };
        cell_elevations.push(elevation);
    }

    let diagnostics = BaseElevationDiagnostics {
        summary: FieldSummary::from_values(&cell_elevations),
        oceanic: FieldSummary::from_values(&oceanic),
        continental: FieldSummary::from_values(&continental),
        oceanic_cell_count: oceanic.len(),
        continental_cell_count: continental.len(),
        deep_ocean_cell_count,
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
    use crate::test_support::{
        empty_boundaries, final_state_fixture, fingerprint, reference_partition,
    };
    use crate::{SeafloorAgeConfig, derive_seafloor_age};

    #[test]
    fn base_elevation_is_deterministic_and_has_a_stable_fingerprint() {
        let (mesh, partition, crust, boundaries) = final_state_fixture();
        let age = derive_seafloor_age(
            &mesh,
            &partition,
            &crust,
            &boundaries,
            SeafloorAgeConfig::default(),
        )
        .unwrap();
        let config = BaseElevationConfig::default();
        let first = derive_base_elevation(&mesh, &partition, &crust, &age, config).unwrap();

        assert_eq!(
            first,
            derive_base_elevation(&mesh, &partition, &crust, &age, config).unwrap()
        );
        assert_eq!(
            first.diagnostics.oceanic_cell_count + first.diagnostics.continental_cell_count,
            mesh.cell_count()
        );
        assert_eq!(
            fingerprint(
                first
                    .cell_elevations
                    .iter()
                    .map(|value| value.to_bits() as u64)
            ),
            5_590_931_095_123_595_247
        );
    }

    #[test]
    fn current_owner_crust_selects_continental_base_or_oceanic_cooling_curve() {
        let (mesh, mut partition) = reference_partition();
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Oceanic, CrustClass::Continental]
                .into_iter()
                .cycle()
                .take(partition.plate_count)
                .collect(),
        };
        let oceanic_cell = partition
            .cell_plates
            .iter()
            .position(|&plate| crust.plate_classes[plate] == CrustClass::Oceanic)
            .unwrap();
        let cooled_oceanic_cell = partition
            .cell_plates
            .iter()
            .enumerate()
            .find_map(|(cell, &plate)| {
                (cell != oceanic_cell && crust.plate_classes[plate] == CrustClass::Oceanic)
                    .then_some(cell)
            })
            .unwrap();
        let continental_plate = crust
            .plate_classes
            .iter()
            .position(|&class| class == CrustClass::Continental)
            .unwrap();
        let mut ages = vec![None; mesh.cell_count()];
        for (cell, &plate) in partition.cell_plates.iter().enumerate() {
            if crust.plate_classes[plate] == CrustClass::Oceanic {
                ages[cell] = Some(2);
            }
        }
        ages[oceanic_cell] = Some(0);
        let age = SeafloorAge {
            cell_ages: ages,
            diagnostics: Default::default(),
        };
        let config = BaseElevationConfig {
            continental_base: 0.7,
            ridge_elevation: 0.3,
            deep_ocean_elevation: 0.1,
            cooling_age: 8,
        };

        let original = derive_base_elevation(&mesh, &partition, &crust, &age, config).unwrap();
        assert_eq!(
            original.cell_elevations[oceanic_cell],
            config.ridge_elevation
        );
        assert!((original.cell_elevations[cooled_oceanic_cell] - 0.2).abs() < f32::EPSILON);

        partition.cell_plates[oceanic_cell] = continental_plate;
        let changed = derive_base_elevation(&mesh, &partition, &crust, &age, config).unwrap();
        assert_eq!(
            changed.cell_elevations[oceanic_cell],
            config.continental_base
        );
    }

    #[test]
    fn ridge_less_oceanic_crust_uses_the_configured_age_and_deep_floor() {
        let (mesh, partition) = reference_partition();
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Oceanic; partition.plate_count],
        };
        let age = derive_seafloor_age(
            &mesh,
            &partition,
            &crust,
            &empty_boundaries(&mesh),
            SeafloorAgeConfig { ridge_less_age: 13 },
        )
        .unwrap();
        let config = BaseElevationConfig {
            cooling_age: 8,
            ..Default::default()
        };
        let base = derive_base_elevation(&mesh, &partition, &crust, &age, config).unwrap();

        assert!(
            base.cell_elevations
                .iter()
                .all(|&value| value == config.deep_ocean_elevation)
        );
        assert_eq!(base.diagnostics.deep_ocean_cell_count, mesh.cell_count());
    }

    #[test]
    fn rejects_invalid_configuration_and_malformed_age_fields() {
        let (mesh, partition) = reference_partition();
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Oceanic; partition.plate_count],
        };
        let age = SeafloorAge {
            cell_ages: vec![Some(0); mesh.cell_count()],
            diagnostics: Default::default(),
        };
        assert_eq!(
            derive_base_elevation(
                &mesh,
                &partition,
                &crust,
                &age,
                BaseElevationConfig {
                    cooling_age: 0,
                    ..Default::default()
                }
            ),
            Err(BaseElevationError::InvalidConfig)
        );

        let mut short_age = age.clone();
        short_age.cell_ages.pop();
        assert_eq!(
            derive_base_elevation(
                &mesh,
                &partition,
                &crust,
                &short_age,
                BaseElevationConfig::default()
            ),
            Err(BaseElevationError::AgeCountMismatch)
        );

        let mut missing_age = age;
        missing_age.cell_ages[0] = None;
        assert_eq!(
            derive_base_elevation(
                &mesh,
                &partition,
                &crust,
                &missing_age,
                BaseElevationConfig::default()
            ),
            Err(BaseElevationError::MissingOceanicAge)
        );
    }
}
