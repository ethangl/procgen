use crate::field::GeologyInputError;
use procgen_sphere_mesh::{SphereMesh, edge_cell_distances};
use procgen_tectonics::{
    CoarseElevation, CrustClass, CrustClassification, FieldSummary, PlatePartition, StageInputError,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CratonFieldConfig {
    /// Minimum graph distance from a final plate boundary before strength can begin.
    pub minimum_boundary_distance: usize,
    /// Additional graph distance over which strength ramps from zero to one.
    /// Zero applies a hard cutoff at `minimum_boundary_distance`.
    pub ramp_width: usize,
}

impl Default for CratonFieldConfig {
    fn default() -> Self {
        Self {
            minimum_boundary_distance: 3,
            ramp_width: 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CratonDiagnostics {
    pub boundary_cell_count: usize,
    pub continental_land_cell_count: usize,
    pub craton_cell_count: usize,
    pub full_strength_cell_count: usize,
    pub maximum_boundary_distance: Option<usize>,
    pub strength: FieldSummary,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CratonField {
    /// Normalized present-day craton eligibility. This field never mutates
    /// tectonic elevation.
    pub cell_strengths: Vec<f32>,
    pub diagnostics: CratonDiagnostics,
}

impl CratonField {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), GeologyInputError> {
        if self.cell_strengths.len() != mesh.cell_count() {
            return Err(GeologyInputError::Cratons);
        }
        Ok(())
    }
}

/// Derives a present-day craton-strength field from final plate-boundary
/// distance. Only continental cells strictly above normalized sea level are
/// eligible. The operation reads tectonic elevation without modifying it and
/// does not infer plate age or continuity history.
pub fn derive_craton_field(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: &CrustClassification,
    elevation: &CoarseElevation,
    config: CratonFieldConfig,
) -> Result<CratonField, StageInputError> {
    plates.validate(mesh)?;
    crust.validate(plates)?;
    elevation.validate(mesh)?;

    let cell_boundary_distances = plate_boundary_distances(mesh, plates);
    let is_eligible =
        |cell| crust.cell_class(plates, cell) == CrustClass::Continental && elevation.is_land(cell);
    let cell_strengths: Vec<_> = cell_boundary_distances
        .iter()
        .enumerate()
        .map(|(cell, &distance)| {
            distance
                .filter(|_| is_eligible(cell))
                .map_or(0.0, |distance| strength_at_distance(distance, config))
        })
        .collect();

    let continental_land_cell_count = (0..mesh.cell_count())
        .filter(|&cell| is_eligible(cell))
        .count();
    let craton_cell_count = cell_strengths
        .iter()
        .filter(|&&strength| strength > 0.0)
        .count();
    let full_strength_cell_count = cell_strengths
        .iter()
        .filter(|&&strength| strength == 1.0)
        .count();
    let boundary_cell_count = cell_boundary_distances
        .iter()
        .filter(|&&distance| distance == Some(0))
        .count();
    let maximum_boundary_distance = cell_boundary_distances.iter().flatten().copied().max();

    Ok(CratonField {
        diagnostics: CratonDiagnostics {
            boundary_cell_count,
            continental_land_cell_count,
            craton_cell_count,
            full_strength_cell_count,
            maximum_boundary_distance,
            strength: FieldSummary::from_values(&cell_strengths),
        },
        cell_strengths,
    })
}

fn plate_boundary_distances(mesh: &SphereMesh, plates: &PlatePartition) -> Vec<Option<usize>> {
    edge_cell_distances(mesh, |_, edge| {
        plates.cell_plates[edge.cells[0]] != plates.cell_plates[edge.cells[1]]
    })
}

fn strength_at_distance(distance: usize, config: CratonFieldConfig) -> f32 {
    if distance < config.minimum_boundary_distance {
        return 0.0;
    }
    if config.ramp_width == 0 {
        return 1.0;
    }
    ((distance - config.minimum_boundary_distance) as f32 / config.ramp_width as f32).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_core::fingerprint;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::build_sphere_mesh;
    use procgen_tectonics::{
        CrustClassificationConfig, PlatePartitionConfig, SEA_LEVEL, classify_crust,
        partition_plates,
    };

    fn fixture(
        cell_count: usize,
        major_plate_count: usize,
        minor_plate_count: usize,
    ) -> (
        SphereMesh,
        PlatePartition,
        CrustClassification,
        CoarseElevation,
    ) {
        let mesh = build_sphere_mesh(
            fibonacci_sphere(FibonacciConfig {
                count: cell_count,
                jitter: 0.5,
                seed: 7,
            })
            .unwrap(),
            1.0,
        )
        .unwrap();
        let plates = partition_plates(
            &mesh,
            PlatePartitionConfig {
                major_plate_count,
                minor_plate_count,
                major_head_start_rounds: 2,
                seed: 11,
            },
        )
        .unwrap();
        let crust = classify_crust(
            &mesh,
            &plates,
            CrustClassificationConfig {
                target_ocean_fraction: 0.7,
                seed: 17,
            },
        )
        .unwrap();
        let elevation = CoarseElevation {
            cell_elevations: vec![0.65; mesh.cell_count()],
            diagnostics: Default::default(),
        };
        (mesh, plates, crust, elevation)
    }

    #[test]
    fn field_is_deterministic_bounded_and_preserves_elevation() {
        let (mesh, plates, crust, elevation) = fixture(1_024, 8, 12);
        let original_elevation = elevation.clone();
        let config = CratonFieldConfig::default();
        let first = derive_craton_field(&mesh, &plates, &crust, &elevation, config).unwrap();

        assert_eq!(
            first,
            derive_craton_field(&mesh, &plates, &crust, &elevation, config).unwrap()
        );
        assert_eq!(elevation, original_elevation);
        assert_eq!(first.cell_strengths.len(), mesh.cell_count());
        assert!(
            first
                .cell_strengths
                .iter()
                .all(|strength| (0.0..=1.0).contains(strength))
        );
        assert!(first.diagnostics.boundary_cell_count > 0);
        assert!(first.diagnostics.craton_cell_count > 0);
        let distances = plate_boundary_distances(&mesh, &plates);
        for edge in &mesh.edges {
            let edge_distances = edge.cells.map(|cell| distances[cell]);
            if plates.cell_plates[edge.cells[0]] != plates.cell_plates[edge.cells[1]] {
                assert_eq!(edge_distances, [Some(0), Some(0)]);
            }
            assert!(
                edge_distances[0]
                    .unwrap()
                    .abs_diff(edge_distances[1].unwrap())
                    <= 1
            );
        }
    }

    #[test]
    fn strength_ramp_has_explicit_cutoff_and_saturation_edges() {
        let ramped = CratonFieldConfig {
            minimum_boundary_distance: 3,
            ramp_width: 3,
        };
        assert_eq!(strength_at_distance(2, ramped), 0.0);
        assert_eq!(strength_at_distance(3, ramped), 0.0);
        assert_eq!(strength_at_distance(4, ramped), 1.0 / 3.0);
        assert_eq!(strength_at_distance(6, ramped), 1.0);
        assert_eq!(strength_at_distance(12, ramped), 1.0);

        let cutoff = CratonFieldConfig {
            minimum_boundary_distance: 3,
            ramp_width: 0,
        };
        assert_eq!(strength_at_distance(2, cutoff), 0.0);
        assert_eq!(strength_at_distance(3, cutoff), 1.0);
    }

    #[test]
    fn reference_field_has_stable_fingerprint() {
        let (mesh, plates, crust, elevation) = fixture(1_024, 8, 12);
        let field = derive_craton_field(
            &mesh,
            &plates,
            &crust,
            &elevation,
            CratonFieldConfig::default(),
        )
        .unwrap();
        let values = field
            .cell_strengths
            .iter()
            .map(|strength| u64::from(strength.to_bits()));

        assert_eq!(fingerprint(values), 15_008_142_936_841_976_928);
    }

    #[test]
    fn eligibility_and_ramp_follow_present_day_inputs() {
        let (mesh, plates, mut crust, mut elevation) = fixture(512, 4, 4);
        crust.plate_classes.fill(CrustClass::Continental);
        let hard_cutoff = derive_craton_field(
            &mesh,
            &plates,
            &crust,
            &elevation,
            CratonFieldConfig {
                minimum_boundary_distance: 0,
                ramp_width: 0,
            },
        )
        .unwrap();
        assert!(
            hard_cutoff
                .cell_strengths
                .iter()
                .all(|&strength| strength == 1.0)
        );

        let distances = plate_boundary_distances(&mesh, &plates);
        let boundary_cell = distances
            .iter()
            .position(|&distance| distance == Some(0))
            .unwrap();
        let interior_cell = distances
            .iter()
            .enumerate()
            .max_by_key(|(_, distance)| *distance)
            .map(|(cell, _)| cell)
            .unwrap();
        elevation.cell_elevations[boundary_cell] = SEA_LEVEL;
        crust.plate_classes[plates.cell_plates[interior_cell]] = CrustClass::Oceanic;

        let ramped = derive_craton_field(
            &mesh,
            &plates,
            &crust,
            &elevation,
            CratonFieldConfig {
                minimum_boundary_distance: 0,
                ramp_width: 2,
            },
        )
        .unwrap();
        assert_eq!(ramped.cell_strengths[boundary_cell], 0.0);
        assert_eq!(ramped.cell_strengths[interior_cell], 0.0);
        for (distance, &strength) in distances.iter().zip(&ramped.cell_strengths) {
            if strength > 0.0 {
                assert_eq!(strength, (distance.unwrap() as f32 / 2.0).min(1.0));
            }
        }
    }

    #[test]
    fn one_plate_world_has_no_boundary_distance_or_cratons() {
        let (mesh, plates, mut crust, elevation) = fixture(128, 1, 0);
        crust.plate_classes.fill(CrustClass::Continental);
        let field = derive_craton_field(
            &mesh,
            &plates,
            &crust,
            &elevation,
            CratonFieldConfig {
                minimum_boundary_distance: 0,
                ramp_width: 0,
            },
        )
        .unwrap();

        assert!(
            plate_boundary_distances(&mesh, &plates)
                .iter()
                .all(Option::is_none)
        );
        assert!(field.cell_strengths.iter().all(|&strength| strength == 0.0));
        assert_eq!(field.diagnostics.boundary_cell_count, 0);
        assert_eq!(field.diagnostics.craton_cell_count, 0);
        assert_eq!(field.diagnostics.maximum_boundary_distance, None);
    }

    #[test]
    fn rejects_mismatched_inputs() {
        let (mesh, plates, crust, elevation) = fixture(128, 2, 2);

        let mut invalid_plates = plates.clone();
        invalid_plates.cell_plates.pop();
        assert_eq!(
            derive_craton_field(
                &mesh,
                &invalid_plates,
                &crust,
                &elevation,
                CratonFieldConfig::default(),
            ),
            Err(StageInputError::Cells)
        );

        let mut invalid_crust = crust.clone();
        invalid_crust.plate_classes.pop();
        assert_eq!(
            derive_craton_field(
                &mesh,
                &plates,
                &invalid_crust,
                &elevation,
                CratonFieldConfig::default(),
            ),
            Err(StageInputError::Plates)
        );

        let mut invalid_elevation = elevation.clone();
        invalid_elevation.cell_elevations.pop();
        assert_eq!(
            derive_craton_field(
                &mesh,
                &plates,
                &crust,
                &invalid_elevation,
                CratonFieldConfig::default(),
            ),
            Err(StageInputError::Elevation)
        );
    }
}
