use procgen_sphere_mesh::SphereMesh;
use procgen_tectonics::{
    CoarseElevation, CrustClass, CrustClassification, FieldSummary, PlatePartition, StageInputError,
};
use std::{collections::VecDeque, fmt};

const SEA_LEVEL: f32 = 0.5;

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
    /// Minimum graph distance from a final plate boundary. Worlds without a
    /// plate boundary retain `None` for every cell.
    pub cell_boundary_distances: Vec<Option<usize>>,
    /// Normalized present-day craton eligibility. This field never mutates elevation.
    pub cell_strengths: Vec<f32>,
    pub diagnostics: CratonDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CratonFieldError {
    Input(StageInputError),
    ElevationCellCountMismatch,
}

impl fmt::Display for CratonFieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => error.fmt(formatter),
            Self::ElevationCellCountMismatch => {
                formatter.write_str("elevation values must match the mesh cell count")
            }
        }
    }
}

impl std::error::Error for CratonFieldError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            Self::ElevationCellCountMismatch => None,
        }
    }
}

impl From<StageInputError> for CratonFieldError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
    }
}

/// Derives a present-day craton-strength field from final plate-boundary
/// distance. Only continental cells strictly above normalized sea level are
/// eligible. The operation reads coarse elevation without modifying it and
/// does not infer plate age or continuity history.
pub fn derive_craton_field(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: &CrustClassification,
    elevation: &CoarseElevation,
    config: CratonFieldConfig,
) -> Result<CratonField, CratonFieldError> {
    validate_inputs(mesh, plates, crust, elevation)?;

    let cell_boundary_distances = boundary_distances(mesh, plates);
    let mut continental_land_cell_count = 0;
    let mut craton_cell_count = 0;
    let mut full_strength_cell_count = 0;
    let cell_strengths: Vec<_> = cell_boundary_distances
        .iter()
        .enumerate()
        .map(|(cell, &distance)| {
            let eligible = crust.cell_class(plates, cell) == CrustClass::Continental
                && elevation.cell_elevations[cell] > SEA_LEVEL;
            continental_land_cell_count += usize::from(eligible);
            let strength = distance
                .filter(|_| eligible)
                .map_or(0.0, |distance| strength_at_distance(distance, config));
            craton_cell_count += usize::from(strength > 0.0);
            full_strength_cell_count += usize::from(strength == 1.0);
            strength
        })
        .collect();

    let boundary_cell_count = cell_boundary_distances
        .iter()
        .filter(|&&distance| distance == Some(0))
        .count();
    let maximum_boundary_distance = cell_boundary_distances.iter().flatten().copied().max();

    Ok(CratonField {
        cell_boundary_distances,
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

fn validate_inputs(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: &CrustClassification,
    elevation: &CoarseElevation,
) -> Result<(), CratonFieldError> {
    plates.validate(mesh)?;
    crust.validate(plates)?;
    if elevation.cell_elevations.len() != mesh.cell_count() {
        return Err(CratonFieldError::ElevationCellCountMismatch);
    }
    Ok(())
}

fn boundary_distances(mesh: &SphereMesh, plates: &PlatePartition) -> Vec<Option<usize>> {
    let mut distances = vec![None; mesh.cell_count()];
    let mut queue = VecDeque::new();
    for edge in &mesh.edges {
        if plates.cell_plates[edge.cells[0]] == plates.cell_plates[edge.cells[1]] {
            continue;
        }
        for cell in edge.cells {
            if distances[cell].is_none() {
                distances[cell] = Some(0);
                queue.push_back(cell);
            }
        }
    }

    while let Some(cell) = queue.pop_front() {
        let next_distance = distances[cell].expect("queued cells have a distance") + 1;
        for corner in mesh.cell_corners(cell) {
            if distances[corner.neighbor].is_none() {
                distances[corner.neighbor] = Some(next_distance);
                queue.push_back(corner.neighbor);
            }
        }
    }
    distances
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
        CrustClassificationConfig, PlatePartitionConfig, classify_crust, partition_plates,
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
        for edge in &mesh.edges {
            let distances = edge.cells.map(|cell| first.cell_boundary_distances[cell]);
            if plates.cell_plates[edge.cells[0]] != plates.cell_plates[edge.cells[1]] {
                assert_eq!(distances, [Some(0), Some(0)]);
            }
            assert!(distances[0].unwrap().abs_diff(distances[1].unwrap()) <= 1);
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
            .cell_boundary_distances
            .iter()
            .zip(&field.cell_strengths)
            .flat_map(|(distance, strength)| {
                [
                    distance.map_or(u64::MAX, |distance| distance as u64),
                    u64::from(strength.to_bits()),
                ]
            });

        assert_eq!(fingerprint(values), 2_206_889_484_806_018_648);
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

        let boundary_cell = hard_cutoff
            .cell_boundary_distances
            .iter()
            .position(|&distance| distance == Some(0))
            .unwrap();
        let interior_cell = hard_cutoff
            .cell_boundary_distances
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
        for (distance, &strength) in ramped
            .cell_boundary_distances
            .iter()
            .zip(&ramped.cell_strengths)
        {
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

        assert!(field.cell_boundary_distances.iter().all(Option::is_none));
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
            Err(CratonFieldError::Input(StageInputError::Cells))
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
            Err(CratonFieldError::Input(StageInputError::Plates))
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
            Err(CratonFieldError::ElevationCellCountMismatch)
        );
    }
}
