use crate::{
    BoundaryClass, BoundaryClassification, CrustClass, CrustClassification, PlatePartition,
};
use procgen_sphere_mesh::SphereMesh;
use std::{collections::VecDeque, fmt};

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
    pub oceanic_cell_count: usize,
    pub ridge_cell_count: usize,
    pub ridge_plate_count: usize,
    pub ridge_less_plate_count: usize,
    pub fallback_cell_count: usize,
    pub minimum_age: usize,
    pub maximum_age: usize,
    pub mean_age: f32,
}

/// Per-cell proxy seafloor age measured in mesh hops from the nearest final ridge.
/// Continental cells have no seafloor age.
#[derive(Clone, Debug, PartialEq)]
pub struct SeafloorAge {
    pub cell_ages: Vec<Option<usize>>,
    pub diagnostics: SeafloorAgeDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeafloorAgeError {
    CellCountMismatch,
    PlateCountMismatch,
    BoundaryCountMismatch,
}

impl fmt::Display for SeafloorAgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CellCountMismatch => {
                formatter.write_str("plate assignments must match the mesh cell count")
            }
            Self::PlateCountMismatch => {
                formatter.write_str("plate classes must match the partition plate count")
            }
            Self::BoundaryCountMismatch => {
                formatter.write_str("boundary arrays must match the mesh edge count")
            }
        }
    }
}

impl std::error::Error for SeafloorAgeError {}

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
) -> Result<SeafloorAge, SeafloorAgeError> {
    validate_inputs(mesh, partition, crust, boundaries)?;

    let mut cell_ages = vec![None; mesh.cell_count()];
    let mut ridge_plates = vec![false; partition.plate_count];
    let mut queue = VecDeque::new();

    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        if boundaries.edge_classes[edge_index] != BoundaryClass::Divergent {
            continue;
        }
        for &cell in &edge.cells {
            let plate = partition.cell_plates[cell];
            if crust.plate_classes[plate] != CrustClass::Oceanic {
                continue;
            }
            ridge_plates[plate] = true;
            if cell_ages[cell].is_none() {
                cell_ages[cell] = Some(0);
                queue.push_back(cell);
            }
        }
    }

    while let Some(cell) = queue.pop_front() {
        let plate = partition.cell_plates[cell];
        let next_age = cell_ages[cell].expect("queued cells have an age") + 1;
        for corner in mesh.cell_corners(cell) {
            let neighbor = corner.neighbor;
            if cell_ages[neighbor].is_none() && partition.cell_plates[neighbor] == plate {
                cell_ages[neighbor] = Some(next_age);
                queue.push_back(neighbor);
            }
        }
    }

    let mut diagnostics = SeafloorAgeDiagnostics {
        ridge_plate_count: ridge_plates.iter().filter(|&&has_ridge| has_ridge).count(),
        ridge_less_plate_count: crust
            .plate_classes
            .iter()
            .zip(&ridge_plates)
            .filter(|&(class, has_ridge)| *class == CrustClass::Oceanic && !*has_ridge)
            .count(),
        ..Default::default()
    };
    let mut total_age = 0.0_f64;
    let mut minimum_age = usize::MAX;
    for (cell, age) in cell_ages.iter_mut().enumerate() {
        let plate = partition.cell_plates[cell];
        if crust.plate_classes[plate] != CrustClass::Oceanic {
            continue;
        }
        diagnostics.oceanic_cell_count += 1;
        if age.is_none() {
            *age = Some(config.ridge_less_age);
            diagnostics.fallback_cell_count += 1;
        }
        let age = age.expect("every oceanic cell receives an age");
        diagnostics.ridge_cell_count += usize::from(age == 0 && ridge_plates[plate]);
        minimum_age = minimum_age.min(age);
        diagnostics.maximum_age = diagnostics.maximum_age.max(age);
        total_age += age as f64;
    }
    if diagnostics.oceanic_cell_count > 0 {
        diagnostics.minimum_age = minimum_age;
        diagnostics.mean_age = (total_age / diagnostics.oceanic_cell_count as f64) as f32;
    }

    Ok(SeafloorAge {
        cell_ages,
        diagnostics,
    })
}

fn validate_inputs(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    boundaries: &BoundaryClassification,
) -> Result<(), SeafloorAgeError> {
    if partition.cell_plates.len() != mesh.cell_count() {
        return Err(SeafloorAgeError::CellCountMismatch);
    }
    if crust.plate_classes.len() != partition.plate_count {
        return Err(SeafloorAgeError::PlateCountMismatch);
    }
    if !boundaries.matches_edge_count(mesh.edge_count()) {
        return Err(SeafloorAgeError::BoundaryCountMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{empty_boundaries, fingerprint, reference_partition};
    use crate::{
        CrustClassificationConfig, PlateEvolutionConfig, PlateKinematicsConfig, classify_crust,
        evolve_plate_ownership, generate_plate_kinematics,
    };

    fn final_fixture() -> (
        SphereMesh,
        PlatePartition,
        CrustClassification,
        BoundaryClassification,
    ) {
        let (mesh, initial) = reference_partition();
        let crust = classify_crust(&mesh, &initial, CrustClassificationConfig::new(17)).unwrap();
        let kinematics =
            generate_plate_kinematics(initial.plate_count, PlateKinematicsConfig::new(7)).unwrap();
        let evolution = evolve_plate_ownership(
            &mesh,
            &initial,
            &crust,
            &kinematics,
            PlateEvolutionConfig::default(),
        )
        .unwrap();
        (mesh, evolution.partition, crust, evolution.boundaries)
    }

    #[test]
    fn final_age_field_is_deterministic_and_has_stable_aggregates() {
        let (mesh, partition, crust, boundaries) = final_fixture();
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
                oceanic_cell_count: 270,
                ridge_cell_count: 147,
                ridge_plate_count: 13,
                ridge_less_plate_count: 0,
                fallback_cell_count: 0,
                minimum_age: 0,
                maximum_age: 3,
                mean_age: 0.666_666_7,
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
            Err(SeafloorAgeError::CellCountMismatch)
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
            Err(SeafloorAgeError::PlateCountMismatch)
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
            Err(SeafloorAgeError::BoundaryCountMismatch)
        );
    }
}
