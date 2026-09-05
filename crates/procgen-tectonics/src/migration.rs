use crate::{
    BoundaryClass, BoundaryClassification, CrustClass, CrustClassification, PlatePartition,
};
use procgen_sphere_mesh::SphereMesh;
use std::{cmp::Reverse, fmt};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateMigrationConfig {
    /// Minimum positive closing speed required for a convergent edge to move.
    pub minimum_convergence: f32,
}

impl Default for PlateMigrationConfig {
    fn default() -> Self {
        Self {
            minimum_convergence: 0.3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMigration {
    pub to_plate: usize,
    pub boundary_edge: usize,
    pub convergence: f32,
}

/// Updated ownership and diagnostics produced by one migration step.
#[derive(Clone, Debug, PartialEq)]
pub struct PlateMigration {
    pub partition: PlatePartition,
    /// Winning ownership change per cell. `None` means ownership was retained.
    pub cell_changes: Vec<Option<CellMigration>>,
    /// Qualifying convergent-edge proposals before conflict resolution.
    pub proposal_count: usize,
    /// Cells targeted by more than one proposal.
    pub contested_cell_count: usize,
}

impl PlateMigration {
    pub fn migrated_cell_count(&self) -> usize {
        self.cell_changes.iter().flatten().count()
    }

    pub fn maximum_convergence(&self) -> f32 {
        self.cell_changes
            .iter()
            .flatten()
            .map(|change| change.convergence)
            .max_by(f32::total_cmp)
            .unwrap_or(0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlateMigrationError {
    InvalidMinimumConvergence,
    CellCountMismatch,
    PlateCountMismatch,
    BoundaryCountMismatch,
}

impl fmt::Display for PlateMigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMinimumConvergence => {
                formatter.write_str("minimum convergence must be finite and non-negative")
            }
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

impl std::error::Error for PlateMigrationError {}

/// Computes one simultaneous boundary-migration transition.
///
/// Only sufficiently strong convergent edges propose changes. Continental
/// plates override oceanic plates; equal-crust boundaries advance whichever
/// plate has the greater local velocity toward the edge. When several edges
/// target one cell, the strongest convergence wins, followed by the lower
/// advancing plate id and edge id for deterministic ties.
///
/// `boundaries` must have been classified from `partition` before this step.
pub fn migrate_plates_once(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    boundaries: &BoundaryClassification,
    config: PlateMigrationConfig,
) -> Result<PlateMigration, PlateMigrationError> {
    if !config.minimum_convergence.is_finite() || config.minimum_convergence < 0.0 {
        return Err(PlateMigrationError::InvalidMinimumConvergence);
    }
    if partition.cell_plates.len() != mesh.cell_count() {
        return Err(PlateMigrationError::CellCountMismatch);
    }
    if crust.plate_classes.len() != partition.plate_count {
        return Err(PlateMigrationError::PlateCountMismatch);
    }
    if boundaries.validate(mesh).is_err() {
        return Err(PlateMigrationError::BoundaryCountMismatch);
    }

    let mut winners = vec![None; mesh.cell_count()];
    let mut proposal_counts = vec![0_usize; mesh.cell_count()];
    let mut proposal_count = 0;

    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        let convergence = boundaries.convergence(edge_index);
        if boundaries.edge_classes[edge_index] != BoundaryClass::Convergent
            || convergence < config.minimum_convergence
        {
            continue;
        }

        let plates = edge.cells.map(|cell| partition.cell_plates[cell]);
        let classes = plates.map(|plate| crust.plate_classes[plate]);
        let normal_speeds = boundaries.edge_normal_speeds[edge_index];
        // Continental dominates, then faster approach, then lower plate id.
        let side_key = |side: usize| {
            (
                classes[side] == CrustClass::Continental,
                normal_speeds[side],
                Reverse(plates[side]),
            )
        };
        let advancing = if side_key(0) >= side_key(1) { 0 } else { 1 };
        let retreating = 1 - advancing;
        let retreating_cell = edge.cells[retreating];
        let proposal = CellMigration {
            to_plate: plates[advancing],
            boundary_edge: edge_index,
            convergence,
        };

        proposal_count += 1;
        proposal_counts[retreating_cell] += 1;
        if winners[retreating_cell].is_none_or(|winner| proposal_precedes(proposal, winner)) {
            winners[retreating_cell] = Some(proposal);
        }
    }

    let mut migrated_partition = partition.clone();
    for (cell, change) in winners.iter().enumerate() {
        if let Some(change) = change {
            migrated_partition.cell_plates[cell] = change.to_plate;
        }
    }

    Ok(PlateMigration {
        partition: migrated_partition,
        cell_changes: winners,
        proposal_count,
        contested_cell_count: proposal_counts.iter().filter(|&&count| count > 1).count(),
    })
}

fn proposal_precedes(candidate: CellMigration, current: CellMigration) -> bool {
    let key = |proposal: CellMigration| {
        (
            proposal.convergence,
            Reverse((proposal.to_plate, proposal.boundary_edge)),
        )
    };
    key(candidate) > key(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        empty_boundaries, fingerprint, reference_partition, two_plate_boundary_partition,
    };
    use crate::{
        CrustClassificationConfig, PlateKinematicsConfig, classify_boundaries, classify_crust,
        generate_plate_kinematics,
    };

    fn fixture() -> (
        SphereMesh,
        PlatePartition,
        CrustClassification,
        BoundaryClassification,
    ) {
        let (mesh, partition) = reference_partition();
        let crust = classify_crust(&mesh, &partition, CrustClassificationConfig::new(17)).unwrap();
        let kinematics =
            generate_plate_kinematics(partition.plate_count, PlateKinematicsConfig::new(7))
                .unwrap();
        let boundaries = classify_boundaries(&mesh, &partition, &kinematics).unwrap();
        (mesh, partition, crust, boundaries)
    }

    fn two_plate_convergent_fixture() -> (
        SphereMesh,
        usize,
        PlatePartition,
        CrustClassification,
        BoundaryClassification,
    ) {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Continental, CrustClass::Oceanic],
        };
        let mut boundaries = empty_boundaries(&mesh);
        boundaries.edge_classes[edge_index] = BoundaryClass::Convergent;
        boundaries.edge_normal_speeds[edge_index] = [1.0, 0.0];
        (mesh, edge_index, partition, crust, boundaries)
    }

    #[test]
    fn one_step_is_deterministic_simultaneous_and_keeps_plate_classes() {
        let (mesh, partition, crust, boundaries) = fixture();
        let first = migrate_plates_once(
            &mesh,
            &partition,
            &crust,
            &boundaries,
            PlateMigrationConfig::default(),
        )
        .unwrap();

        assert_eq!(
            first,
            migrate_plates_once(
                &mesh,
                &partition,
                &crust,
                &boundaries,
                PlateMigrationConfig::default(),
            )
            .unwrap()
        );
        assert!(first.migrated_cell_count() > 0);
        assert!(first.contested_cell_count > 0);
        let fingerprint = fingerprint(
            first
                .partition
                .cell_plates
                .iter()
                .map(|&plate| plate as u64),
        );
        assert_eq!(fingerprint, 13_160_498_416_595_985_480);
    }

    #[test]
    fn continental_plate_overrides_oceanic_cell() {
        let (mesh, edge_index, partition, crust, boundaries) = two_plate_convergent_fixture();
        let edge = mesh.edges[edge_index];

        let migration = migrate_plates_once(
            &mesh,
            &partition,
            &crust,
            &boundaries,
            PlateMigrationConfig::default(),
        )
        .unwrap();
        assert_eq!(
            migration.cell_changes[edge.cells[1]],
            Some(CellMigration {
                to_plate: 0,
                boundary_edge: edge_index,
                convergence: 1.0,
            })
        );

        assert_eq!(migration.partition.cell_plates[edge.cells[1]], 0);
        assert_eq!(migration.migrated_cell_count(), 1);
        assert_eq!(
            crust.cell_class(&migration.partition, edge.cells[1]),
            CrustClass::Continental
        );
        assert!(crust.ocean_fraction(&mesh, &partition) > 0.0);
        assert_eq!(crust.ocean_fraction(&mesh, &migration.partition), 0.0);
    }

    #[test]
    fn minimum_convergence_suppresses_weaker_boundaries() {
        let (mesh, _, partition, crust, boundaries) = two_plate_convergent_fixture();
        let suppressed = migrate_plates_once(
            &mesh,
            &partition,
            &crust,
            &boundaries,
            PlateMigrationConfig {
                minimum_convergence: 1.1,
            },
        )
        .unwrap();
        assert_eq!(suppressed.migrated_cell_count(), 0);
        assert_eq!(suppressed.partition, partition);
    }

    #[test]
    fn equal_crust_advances_the_faster_side() {
        let (mesh, edge_index, partition, _, boundaries) = two_plate_convergent_fixture();
        let edge = mesh.edges[edge_index];
        let same_crust = CrustClassification {
            plate_classes: vec![CrustClass::Continental; 2],
        };
        let same_crust_migration = migrate_plates_once(
            &mesh,
            &partition,
            &same_crust,
            &boundaries,
            PlateMigrationConfig::default(),
        )
        .unwrap();
        assert_eq!(same_crust_migration.partition.cell_plates[edge.cells[1]], 0);
    }

    #[test]
    fn strongest_then_stable_ids_resolve_conflicts() {
        let current = CellMigration {
            to_plate: 4,
            boundary_edge: 8,
            convergence: 0.7,
        };
        assert!(proposal_precedes(
            CellMigration {
                convergence: 0.8,
                ..current
            },
            current
        ));
        assert!(proposal_precedes(
            CellMigration {
                to_plate: 3,
                boundary_edge: 20,
                ..current
            },
            current
        ));
        assert!(proposal_precedes(
            CellMigration {
                boundary_edge: 7,
                ..current
            },
            current
        ));
    }

    #[test]
    fn rejects_invalid_or_misaligned_inputs() {
        let (mesh, partition, crust, boundaries) = fixture();
        assert_eq!(
            migrate_plates_once(
                &mesh,
                &partition,
                &crust,
                &boundaries,
                PlateMigrationConfig {
                    minimum_convergence: f32::NAN,
                },
            ),
            Err(PlateMigrationError::InvalidMinimumConvergence)
        );

        let mut short_boundaries = boundaries;
        short_boundaries.edge_classes.pop();
        assert_eq!(
            migrate_plates_once(
                &mesh,
                &partition,
                &crust,
                &short_boundaries,
                PlateMigrationConfig::default(),
            ),
            Err(PlateMigrationError::BoundaryCountMismatch)
        );
    }
}
