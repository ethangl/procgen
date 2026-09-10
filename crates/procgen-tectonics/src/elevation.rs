use crate::{BaseElevation, BoundaryDeformation, FieldSummary, StageInputError};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

/// Classifies one normalized elevation against a sea-level datum, using the
/// pipeline-wide strict boundary: the datum itself is ocean.
pub const fn is_land(elevation: f32, sea_level: f32) -> bool {
    elevation > sea_level
}

/// Converts normalized land elevation to physical meters against a sea-level
/// datum. Ocean and sea-level cells have zero land height.
pub fn land_elevation_meters(
    elevation: f32,
    sea_level: f32,
    maximum_land_elevation_meters: f64,
) -> f64 {
    if is_land(elevation, sea_level) {
        (f64::from(elevation) - f64::from(sea_level)) / (1.0 - f64::from(sea_level))
            * maximum_land_elevation_meters
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoarseElevationConfig {
    pub smoothing_passes: usize,
    pub smoothing_weight: f32,
    /// Normalized elevation of the ocean/land boundary. Raising it floods low
    /// margins and interiors; lowering it exposes them.
    pub sea_level: f32,
}

impl Default for CoarseElevationConfig {
    fn default() -> Self {
        Self {
            smoothing_passes: 2,
            smoothing_weight: 0.2,
            sea_level: 0.5,
        }
    }
}

/// A borrowed normalized elevation field together with the datum it was
/// composed against.
///
/// The datum is part of the field rather than an echo of a knob: an elevation
/// field is not interpretable without knowing where its ocean ends, the same
/// way a [`SphereMesh`] is not interpretable without its radius. Every stage
/// that produces normalized elevation lends one of these, so a reader is
/// handed the datum with the values instead of a slice and a loose `f32`
/// beside it.
#[derive(Clone, Copy, Debug)]
pub struct ElevationField<'a> {
    pub cell_elevations: &'a [f32],
    pub sea_level: f32,
}

impl ElevationField<'_> {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), StageInputError> {
        if self.cell_elevations.len() != mesh.cell_count() {
            return Err(StageInputError::Elevation);
        }
        Ok(())
    }

    pub fn is_land(&self, cell: usize) -> bool {
        is_land(self.cell_elevations[cell], self.sea_level)
    }

    /// Cells standing strictly above the field's own datum.
    pub fn land_cell_count(&self) -> usize {
        (0..self.cell_elevations.len())
            .filter(|&cell| self.is_land(cell))
            .count()
    }
}

/// Normalized coarse elevation, its datum, and the summary of its values.
#[derive(Clone, Debug, PartialEq)]
pub struct CoarseElevation {
    pub cell_elevations: Vec<f32>,
    pub sea_level: f32,
    pub diagnostics: FieldSummary,
}

impl CoarseElevation {
    pub fn field(&self) -> ElevationField<'_> {
        ElevationField {
            cell_elevations: &self.cell_elevations,
            sea_level: self.sea_level,
        }
    }

    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), StageInputError> {
        self.field().validate(mesh)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoarseElevationError {
    InvalidConfig,
    InvalidSeaLevel,
    FieldCountMismatch,
}

impl fmt::Display for CoarseElevationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => {
                formatter.write_str("smoothing weight must be finite and between 0 and 1")
            }
            Self::InvalidSeaLevel => {
                formatter.write_str("sea level must be finite and strictly between 0 and 1")
            }
            Self::FieldCountMismatch => formatter
                .write_str("base elevation and deformation values must match the mesh cell count"),
        }
    }
}

impl std::error::Error for CoarseElevationError {}

/// Composes normalized coarse elevation from separately derived base elevation
/// and signed boundary deformation fields.
///
/// Base elevation and deformation are added once, smoothed simultaneously, and
/// clamped. Ownership evolution, seafloor age, and boundary derivation remain
/// separate stages with no elevation history or inter-step state. The
/// configured sea level does not enter the arithmetic; it travels with the
/// composed field so its readers agree on where its ocean ends.
pub fn compose_coarse_elevation(
    mesh: &SphereMesh,
    base_elevation: &BaseElevation,
    deformation: &BoundaryDeformation,
    config: CoarseElevationConfig,
) -> Result<CoarseElevation, CoarseElevationError> {
    validate_config(config)?;
    validate_inputs(mesh, base_elevation, deformation)?;

    let mut elevation: Vec<_> = base_elevation
        .cell_elevations
        .iter()
        .zip(&deformation.cell_deformation)
        .map(|(&base, &deformation)| base + deformation)
        .collect();

    smooth(
        mesh,
        &mut elevation,
        config.smoothing_passes,
        config.smoothing_weight,
    );
    elevation
        .iter_mut()
        .for_each(|value| *value = value.clamp(0.0, 1.0));

    let diagnostics = FieldSummary::from_values(&elevation);
    Ok(CoarseElevation {
        cell_elevations: elevation,
        sea_level: config.sea_level,
        diagnostics,
    })
}

fn validate_config(config: CoarseElevationConfig) -> Result<(), CoarseElevationError> {
    if !config.smoothing_weight.is_finite() || !(0.0..=1.0).contains(&config.smoothing_weight) {
        return Err(CoarseElevationError::InvalidConfig);
    }
    if !config.sea_level.is_finite() || config.sea_level <= 0.0 || config.sea_level >= 1.0 {
        return Err(CoarseElevationError::InvalidSeaLevel);
    }
    Ok(())
}

fn validate_inputs(
    mesh: &SphereMesh,
    base_elevation: &BaseElevation,
    deformation: &BoundaryDeformation,
) -> Result<(), CoarseElevationError> {
    if base_elevation.cell_elevations.len() != mesh.cell_count()
        || deformation.cell_deformation.len() != mesh.cell_count()
    {
        return Err(CoarseElevationError::FieldCountMismatch);
    }
    Ok(())
}

fn smooth(mesh: &SphereMesh, elevation: &mut Vec<f32>, passes: usize, weight: f32) {
    let mut next = vec![0.0; elevation.len()];
    for _ in 0..passes {
        for cell in 0..mesh.cell_count() {
            let neighbors = mesh.cell_corners(cell);
            let average = neighbors
                .iter()
                .map(|corner| elevation[corner.neighbor])
                .sum::<f32>()
                / neighbors.len() as f32;
            next[cell] = elevation[cell] + weight * (average - elevation[cell]);
        }
        std::mem::swap(elevation, &mut next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        final_state_fixture, reference_evolution_config, reference_flow_field,
        two_plate_boundary_partition,
    };
    use crate::{
        BaseElevationConfig, BaseElevationDiagnostics, CellCrust, derive_base_elevation,
        derive_seafloor_age,
    };

    /// The datum the pipeline defaults to, which several cases here vary from.
    fn default_sea_level() -> f32 {
        CoarseElevationConfig::default().sea_level
    }

    fn final_fixture() -> (SphereMesh, BaseElevation, BoundaryDeformation) {
        let (mesh, _, evolution) = final_state_fixture();
        let age = derive_seafloor_age(&mesh, &evolution, reference_evolution_config().step_count)
            .unwrap();
        let base = derive_base_elevation(
            &mesh,
            &age,
            CellCrust {
                cell_birth: &evolution.cell_birth,
            },
            &reference_flow_field(),
            BaseElevationConfig::default(),
        )
        .unwrap();
        (mesh, base, evolution.deformation)
    }

    #[test]
    fn an_elevation_field_classifies_land_against_its_own_datum() {
        let (mesh, _, _) = two_plate_boundary_partition();
        for sea_level in [0.3, default_sea_level(), 0.7] {
            let mut elevation = CoarseElevation {
                cell_elevations: vec![sea_level; mesh.cell_count()],
                sea_level,
                diagnostics: Default::default(),
            };
            assert_eq!(elevation.validate(&mesh), Ok(()));
            assert!(!elevation.field().is_land(0));
            assert_eq!(elevation.field().land_cell_count(), 0);

            elevation.cell_elevations[0] = sea_level + 0.01;
            assert!(elevation.field().is_land(0));
            assert_eq!(elevation.field().land_cell_count(), 1);
            elevation.cell_elevations.pop();
            assert_eq!(elevation.validate(&mesh), Err(StageInputError::Elevation));
        }
    }

    #[test]
    fn normalized_land_elevation_has_one_canonical_physical_scale() {
        // Both datums and their midpoints are exact in binary32, so the scale
        // is asserted exactly rather than within a tolerance.
        for sea_level in [default_sea_level(), 0.25] {
            assert_eq!(land_elevation_meters(0.0, sea_level, 10_000.0), 0.0);
            assert_eq!(land_elevation_meters(sea_level, sea_level, 10_000.0), 0.0);
            assert_eq!(
                land_elevation_meters((1.0 + sea_level) / 2.0, sea_level, 10_000.0),
                5_000.0
            );
            assert_eq!(land_elevation_meters(1.0, sea_level, 10_000.0), 10_000.0);
        }
    }

    #[test]
    fn raising_the_datum_only_floods_and_lowering_it_only_exposes() {
        let (mesh, base, deformation) = final_fixture();
        let counts: Vec<_> = [0.45, default_sea_level(), 0.55]
            .into_iter()
            .map(|sea_level| {
                compose_coarse_elevation(
                    &mesh,
                    &base,
                    &deformation,
                    CoarseElevationConfig {
                        sea_level,
                        ..Default::default()
                    },
                )
                .unwrap()
                .field()
                .land_cell_count()
            })
            .collect();

        assert!(counts[0] > counts[1], "{counts:?}");
        assert!(counts[1] > counts[2], "{counts:?}");
    }

    #[test]
    fn the_datum_changes_classification_without_changing_the_field() {
        let (mesh, base, deformation) = final_fixture();
        let default =
            compose_coarse_elevation(&mesh, &base, &deformation, CoarseElevationConfig::default())
                .unwrap();
        let flooded = compose_coarse_elevation(
            &mesh,
            &base,
            &deformation,
            CoarseElevationConfig {
                sea_level: 0.55,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(default.cell_elevations, flooded.cell_elevations);
        assert_eq!(default.diagnostics, flooded.diagnostics);
        assert_eq!(flooded.sea_level, 0.55);
    }

    #[test]
    fn composition_is_deterministic_and_normalized() {
        let (mesh, base, deformation) = final_fixture();
        let config = CoarseElevationConfig::default();
        let first = compose_coarse_elevation(&mesh, &base, &deformation, config).unwrap();

        assert_eq!(
            first,
            compose_coarse_elevation(&mesh, &base, &deformation, config).unwrap()
        );
        assert!(
            first
                .cell_elevations
                .iter()
                .all(|value| (0.0..=1.0).contains(value))
        );
        // Normalization would also accept a field that collapsed to one value.
        assert!(first.diagnostics.minimum < first.diagnostics.maximum);
    }

    #[test]
    fn composition_adds_base_and_deformation_exactly_once() {
        let (mesh, _, _) = two_plate_boundary_partition();
        let cell = mesh.edges[0].cells[0];
        let mut base_values = vec![0.2; mesh.cell_count()];
        base_values[cell] = 0.3;
        let base = BaseElevation {
            cell_elevations: base_values,
            diagnostics: BaseElevationDiagnostics::default(),
        };
        let mut deformation_values = vec![0.0; mesh.cell_count()];
        deformation_values[cell] = 0.1;
        let deformation = BoundaryDeformation {
            cell_deformation: deformation_values,
            diagnostics: Default::default(),
        };

        let composed = compose_coarse_elevation(
            &mesh,
            &base,
            &deformation,
            CoarseElevationConfig {
                smoothing_passes: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(composed.cell_elevations[cell], 0.4);
    }

    #[test]
    fn composition_smooths_simultaneously_then_clamps() {
        let (mesh, _, _) = two_plate_boundary_partition();
        let source = mesh.edges[0].cells[0];
        let neighbor = mesh.cell_corners(source)[0].neighbor;
        let base = BaseElevation {
            cell_elevations: vec![0.65; mesh.cell_count()],
            diagnostics: Default::default(),
        };
        let mut cell_deformation = vec![0.0; mesh.cell_count()];
        cell_deformation[source] = 1.0;
        cell_deformation[neighbor] = -1.0;
        let deformation = BoundaryDeformation {
            cell_deformation,
            diagnostics: Default::default(),
        };
        let unsmoothed = compose_coarse_elevation(
            &mesh,
            &base,
            &deformation,
            CoarseElevationConfig {
                smoothing_passes: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(unsmoothed.cell_elevations[source], 1.0);
        assert_eq!(unsmoothed.cell_elevations[neighbor], 0.0);

        let smoothed = compose_coarse_elevation(
            &mesh,
            &base,
            &deformation,
            CoarseElevationConfig {
                smoothing_passes: 1,
                smoothing_weight: 1.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(smoothed.cell_elevations[source] < 1.0);
        assert!(smoothed.cell_elevations[neighbor] > 0.0);
    }

    #[test]
    fn rejects_invalid_configuration_and_mismatched_fields() {
        let (mesh, base, deformation) = final_fixture();
        assert_eq!(
            compose_coarse_elevation(
                &mesh,
                &base,
                &deformation,
                CoarseElevationConfig {
                    smoothing_weight: 1.1,
                    ..Default::default()
                }
            ),
            Err(CoarseElevationError::InvalidConfig)
        );

        for sea_level in [0.0, 1.0, -0.1, 1.1, f32::NAN, f32::INFINITY] {
            assert_eq!(
                compose_coarse_elevation(
                    &mesh,
                    &base,
                    &deformation,
                    CoarseElevationConfig {
                        sea_level,
                        ..Default::default()
                    }
                ),
                Err(CoarseElevationError::InvalidSeaLevel),
                "sea level {sea_level}"
            );
        }

        let short_base = BaseElevation {
            cell_elevations: base.cell_elevations[..mesh.cell_count() - 1].to_vec(),
            diagnostics: base.diagnostics,
        };
        assert_eq!(
            compose_coarse_elevation(
                &mesh,
                &short_base,
                &deformation,
                CoarseElevationConfig::default()
            ),
            Err(CoarseElevationError::FieldCountMismatch)
        );

        let short_deformation = BoundaryDeformation {
            cell_deformation: deformation.cell_deformation[..mesh.cell_count() - 1].to_vec(),
            diagnostics: deformation.diagnostics,
        };
        assert_eq!(
            compose_coarse_elevation(
                &mesh,
                &base,
                &short_deformation,
                CoarseElevationConfig::default()
            ),
            Err(CoarseElevationError::FieldCountMismatch)
        );
    }
}
