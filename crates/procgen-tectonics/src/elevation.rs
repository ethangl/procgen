use crate::{BaseElevation, BoundaryDeformation, FieldSummary};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoarseElevationConfig {
    pub smoothing_passes: usize,
    pub smoothing_weight: f32,
}

impl Default for CoarseElevationConfig {
    fn default() -> Self {
        Self {
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
    FieldCountMismatch,
}

impl fmt::Display for CoarseElevationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => {
                formatter.write_str("smoothing weight must be finite and between 0 and 1")
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
/// separate stages with no elevation history or inter-step state.
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
        diagnostics,
    })
}

fn validate_config(config: CoarseElevationConfig) -> Result<(), CoarseElevationError> {
    if !config.smoothing_weight.is_finite() || !(0.0..=1.0).contains(&config.smoothing_weight) {
        return Err(CoarseElevationError::InvalidConfig);
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
    use crate::test_support::{final_state_fixture, fingerprint, two_plate_boundary_partition};
    use crate::{
        BaseElevationConfig, BaseElevationDiagnostics, BoundaryDeformationConfig,
        SeafloorAgeConfig, derive_base_elevation, derive_boundary_deformation, derive_seafloor_age,
    };

    fn final_fixture() -> (SphereMesh, BaseElevation, BoundaryDeformation) {
        let (mesh, partition, crust, boundaries) = final_state_fixture();
        let age = derive_seafloor_age(
            &mesh,
            &partition,
            &crust,
            &boundaries,
            SeafloorAgeConfig::default(),
        )
        .unwrap();
        let base = derive_base_elevation(
            &mesh,
            &partition,
            &crust,
            &age,
            BaseElevationConfig::default(),
        )
        .unwrap();
        let deformation = derive_boundary_deformation(
            &mesh,
            &partition,
            &crust,
            &boundaries,
            BoundaryDeformationConfig::default(),
        )
        .unwrap();
        (mesh, base, deformation)
    }

    #[test]
    fn composition_is_deterministic_normalized_and_preserves_the_pipeline_fingerprint() {
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
        let fingerprint = fingerprint(
            first
                .cell_elevations
                .iter()
                .map(|value| value.to_bits() as u64),
        );
        assert_eq!(fingerprint, 18_396_277_247_688_344_762);
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
