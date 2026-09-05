use crate::{
    BoundaryClass, BoundaryClassification, CrustClass, CrustClassification, FieldSummary,
    PlatePartition,
    stage::{StageInputError, validate_boundaries, validate_ownership_and_crust},
};
use procgen_sphere_mesh::SphereMesh;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeafloorAgeConfig {
    /// Hop age assigned to every cell on an oceanic plate with no divergent boundary.
    pub ridge_less_age: usize,
}

impl Default for SeafloorAgeConfig {
    fn default() -> Self {
        Self { ridge_less_age: 8 }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeafloorAgeDiagnostics {
    pub summary: FieldSummary,
    pub oceanic_cell_count: usize,
    pub ridge_cell_count: usize,
    pub ridge_plate_count: usize,
    pub ridge_less_plate_count: usize,
    pub fallback_cell_count: usize,
}

/// Per-cell proxy seafloor age measured in mesh hops from the nearest final ridge.
/// Continental cells have no seafloor age.
#[derive(Clone, Debug, PartialEq)]
pub struct SeafloorAge {
    pub cell_ages: Vec<Option<usize>>,
    pub diagnostics: SeafloorAgeDiagnostics,
}

/// Derives seafloor hop age from the final divergent boundaries and ownership.
///
/// Oceanic cells touching a divergent edge are age zero. A multi-source BFS
/// propagates age only through cells with the same final plate owner. Oceanic
/// plates with no ridge receive the configured fallback age uniformly;
/// continental cells remain `None`.
pub fn derive_seafloor_age(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    boundaries: &BoundaryClassification,
    config: SeafloorAgeConfig,
) -> Result<SeafloorAge, StageInputError> {
    validate_ownership_and_crust(mesh, partition, crust)?;
    validate_boundaries(mesh, boundaries)?;

    let mut cell_ages = vec![None; mesh.cell_count()];
    let mut ridge_plates = vec![false; partition.plate_count];
    let mut queue = VecDeque::new();

    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        if boundaries.edge_classes[edge_index] != BoundaryClass::Divergent {
            continue;
        }
        for &cell in &edge.cells {
            let plate = partition.cell_plates[cell];
            if crust.cell_class(partition, cell) != CrustClass::Oceanic {
                continue;
            }
            ridge_plates[plate] = true;
            if cell_ages[cell].is_none() {
                cell_ages[cell] = Some(0);
                queue.push_back((cell, 0));
            }
        }
    }
    let ridge_cell_count = queue.len();

    while let Some((cell, age)) = queue.pop_front() {
        let plate = partition.cell_plates[cell];
        let next_age = age + 1;
        for corner in mesh.cell_corners(cell) {
            let neighbor = corner.neighbor;
            if cell_ages[neighbor].is_none() && partition.cell_plates[neighbor] == plate {
                cell_ages[neighbor] = Some(next_age);
                queue.push_back((neighbor, next_age));
            }
        }
    }

    let mut fallback_cell_count = 0;
    for (cell, age) in cell_ages.iter_mut().enumerate() {
        if crust.cell_class(partition, cell) == CrustClass::Oceanic && age.is_none() {
            *age = Some(config.ridge_less_age);
            fallback_cell_count += 1;
        }
    }
    let oceanic_ages: Vec<_> = cell_ages.iter().flatten().map(|&age| age as f32).collect();
    let ridge_plate_count = ridge_plates.iter().filter(|&&has_ridge| has_ridge).count();
    let diagnostics = SeafloorAgeDiagnostics {
        summary: FieldSummary::from_values(&oceanic_ages),
        oceanic_cell_count: oceanic_ages.len(),
        ridge_cell_count,
        ridge_plate_count,
        ridge_less_plate_count: crust.plate_count(CrustClass::Oceanic) - ridge_plate_count,
        fallback_cell_count,
    };

    Ok(SeafloorAge {
        cell_ages,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        empty_boundaries, final_state_fixture, fingerprint, reference_partition,
    };

    #[test]
    fn final_age_field_is_deterministic_and_has_stable_aggregates() {
        let (mesh, partition, crust, boundaries) = final_state_fixture();
        let config = SeafloorAgeConfig::default();
        let first = derive_seafloor_age(&mesh, &partition, &crust, &boundaries, config).unwrap();

        assert_eq!(
            first,
            derive_seafloor_age(&mesh, &partition, &crust, &boundaries, config).unwrap()
        );
        assert_eq!(first.cell_ages.len(), mesh.cell_count());
        assert_eq!(
            first.diagnostics,
            SeafloorAgeDiagnostics {
                summary: FieldSummary {
                    minimum: 0.0,
                    maximum: 3.0,
                    mean: 0.666_666_7,
                },
                oceanic_cell_count: 270,
                ridge_cell_count: 147,
                ridge_plate_count: 13,
                ridge_less_plate_count: 0,
                fallback_cell_count: 0,
            }
        );
        assert_eq!(
            fingerprint(
                first
                    .cell_ages
                    .iter()
                    .map(|age| { age.map_or(u64::MAX, |age| age as u64) })
            ),
            7_651_439_301_921_684_041
        );
    }

    #[test]
    fn ridge_less_oceanic_plates_use_the_configured_age_and_continents_stay_empty() {
        let (mesh, partition) = reference_partition();
        let crust = CrustClassification {
            plate_classes: (0..partition.plate_count)
                .map(|plate| {
                    if plate == 0 {
                        CrustClass::Continental
                    } else {
                        CrustClass::Oceanic
                    }
                })
                .collect(),
        };
        let age = derive_seafloor_age(
            &mesh,
            &partition,
            &crust,
            &empty_boundaries(&mesh),
            SeafloorAgeConfig { ridge_less_age: 13 },
        )
        .unwrap();

        for (cell, &plate) in partition.cell_plates.iter().enumerate() {
            assert_eq!(
                age.cell_ages[cell],
                (plate != 0).then_some(13),
                "cell {cell} on plate {plate}"
            );
        }
        assert_eq!(age.diagnostics.ridge_plate_count, 0);
        assert_eq!(
            age.diagnostics.ridge_less_plate_count,
            partition.plate_count - 1
        );
        assert_eq!(
            age.diagnostics.fallback_cell_count,
            age.diagnostics.oceanic_cell_count
        );
    }

    #[test]
    fn all_continental_world_has_no_age_or_aggregates() {
        let (mesh, partition) = reference_partition();
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Continental; partition.plate_count],
        };

        let age = derive_seafloor_age(
            &mesh,
            &partition,
            &crust,
            &empty_boundaries(&mesh),
            SeafloorAgeConfig::default(),
        )
        .unwrap();

        assert!(age.cell_ages.iter().all(Option::is_none));
        assert_eq!(age.diagnostics, SeafloorAgeDiagnostics::default());
    }

    #[test]
    fn propagation_stays_within_final_oceanic_plate_ownership() {
        let (mesh, partition) = reference_partition();
        let mut boundaries = empty_boundaries(&mesh);
        let edge_index = mesh
            .edges
            .iter()
            .position(|edge| {
                partition.cell_plates[edge.cells[0]] != partition.cell_plates[edge.cells[1]]
            })
            .unwrap();
        boundaries.edge_classes[edge_index] = BoundaryClass::Divergent;
        let ridge_edge = mesh.edges[edge_index];
        let ridge_plates = ridge_edge.cells.map(|cell| partition.cell_plates[cell]);
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Oceanic; partition.plate_count],
        };
        let age = derive_seafloor_age(
            &mesh,
            &partition,
            &crust,
            &boundaries,
            SeafloorAgeConfig { ridge_less_age: 23 },
        )
        .unwrap();

        assert_eq!(age.cell_ages[ridge_edge.cells[0]], Some(0));
        assert_eq!(age.cell_ages[ridge_edge.cells[1]], Some(0));
        let same_plate_neighbor = mesh
            .cell_corners(ridge_edge.cells[0])
            .iter()
            .map(|corner| corner.neighbor)
            .find(|&neighbor| partition.cell_plates[neighbor] == ridge_plates[0])
            .unwrap();
        assert_eq!(age.cell_ages[same_plate_neighbor], Some(1));
        for (cell, &plate) in partition.cell_plates.iter().enumerate() {
            if !ridge_plates.contains(&plate) {
                assert_eq!(age.cell_ages[cell], Some(23));
            }
        }
        assert_eq!(age.diagnostics.ridge_plate_count, 2);
        assert_eq!(
            age.diagnostics.ridge_less_plate_count,
            partition.plate_count - 2
        );
    }

    #[test]
    fn rejects_mismatched_inputs() {
        let (mesh, partition) = reference_partition();
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Oceanic; partition.plate_count],
        };
        let boundaries = empty_boundaries(&mesh);

        let mut wrong_partition = partition.clone();
        wrong_partition.cell_plates.pop();
        assert_eq!(
            derive_seafloor_age(
                &mesh,
                &wrong_partition,
                &crust,
                &boundaries,
                Default::default()
            ),
            Err(StageInputError::Cells)
        );

        let wrong_crust = CrustClassification {
            plate_classes: Vec::new(),
        };
        assert_eq!(
            derive_seafloor_age(
                &mesh,
                &partition,
                &wrong_crust,
                &boundaries,
                Default::default()
            ),
            Err(StageInputError::Plates)
        );

        let mut wrong_boundaries = boundaries;
        wrong_boundaries.edge_classes.pop();
        assert_eq!(
            derive_seafloor_age(
                &mesh,
                &partition,
                &crust,
                &wrong_boundaries,
                Default::default()
            ),
            Err(StageInputError::Boundaries)
        );
    }
}
