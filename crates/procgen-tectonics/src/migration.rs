use crate::{
    BoundaryClass, BoundaryClassification, CrustClass, CrustClassification, PlatePartition,
};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

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
    pub from_plate: usize,
    pub to_plate: usize,
    pub boundary_edge: usize,
    pub convergence: f32,
}

/// Transition record for one migration step.
///
/// Apply it to the originating partition with [`Self::apply`].
#[derive(Clone, Debug, PartialEq)]
pub struct PlateMigration {
    /// Winning ownership change per cell. `None` means ownership was retained.
    pub cell_changes: Vec<Option<CellMigration>>,
    /// Qualifying convergent-edge proposal count per target cell.
    ///
    /// This dense diagnostic supports aggregate statistics and future
    /// per-cell visualization of contested migration.
    pub cell_proposal_counts: Vec<usize>,
}

impl PlateMigration {
    /// Applies this transition to the partition it was computed from.
    pub fn apply(&self, partition: &mut PlatePartition) {
        for (cell, change) in self.cell_changes.iter().enumerate() {
            if let Some(change) = change {
                partition.cell_plates[cell] = change.to_plate;
            }
        }
    }

    pub fn migrated_cell_count(&self) -> usize {
        self.cell_changes.iter().flatten().count()
    }

    pub fn proposal_count(&self) -> usize {
        self.cell_proposal_counts.iter().sum()
    }

    pub fn contested_cell_count(&self) -> usize {
        self.cell_proposal_counts
            .iter()
            .filter(|&&count| count > 1)
            .count()
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
    if !boundaries.matches_edge_count(mesh.edge_count()) {
        return Err(PlateMigrationError::BoundaryCountMismatch);
    }

    let mut winners = vec![None; mesh.cell_count()];
    let mut proposal_counts = vec![0_usize; mesh.cell_count()];

    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        let convergence = boundaries.convergence(edge_index);
        if boundaries.edge_classes[edge_index] != BoundaryClass::Convergent
            || convergence < config.minimum_convergence
        {
            continue;
        }

        let plates = edge.cells.map(|cell| partition.cell_plates[cell]);
        let [class_0, class_1] = plates.map(|plate| crust.plate_classes[plate]);
        let [normal_speed_0, normal_speed_1] = boundaries.edge_normal_speeds[edge_index];
        let advancing = match (class_0, class_1) {
            (CrustClass::Oceanic, CrustClass::Continental) => 1,
            (CrustClass::Continental, CrustClass::Oceanic) => 0,
            _ if normal_speed_0 > normal_speed_1
                || (normal_speed_0 == normal_speed_1 && plates[0] < plates[1]) =>
            {
                0
            }
            _ => 1,
        };
        let retreating = 1 - advancing;
        let retreating_cell = edge.cells[retreating];
        let proposal = CellMigration {
            from_plate: plates[retreating],
            to_plate: plates[advancing],
            boundary_edge: edge_index,
            convergence,
        };

        proposal_counts[retreating_cell] += 1;
        if winners[retreating_cell].is_none_or(|winner| proposal_precedes(proposal, winner)) {
            winners[retreating_cell] = Some(proposal);
        }
    }

    Ok(PlateMigration {
        cell_changes: winners,
        cell_proposal_counts: proposal_counts,
    })
}

fn proposal_precedes(candidate: CellMigration, current: CellMigration) -> bool {
    candidate.convergence > current.convergence
        || (candidate.convergence == current.convergence
            && (candidate.to_plate, candidate.boundary_edge)
                < (current.to_plate, current.boundary_edge))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{reference_partition, two_plate_boundary_partition};
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
        assert!(first.contested_cell_count() > 0);
        let mut migrated_partition = partition.clone();
        first.apply(&mut migrated_partition);
        let fingerprint = migrated_partition
            .cell_plates
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, &plate| {
                (hash ^ plate as u64).wrapping_mul(0x0000_0100_0000_01b3)
            });
        assert_eq!(fingerprint, 13_160_498_416_595_985_480);
    }

    #[test]
    fn continental_plate_overrides_oceanic_cell() {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Continental, CrustClass::Oceanic],
        };
        let mut boundaries = BoundaryClassification {
            edge_classes: vec![BoundaryClass::Interior; mesh.edge_count()],
            edge_normal_speeds: vec![[0.0; 2]; mesh.edge_count()],
            edge_shear: vec![0.0; mesh.edge_count()],
        };
        boundaries.edge_classes[edge_index] = BoundaryClass::Convergent;
        boundaries.edge_normal_speeds[edge_index] = [1.0, 0.0];

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
                from_plate: 1,
                to_plate: 0,
                boundary_edge: edge_index,
                convergence: 1.0,
            })
        );
        let mut migrated_partition = partition.clone();
        migration.apply(&mut migrated_partition);

        assert_eq!(migrated_partition.cell_plates[edge.cells[1]], 0);
        assert_eq!(migration.migrated_cell_count(), 1);
        assert_eq!(
            crust.cell_class(&migrated_partition, edge.cells[1]),
            CrustClass::Continental
        );
        assert!(crust.ocean_fraction(&mesh, &partition) > 0.0);
        assert_eq!(crust.ocean_fraction(&mesh, &migrated_partition), 0.0);

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
        let mut suppressed_partition = partition.clone();
        suppressed.apply(&mut suppressed_partition);
        assert_eq!(suppressed_partition, partition);

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
        let mut same_crust_partition = partition.clone();
        same_crust_migration.apply(&mut same_crust_partition);
        assert_eq!(same_crust_partition.cell_plates[edge.cells[1]], 0);
    }

    #[test]
    fn strongest_then_stable_ids_resolve_conflicts() {
        let current = CellMigration {
            from_plate: 0,
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
