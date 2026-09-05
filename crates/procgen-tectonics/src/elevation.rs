use crate::{
    BoundaryDeformation, CrustClass, CrustClassification, FieldSummary, PlatePartition,
    stage::{StageInputError, validate_ownership_and_crust},
};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoarseElevationConfig {
    pub oceanic_base: f32,
    pub continental_base: f32,
    pub smoothing_passes: usize,
    pub smoothing_weight: f32,
}

impl Default for CoarseElevationConfig {
    fn default() -> Self {
        Self {
            oceanic_base: 0.15,
            continental_base: 0.65,
            smoothing_passes: 2,
            smoothing_weight: 0.2,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CoarseElevation {
    pub cell_elevations: Vec<f32>,
    pub diagnostics: FieldSummary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoarseElevationError {
    InvalidConfig,
    Input(StageInputError),
    DeformationCountMismatch,
}

impl fmt::Display for CoarseElevationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str(
                "base elevations must be finite and between 0 and 1, and smoothing weight must be finite and between 0 and 1",
            ),
            Self::Input(error) => error.fmt(formatter),
            Self::DeformationCountMismatch => {
                formatter.write_str("deformation values must match the mesh cell count")
            }
        }
    }
}

impl std::error::Error for CoarseElevationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidConfig | Self::DeformationCountMismatch => None,
            Self::Input(error) => Some(error),
        }
    }
}

impl From<StageInputError> for CoarseElevationError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
    }
}

/// Composes normalized coarse elevation from current-owner crust and a
/// separately derived signed deformation field.
///
/// Base crust elevation and deformation are added once, smoothed
/// simultaneously, and clamped. Ownership evolution and boundary derivation
/// remain separate stages with no elevation history or inter-step state.
pub fn compose_coarse_elevation(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    deformation: &BoundaryDeformation,
    config: CoarseElevationConfig,
) -> Result<CoarseElevation, CoarseElevationError> {
    validate_config(config)?;
    validate_inputs(mesh, partition, crust, deformation)?;

    let mut elevation: Vec<_> = (0..mesh.cell_count())
        .map(|cell| {
            let base = match crust.cell_class(partition, cell) {
                CrustClass::Oceanic => config.oceanic_base,
                CrustClass::Continental => config.continental_base,
            };
            base + deformation.cell_deformation[cell]
        })
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
        diagnostics,
    })
}

fn validate_config(config: CoarseElevationConfig) -> Result<(), CoarseElevationError> {
    if !config.oceanic_base.is_finite()
        || !config.continental_base.is_finite()
        || !config.smoothing_weight.is_finite()
        || !(0.0..=1.0).contains(&config.oceanic_base)
        || !(0.0..=1.0).contains(&config.continental_base)
        || !(0.0..=1.0).contains(&config.smoothing_weight)
    {
        return Err(CoarseElevationError::InvalidConfig);
    }
    Ok(())
}

fn validate_inputs(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    deformation: &BoundaryDeformation,
) -> Result<(), CoarseElevationError> {
    validate_ownership_and_crust(mesh, partition, crust)?;
    if deformation.cell_deformation.len() != mesh.cell_count() {
        return Err(CoarseElevationError::DeformationCountMismatch);
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
    use crate::test_support::{final_state_fixture, fingerprint, two_plate_boundary_partition};
    use crate::{BoundaryDeformationConfig, derive_boundary_deformation};

    fn final_fixture() -> (
        SphereMesh,
        PlatePartition,
        CrustClassification,
        BoundaryDeformation,
    ) {
        let (mesh, partition, crust, boundaries) = final_state_fixture();
        let deformation = derive_boundary_deformation(
            &mesh,
            &partition,
            &crust,
            &boundaries,
            BoundaryDeformationConfig::default(),
        )
        .unwrap();
        (mesh, partition, crust, deformation)
    }

    #[test]
    fn composition_is_deterministic_normalized_and_preserves_the_pipeline_fingerprint() {
        let (mesh, partition, crust, deformation) = final_fixture();
        let config = CoarseElevationConfig::default();
        let first =
            compose_coarse_elevation(&mesh, &partition, &crust, &deformation, config).unwrap();

        assert_eq!(
            first,
            compose_coarse_elevation(&mesh, &partition, &crust, &deformation, config).unwrap()
        );
        assert!(
            first
                .cell_elevations
                .iter()
                .all(|value| (0.0..=1.0).contains(value))
        );
        let fingerprint = fingerprint(
            first
                .cell_elevations
                .iter()
                .map(|value| value.to_bits() as u64),
        );
        assert_eq!(fingerprint, 4_072_787_338_629_474_632);
    }

    #[test]
    fn base_elevation_follows_current_owner_crust_and_adds_deformation() {
        let (mesh, _, mut partition) = two_plate_boundary_partition();
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Oceanic, CrustClass::Continental],
        };
        let cell = mesh.edges[0].cells[0];
        let mut cell_deformation = vec![0.0; mesh.cell_count()];
        cell_deformation[cell] = 0.1;
        let deformation = BoundaryDeformation {
            cell_deformation,
            diagnostics: Default::default(),
        };
        let config = CoarseElevationConfig {
            smoothing_passes: 0,
            ..Default::default()
        };

        let original =
            compose_coarse_elevation(&mesh, &partition, &crust, &deformation, config).unwrap();
        assert_eq!(original.cell_elevations[cell], config.oceanic_base + 0.1);

        partition.cell_plates[cell] = 1;
        let changed =
            compose_coarse_elevation(&mesh, &partition, &crust, &deformation, config).unwrap();
        assert_eq!(changed.cell_elevations[cell], config.continental_base + 0.1);
    }

    #[test]
    fn composition_smooths_simultaneously_then_clamps() {
        let (mesh, _, partition) = two_plate_boundary_partition();
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Continental; 2],
        };
        let source = mesh.edges[0].cells[0];
        let neighbor = mesh.cell_corners(source)[0].neighbor;
        let mut cell_deformation = vec![0.0; mesh.cell_count()];
        cell_deformation[source] = 1.0;
        cell_deformation[neighbor] = -1.0;
        let deformation = BoundaryDeformation {
            cell_deformation,
            diagnostics: Default::default(),
        };
        let unsmoothed = compose_coarse_elevation(
            &mesh,
            &partition,
            &crust,
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
            &partition,
            &crust,
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
    fn rejects_invalid_configuration_and_mismatched_inputs() {
        let (mesh, partition, crust, deformation) = final_fixture();
        assert_eq!(
            compose_coarse_elevation(
                &mesh,
                &partition,
                &crust,
                &deformation,
                CoarseElevationConfig {
                    smoothing_weight: 1.1,
                    ..Default::default()
                }
            ),
            Err(CoarseElevationError::InvalidConfig)
        );

        let mut short_partition = partition.clone();
        short_partition.cell_plates.pop();
        assert_eq!(
            compose_coarse_elevation(
                &mesh,
                &short_partition,
                &crust,
                &deformation,
                CoarseElevationConfig::default()
            ),
            Err(CoarseElevationError::Input(StageInputError::Cells))
        );

        let short_crust = CrustClassification {
            plate_classes: crust.plate_classes[..crust.plate_classes.len() - 1].to_vec(),
        };
        assert_eq!(
            compose_coarse_elevation(
                &mesh,
                &partition,
                &short_crust,
                &deformation,
                CoarseElevationConfig::default()
            ),
            Err(CoarseElevationError::Input(StageInputError::Plates))
        );

        let short_deformation = BoundaryDeformation {
            cell_deformation: deformation.cell_deformation[..mesh.cell_count() - 1].to_vec(),
            diagnostics: deformation.diagnostics,
        };
        assert_eq!(
            compose_coarse_elevation(
                &mesh,
                &partition,
                &crust,
                &short_deformation,
                CoarseElevationConfig::default()
            ),
            Err(CoarseElevationError::DeformationCountMismatch)
        );
    }
}
