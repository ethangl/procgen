use crate::{
    BoundaryClass, BoundaryClassification, CrustClass, CrustClassification, PlateKinematics,
    PlatePartition,
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

#[derive(Clone, Debug, PartialEq)]
pub struct PlateMigration {
    /// Plate partition after exactly one simultaneous migration step.
    pub partition: PlatePartition,
    /// Winning ownership change per cell. `None` means ownership was retained.
    pub cell_changes: Vec<Option<CellMigration>>,
    /// Number of qualifying convergent-edge proposals before conflict resolution.
    pub proposal_count: usize,
    /// Number of cells that received proposals from more than one edge.
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
            Self::PlateCountMismatch => formatter.write_str(
                "plate classes and angular velocities must match the partition plate count",
            ),
            Self::BoundaryCountMismatch => {
                formatter.write_str("boundary arrays must match the mesh edge count")
            }
        }
    }
}

impl std::error::Error for PlateMigrationError {}

/// Advances plate ownership by one simultaneous boundary-migration step.
///
/// Only sufficiently strong convergent edges propose changes. Continental
/// plates override oceanic plates; equal-crust boundaries advance whichever
/// plate has the greater local velocity toward the edge. When several edges
/// target one cell, the strongest convergence wins, followed by the lower
/// advancing plate id and edge id for deterministic ties.
pub fn migrate_plates_once(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    kinematics: &PlateKinematics,
    boundaries: &BoundaryClassification,
    config: PlateMigrationConfig,
) -> Result<PlateMigration, PlateMigrationError> {
    if !config.minimum_convergence.is_finite() || config.minimum_convergence < 0.0 {
        return Err(PlateMigrationError::InvalidMinimumConvergence);
    }
    if partition.cell_plates.len() != mesh.cell_count() {
        return Err(PlateMigrationError::CellCountMismatch);
    }
    if crust.plate_classes.len() != partition.plate_count()
        || kinematics.angular_velocities.len() != partition.plate_count()
    {
        return Err(PlateMigrationError::PlateCountMismatch);
    }
    if boundaries.edge_classes.len() != mesh.edge_count()
        || boundaries.edge_convergence.len() != mesh.edge_count()
        || boundaries.edge_shear.len() != mesh.edge_count()
    {
        return Err(PlateMigrationError::BoundaryCountMismatch);
    }

    let mut winners = vec![None; mesh.cell_count()];
    let mut proposal_counts = vec![0_usize; mesh.cell_count()];
    let mut proposal_count = 0;

    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        let convergence = boundaries.edge_convergence[edge_index];
        if boundaries.edge_classes[edge_index] != BoundaryClass::Convergent
            || convergence < config.minimum_convergence
        {
            continue;
        }

        let [cell_0, cell_1] = edge.cells;
        let plate_0 = partition.cell_plates[cell_0];
        let plate_1 = partition.cell_plates[cell_1];
        if plate_0 == plate_1 {
            continue;
        }

        let (to_plate, retreating_cell) =
            match (crust.plate_classes[plate_0], crust.plate_classes[plate_1]) {
                (CrustClass::Oceanic, CrustClass::Continental) => (plate_1, cell_0),
                (CrustClass::Continental, CrustClass::Oceanic) => (plate_0, cell_1),
                _ => {
                    let unit_position = (mesh.vertices[edge.vertices[0]]
                        + mesh.vertices[edge.vertices[1]])
                        .normalized();
                    let position = unit_position * mesh.radius;
                    let normal =
                        (mesh.cell_centers[cell_1] - mesh.cell_centers[cell_0]).normalized();
                    let push_0 = kinematics.velocity_at(plate_0, position).dot(normal);
                    let push_1 = -kinematics.velocity_at(plate_1, position).dot(normal);
                    if push_0 > push_1 || (push_0 == push_1 && plate_0 < plate_1) {
                        (plate_0, cell_1)
                    } else {
                        (plate_1, cell_0)
                    }
                }
            };
        let proposal = CellMigration {
            from_plate: partition.cell_plates[retreating_cell],
            to_plate,
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
    candidate.convergence > current.convergence
        || (candidate.convergence == current.convergence
            && (candidate.to_plate, candidate.boundary_edge)
                < (current.to_plate, current.boundary_edge))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mesh;
    use crate::{
        CrustClassificationConfig, PlateKinematicsConfig, PlatePartitionConfig,
        classify_boundaries, classify_crust, generate_plate_kinematics, partition_plates,
    };
    use procgen_core::Vec3;

    fn fixture() -> (
        SphereMesh,
        PlatePartition,
        CrustClassification,
        PlateKinematics,
        BoundaryClassification,
    ) {
        let mesh = mesh(512);
        let partition = partition_plates(
            &mesh,
            PlatePartitionConfig {
                major_plate_count: 5,
                minor_plate_count: 11,
                major_head_start_rounds: 2,
                seed: 7,
            },
        )
        .unwrap();
        let crust = classify_crust(&mesh, &partition, CrustClassificationConfig::new(17)).unwrap();
        let kinematics =
            generate_plate_kinematics(partition.plate_count(), PlateKinematicsConfig::new(7))
                .unwrap();
        let boundaries = classify_boundaries(&mesh, &partition, &kinematics).unwrap();
        (mesh, partition, crust, kinematics, boundaries)
    }

    #[test]
    fn one_step_is_deterministic_simultaneous_and_keeps_plate_classes() {
        let (mesh, partition, crust, kinematics, boundaries) = fixture();
        let first = migrate_plates_once(
            &mesh,
            &partition,
            &crust,
            &kinematics,
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
                &kinematics,
                &boundaries,
                PlateMigrationConfig::default(),
            )
            .unwrap()
        );
        assert!(first.migrated_cell_count() > 0);
        assert!(first.contested_cell_count > 0);
        assert_eq!(crust.plate_classes.len(), partition.plate_count());
        for (cell, change) in first.cell_changes.iter().enumerate() {
            if let Some(change) = change {
                assert_eq!(partition.cell_plates[cell], change.from_plate);
                assert_eq!(first.partition.cell_plates[cell], change.to_plate);
                assert_eq!(
                    crust.cell_class(&first.partition, cell),
                    crust.plate_classes[change.to_plate]
                );
            } else {
                assert_eq!(
                    first.partition.cell_plates[cell],
                    partition.cell_plates[cell]
                );
            }
        }

        let fingerprint = first
            .partition
            .cell_plates
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, &plate| {
                (hash ^ plate as u64).wrapping_mul(0x0000_0100_0000_01b3)
            });
        assert_eq!(fingerprint, 13_160_498_416_595_985_480);
    }

    #[test]
    fn continental_plate_overrides_oceanic_cell() {
        let mesh = mesh(32);
        let edge = mesh.edges[0];
        let mut cell_plates = vec![1; mesh.cell_count()];
        cell_plates[edge.cells[0]] = 0;
        let partition = PlatePartition {
            cell_plates,
            plate_seeds: vec![edge.cells[0], edge.cells[1]],
            major_plate_count: 2,
        };
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Oceanic, CrustClass::Continental],
        };
        let unit_position =
            (mesh.vertices[edge.vertices[0]] + mesh.vertices[edge.vertices[1]]).normalized();
        let normal =
            (mesh.cell_centers[edge.cells[1]] - mesh.cell_centers[edge.cells[0]]).normalized();
        let kinematics = PlateKinematics {
            angular_velocities: vec![unit_position.cross(normal), Vec3::ZERO],
        };
        let mut boundaries = BoundaryClassification {
            edge_classes: vec![BoundaryClass::Interior; mesh.edge_count()],
            edge_convergence: vec![0.0; mesh.edge_count()],
            edge_shear: vec![0.0; mesh.edge_count()],
        };
        boundaries.edge_classes[0] = BoundaryClass::Convergent;
        boundaries.edge_convergence[0] = 1.0;

        let migration = migrate_plates_once(
            &mesh,
            &partition,
            &crust,
            &kinematics,
            &boundaries,
            PlateMigrationConfig::default(),
        )
        .unwrap();

        assert_eq!(migration.partition.cell_plates[edge.cells[0]], 1);
        assert_eq!(migration.migrated_cell_count(), 1);
        assert_eq!(
            crust.cell_class(&migration.partition, edge.cells[0]),
            CrustClass::Continental
        );
        assert!(crust.ocean_fraction(&mesh, &partition) > 0.0);
        assert_eq!(crust.ocean_fraction(&mesh, &migration.partition), 0.0);

        let suppressed = migrate_plates_once(
            &mesh,
            &partition,
            &crust,
            &kinematics,
            &boundaries,
            PlateMigrationConfig {
                minimum_convergence: 1.1,
            },
        )
        .unwrap();
        assert_eq!(suppressed.migrated_cell_count(), 0);

        let same_crust = CrustClassification {
            plate_classes: vec![CrustClass::Continental; 2],
        };
        let same_crust_migration = migrate_plates_once(
            &mesh,
            &partition,
            &same_crust,
            &kinematics,
            &boundaries,
            PlateMigrationConfig::default(),
        )
        .unwrap();
        assert_eq!(same_crust_migration.partition.cell_plates[edge.cells[1]], 0);
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
        let (mesh, partition, crust, kinematics, boundaries) = fixture();
        assert_eq!(
            migrate_plates_once(
                &mesh,
                &partition,
                &crust,
                &kinematics,
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
                &kinematics,
                &short_boundaries,
                PlateMigrationConfig::default(),
            ),
            Err(PlateMigrationError::BoundaryCountMismatch)
        );
    }
}
