use crate::{
    BoundaryClassification, BoundaryClassificationError, CrustClassification, PlateKinematics,
    PlateMigration, PlateMigrationConfig, PlateMigrationError, PlatePartition, classify_boundaries,
    migrate_plates_once,
};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateEvolutionConfig {
    /// Number of complete boundary-classification and migration transitions.
    pub step_count: usize,
    pub migration: PlateMigrationConfig,
}

impl Default for PlateEvolutionConfig {
    fn default() -> Self {
        Self {
            step_count: 5,
            migration: PlateMigrationConfig::default(),
        }
    }
}

/// Totals accumulated without retaining per-step or per-cell history.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlateEvolutionDiagnostics {
    /// Steps that produced at least one ownership change.
    pub active_step_count: usize,
    /// Qualifying convergent-edge proposals across all steps.
    pub proposal_count: usize,
    /// Contested-cell events across all steps. A cell can contribute once per step.
    pub contested_cell_count: usize,
    /// Ownership-change events across all steps. A cell can migrate more than once.
    pub migrated_cell_count: usize,
    pub maximum_convergence: f32,
}

impl PlateEvolutionDiagnostics {
    fn record_step(&mut self, step: &PlateMigration) {
        let migrated_cell_count = step.migrated_cell_count();
        self.active_step_count += usize::from(migrated_cell_count > 0);
        self.proposal_count += step.proposal_count;
        self.contested_cell_count += step.contested_cell_count;
        self.migrated_cell_count += migrated_cell_count;
        self.maximum_convergence = self.maximum_convergence.max(step.maximum_convergence());
    }
}

/// Final ownership and boundary state after deterministic eager evolution.
#[derive(Clone, Debug, PartialEq)]
pub struct PlateEvolution {
    pub partition: PlatePartition,
    pub boundaries: BoundaryClassification,
    pub diagnostics: PlateEvolutionDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlateEvolutionError {
    Boundary(BoundaryClassificationError),
    Migration(PlateMigrationError),
}

impl fmt::Display for PlateEvolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boundary(error) => error.fmt(formatter),
            Self::Migration(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PlateEvolutionError {}

impl From<BoundaryClassificationError> for PlateEvolutionError {
    fn from(error: BoundaryClassificationError) -> Self {
        Self::Boundary(error)
    }
}

impl From<PlateMigrationError> for PlateEvolutionError {
    fn from(error: PlateMigrationError) -> Self {
        Self::Migration(error)
    }
}

/// Repeatedly classifies current boundaries and eagerly applies one simultaneous
/// migration transition. Plate crust classes are read-only; callers derive a
/// cell's current crust class from the returned ownership.
pub fn evolve_plate_ownership(
    mesh: &SphereMesh,
    initial_partition: &PlatePartition,
    crust: &CrustClassification,
    kinematics: &PlateKinematics,
    config: PlateEvolutionConfig,
) -> Result<PlateEvolution, PlateEvolutionError> {
    let mut partition = initial_partition.clone();
    let mut boundaries = classify_boundaries(mesh, &partition, kinematics)?;
    let mut diagnostics = PlateEvolutionDiagnostics::default();

    for _ in 0..config.step_count {
        let migration =
            migrate_plates_once(mesh, &partition, crust, &boundaries, config.migration)?;
        diagnostics.record_step(&migration);
        partition = migration.partition;
        boundaries = classify_boundaries(mesh, &partition, kinematics)?;
    }

    Ok(PlateEvolution {
        partition,
        boundaries,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::reference_partition;
    use crate::{
        CrustClassificationConfig, PlateKinematicsConfig, classify_crust, generate_plate_kinematics,
    };

    fn fixture() -> (
        SphereMesh,
        PlatePartition,
        CrustClassification,
        PlateKinematics,
    ) {
        let (mesh, partition) = reference_partition();
        let crust = classify_crust(&mesh, &partition, CrustClassificationConfig::new(17)).unwrap();
        let kinematics =
            generate_plate_kinematics(partition.plate_count, PlateKinematicsConfig::new(7))
                .unwrap();
        (mesh, partition, crust, kinematics)
    }

    #[test]
    fn multi_step_evolution_is_deterministic_and_has_stable_aggregates() {
        let (mesh, partition, crust, kinematics) = fixture();
        let config = PlateEvolutionConfig {
            step_count: 5,
            migration: PlateMigrationConfig::default(),
        };
        let first = evolve_plate_ownership(&mesh, &partition, &crust, &kinematics, config).unwrap();

        assert_eq!(
            first,
            evolve_plate_ownership(&mesh, &partition, &crust, &kinematics, config).unwrap()
        );
        assert_eq!(first.diagnostics.active_step_count, config.step_count);
        assert!(first.diagnostics.migrated_cell_count > 0);
        assert!(first.diagnostics.proposal_count >= first.diagnostics.migrated_cell_count);

        let fingerprint = first
            .partition
            .cell_plates
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, &plate| {
                (hash ^ plate as u64).wrapping_mul(0x0000_0100_0000_01b3)
            });
        assert_eq!(fingerprint, 9_637_389_478_425_232_066);
        assert_eq!(
            first.diagnostics,
            PlateEvolutionDiagnostics {
                active_step_count: 5,
                proposal_count: 601,
                contested_cell_count: 101,
                migrated_cell_count: 487,
                maximum_convergence: 1.600_926_4,
            }
        );
    }

    #[test]
    fn zero_steps_returns_initial_ownership_and_current_boundaries() {
        let (mesh, partition, crust, kinematics) = fixture();
        let evolution = evolve_plate_ownership(
            &mesh,
            &partition,
            &crust,
            &kinematics,
            PlateEvolutionConfig {
                step_count: 0,
                migration: PlateMigrationConfig::default(),
            },
        )
        .unwrap();

        assert_eq!(evolution.partition, partition);
        assert_eq!(evolution.diagnostics, PlateEvolutionDiagnostics::default());
        assert_eq!(
            evolution.boundaries,
            classify_boundaries(&mesh, &partition, &kinematics).unwrap()
        );
    }

    #[test]
    fn one_step_matches_the_single_transition_primitive() {
        let (mesh, partition, crust, kinematics) = fixture();
        let boundaries = classify_boundaries(&mesh, &partition, &kinematics).unwrap();
        let migration = migrate_plates_once(
            &mesh,
            &partition,
            &crust,
            &boundaries,
            PlateMigrationConfig::default(),
        )
        .unwrap();
        let evolution = evolve_plate_ownership(
            &mesh,
            &partition,
            &crust,
            &kinematics,
            PlateEvolutionConfig {
                step_count: 1,
                migration: PlateMigrationConfig::default(),
            },
        )
        .unwrap();

        assert_eq!(evolution.partition, migration.partition);
        assert_eq!(
            evolution.boundaries,
            classify_boundaries(&mesh, &migration.partition, &kinematics).unwrap()
        );
        assert_eq!(
            evolution.diagnostics.migrated_cell_count,
            migration.migrated_cell_count()
        );
    }

    #[test]
    fn plate_classes_stay_fixed_while_cell_crust_follows_final_ownership() {
        let (mesh, partition, crust, kinematics) = fixture();
        let original_classes = crust.plate_classes.clone();
        let evolution = evolve_plate_ownership(
            &mesh,
            &partition,
            &crust,
            &kinematics,
            PlateEvolutionConfig::default(),
        )
        .unwrap();

        assert_eq!(crust.plate_classes, original_classes);
        for (cell, &plate) in evolution.partition.cell_plates.iter().enumerate() {
            assert_eq!(
                crust.cell_class(&evolution.partition, cell),
                original_classes[plate]
            );
        }
    }

    #[test]
    fn configured_steps_run_even_when_migration_is_quiescent() {
        let (mesh, partition, crust, kinematics) = fixture();
        let evolution = evolve_plate_ownership(
            &mesh,
            &partition,
            &crust,
            &kinematics,
            PlateEvolutionConfig {
                step_count: 4,
                migration: PlateMigrationConfig {
                    minimum_convergence: f32::MAX,
                },
            },
        )
        .unwrap();

        assert_eq!(evolution.partition, partition);
        assert_eq!(evolution.diagnostics, PlateEvolutionDiagnostics::default());
    }
}
