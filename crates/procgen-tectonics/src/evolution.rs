//! Deterministic multi-step plate evolution.
//!
//! A step classifies the boundaries of the current ownership and then moves
//! two kinds of distance. Convergent edges accumulate what they close, and an
//! edge that has closed a whole cell width moves one cell across itself: the
//! retreating cell joins the advancing plate and takes the advancing cell's
//! crust with it, because the overriding plate's material now covers it. Every
//! cell separately accumulates the distance it has travelled, and a cell that
//! has travelled a whole cell width pulls the crust of the same-plate
//! neighbour behind it. A cell with no neighbour behind it sits on the plate's
//! trailing edge, and if the boundary there is a ridge the plate has opened
//! and the cell is reborn as crust made this step.
//!
//! Four things therefore survive a step: ownership, each cell's birth step,
//! and the two debts. Crust class is not one of them. It is read from birth,
//! so a continental plate that rifts grows an oceanic margin and an overridden
//! cell takes the overriding material's class without anything storing a
//! second answer to the question.

use crate::{
    BoundaryClass, BoundaryClassification, BoundaryClassificationError, CellCrust, CrustBirthPrior,
    CrustClass, CrustClassification, PlateKinematics, PlateMigration, PlateMigrationConfig,
    PlateMigrationError, PlatePartition, StageInputError, classify_boundaries,
    field::mean_cell_width, migrate_plates_once, migration::accumulate_closing_distances,
};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

/// Model time per step at the viewer's default mesh: the unit sphere's cell
/// width `sqrt(4 pi / 65_536)` is 0.0138, and a plate at the default maximum
/// angular speed of 1.0 covers unit distance per unit time, so one cell width
/// takes that long. Rounded to a round number, because nothing downstream
/// resolves the difference.
pub const DEFAULT_STEP_DURATION: f32 = 0.014;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateEvolutionConfig {
    /// Number of complete boundary-classification, migration, and advection
    /// transitions.
    pub step_count: usize,
    /// Model time advanced per step. Every displacement is a speed times this
    /// duration measured against the mesh's cell width, so a finer mesh has a
    /// smaller cell width and moves more cells per step for the same motion,
    /// which is what a fixed model time per step should do. Zero freezes the
    /// world. See [`DEFAULT_STEP_DURATION`] for where the default sits.
    pub step_duration: f32,
    pub migration: PlateMigrationConfig,
}

impl Default for PlateEvolutionConfig {
    fn default() -> Self {
        Self {
            step_count: 5,
            step_duration: DEFAULT_STEP_DURATION,
            migration: PlateMigrationConfig::default(),
        }
    }
}

/// Totals accumulated without retaining per-step history.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlateEvolutionDiagnostics {
    /// Steps that changed ownership or created crust.
    pub active_step_count: usize,
    /// Qualifying convergent-edge proposals across all steps.
    pub proposal_count: usize,
    /// Contested-cell events across all steps. A cell can contribute once per step.
    pub contested_cell_count: usize,
    /// Ownership-change events across all steps. A cell can migrate more than once.
    pub migrated_cell_count: usize,
    /// Crust-creation events at trailing divergent boundaries across all
    /// steps. A cell can be reborn more than once.
    pub born_cell_count: usize,
    pub maximum_convergence: f32,
}

impl PlateEvolutionDiagnostics {
    /// Records one step's migration and returns how many cells it moved.
    fn record_migration(&mut self, migration: &PlateMigration) -> usize {
        let migrated_cell_count = migration.migrated_cell_count();
        self.proposal_count += migration.proposal_count;
        self.contested_cell_count += migration.contested_cell_count;
        self.migrated_cell_count += migrated_cell_count;
        self.maximum_convergence = self
            .maximum_convergence
            .max(migration.maximum_convergence());
        migrated_cell_count
    }
}

/// Everything one evolution run reads that it does not produce.
#[derive(Clone, Copy, Debug)]
pub struct PlateEvolutionInputs<'a> {
    pub partition: &'a PlatePartition,
    pub crust: &'a CrustClassification,
    pub kinematics: &'a PlateKinematics,
    /// Boundaries classified from `partition`, which the first step reads.
    pub boundaries: &'a BoundaryClassification,
    /// Crust birth from before step zero.
    pub birth_prior: &'a CrustBirthPrior,
}

/// Final ownership, boundary state, and crust birth after deterministic
/// evolution.
#[derive(Clone, Debug, PartialEq)]
pub struct PlateEvolution {
    pub partition: PlatePartition,
    pub boundaries: BoundaryClassification,
    /// Step at which each cell's crust was created. Negative steps come from
    /// the prior for crust that predates step zero; `None` is original
    /// continental crust that evolution never re-made.
    pub cell_birth: Vec<Option<i32>>,
    pub diagnostics: PlateEvolutionDiagnostics,
}

impl PlateEvolution {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), StageInputError> {
        self.partition.validate(mesh)?;
        self.boundaries.validate(mesh)?;
        self.cell_crust().validate(mesh)
    }

    /// The one per-cell answer to what crust a cell carries after evolution.
    pub fn cell_crust(&self) -> CellCrust<'_> {
        CellCrust {
            cell_birth: &self.cell_birth,
        }
    }

    pub fn cell_class(&self, cell: usize) -> CrustClass {
        self.cell_crust().class(cell)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlateEvolutionError {
    InvalidStepDuration,
    InvalidMinimumConvergence,
    Input(StageInputError),
    Boundary(BoundaryClassificationError),
    Migration(PlateMigrationError),
}

impl fmt::Display for PlateEvolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidStepDuration => {
                formatter.write_str("step duration must be finite and non-negative")
            }
            Self::InvalidMinimumConvergence => {
                formatter.write_str("minimum convergence must be finite and non-negative")
            }
            Self::Input(error) => error.fmt(formatter),
            Self::Boundary(error) => error.fmt(formatter),
            Self::Migration(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PlateEvolutionError {}

impl From<StageInputError> for PlateEvolutionError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
    }
}

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

/// Repeatedly classifies current boundaries, applies one simultaneous
/// migration transition, and advects crust one cell width at a time.
///
/// Plate crust classes are read-only and describe plates; the returned
/// [`PlateEvolution::cell_crust`] is what a cell carries.
pub fn evolve_plate_ownership(
    mesh: &SphereMesh,
    inputs: PlateEvolutionInputs<'_>,
    config: PlateEvolutionConfig,
) -> Result<PlateEvolution, PlateEvolutionError> {
    if !config.step_duration.is_finite() || config.step_duration < 0.0 {
        return Err(PlateEvolutionError::InvalidStepDuration);
    }
    if !config.migration.minimum_convergence.is_finite()
        || config.migration.minimum_convergence < 0.0
    {
        return Err(PlateEvolutionError::InvalidMinimumConvergence);
    }
    inputs.partition.validate(mesh)?;
    inputs.crust.validate(inputs.partition)?;
    inputs.kinematics.validate(inputs.partition)?;
    inputs.boundaries.validate(mesh)?;
    inputs.birth_prior.validate(mesh)?;

    let mut world = EvolvingWorld {
        mesh,
        crust: inputs.crust,
        kinematics: inputs.kinematics,
        config,
        cell_width: mean_cell_width(mesh),
        partition: inputs.partition.clone(),
        cell_birth: inputs.birth_prior.cell_birth.clone(),
        edge_closing: vec![0.0; mesh.edge_count()],
        cell_travel: vec![0.0; mesh.cell_count()],
    };
    let mut boundaries = inputs.boundaries.clone();
    let mut diagnostics = PlateEvolutionDiagnostics::default();

    for step in 0..config.step_count {
        let migrated_cell_count = diagnostics.record_migration(&world.migrate(&boundaries)?);
        let born_cell_count = world.advect(&boundaries, step as i32);
        diagnostics.born_cell_count += born_cell_count;
        diagnostics.active_step_count +=
            usize::from(migrated_cell_count > 0 || born_cell_count > 0);
        boundaries = classify_boundaries(mesh, &world.partition, inputs.kinematics)?;
    }

    Ok(PlateEvolution {
        partition: world.partition,
        boundaries,
        cell_birth: world.cell_birth,
        diagnostics,
    })
}

/// The state one step advances, together with the inputs every step reads.
///
/// Ownership, birth, and the two debts move together once per step and nothing
/// outside evolution owns a step, so holding them in one place keeps the
/// per-step solves in `migration.rs` pure functions of the current state
/// rather than functions that also return four vectors.
struct EvolvingWorld<'a> {
    mesh: &'a SphereMesh,
    crust: &'a CrustClassification,
    kinematics: &'a PlateKinematics,
    config: PlateEvolutionConfig,
    /// The one distance every accumulated displacement is measured against.
    cell_width: f32,
    partition: PlatePartition,
    cell_birth: Vec<Option<i32>>,
    /// Closing distance accumulated per boundary edge, in model units.
    edge_closing: Vec<f32>,
    /// Distance travelled per cell since its last pull, in model units.
    cell_travel: Vec<f32>,
}

impl EvolvingWorld<'_> {
    /// Advances every edge's closing debt and applies the migrations the debts
    /// have paid for. A migrating cell takes the advancing cell's crust across
    /// the edge, and the winning edge's debt drops by one cell width.
    fn migrate(
        &mut self,
        boundaries: &BoundaryClassification,
    ) -> Result<PlateMigration, PlateMigrationError> {
        accumulate_closing_distances(
            boundaries,
            &mut self.edge_closing,
            self.config.migration,
            self.config.step_duration,
        );
        let migration = migrate_plates_once(
            self.mesh,
            &self.partition,
            self.crust,
            boundaries,
            &self.edge_closing,
            self.cell_width,
        )?;

        let previous_birth = self.cell_birth.clone();
        for (cell, change) in migration.cell_changes.iter().enumerate() {
            let Some(change) = change else { continue };
            let cells = self.mesh.edges[change.boundary_edge].cells;
            let advancing = if cells[0] == cell { cells[1] } else { cells[0] };
            self.cell_birth[cell] = previous_birth[advancing];
            self.edge_closing[change.boundary_edge] -= self.cell_width;
        }
        self.partition.clone_from(&migration.partition);
        Ok(migration)
    }

    /// Advances every cell's travel debt and pulls crust upstream by one cell
    /// for the cells that have paid for it.
    ///
    /// A cell's upstream neighbour is the same-plate neighbour lying most
    /// nearly opposite its velocity. A cell with none is at the plate's
    /// trailing edge: if the boundary behind it is a ridge it is new crust
    /// born this step, and otherwise it keeps the crust it has. Every pull
    /// reads the field as it stood before this substep, so the update is
    /// simultaneous.
    fn advect(&mut self, boundaries: &BoundaryClassification, step: i32) -> usize {
        let mesh = self.mesh;
        let previous_birth = self.cell_birth.clone();
        let mut born_cell_count = 0;

        for cell in 0..mesh.cell_count() {
            let plate = self.partition.cell_plates[cell];
            let center = mesh.cell_centers[cell];
            let velocity = self.kinematics.velocity_at(plate, center);
            self.cell_travel[cell] += velocity.length() * self.config.step_duration;
            if self.cell_travel[cell] < self.cell_width {
                continue;
            }
            self.cell_travel[cell] -= self.cell_width;

            let mut upstream: Option<(f32, usize)> = None;
            let mut behind: Option<(f32, usize)> = None;
            for corner in mesh.cell_corners(cell) {
                let opposition = (mesh.cell_centers[corner.neighbor] - center).dot(-velocity);
                if opposition <= 0.0 {
                    continue;
                }
                if behind.is_none_or(|(best, _)| opposition > best) {
                    behind = Some((opposition, corner.edge));
                }
                if self.partition.cell_plates[corner.neighbor] == plate
                    && upstream.is_none_or(|(best, _)| opposition > best)
                {
                    upstream = Some((opposition, corner.neighbor));
                }
            }

            match upstream {
                Some((_, neighbor)) => self.cell_birth[cell] = previous_birth[neighbor],
                None => {
                    let opening = behind.is_some_and(|(_, edge)| {
                        boundaries.edge_classes[edge] == BoundaryClass::Divergent
                    });
                    if opening {
                        self.cell_birth[cell] = Some(step);
                        born_cell_count += 1;
                    }
                }
            }
        }
        born_cell_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        EvolutionFixture, empty_boundaries, evolution_fixture, fingerprint,
        reference_evolution_config, two_plate_boundary_partition,
    };
    use crate::{CrustBirthPriorConfig, derive_crust_birth_prior};
    use procgen_core::Vec3;

    fn ownership_fingerprint(evolution: &PlateEvolution) -> u64 {
        fingerprint(
            evolution
                .partition
                .cell_plates
                .iter()
                .map(|&plate| plate as u64),
        )
    }

    fn birth_fingerprint(evolution: &PlateEvolution) -> u64 {
        fingerprint(
            evolution
                .cell_birth
                .iter()
                .map(|birth| birth.map_or(u64::MAX, |birth| birth as u32 as u64)),
        )
    }

    /// Rigid rotations that carry the two cells of `edge` along their own
    /// separation direction at unit speed: `outward` of one pulls them apart
    /// and minus one pushes them together. `omega = p x d` is the rotation
    /// whose velocity at `p` is the tangential part of `d`.
    fn opposed_kinematics(mesh: &SphereMesh, edge: usize, outward: f32) -> PlateKinematics {
        let [first, second] = mesh.edges[edge].cells.map(|cell| mesh.cell_centers[cell]);
        let apart = (second - first).normalized() * outward;
        PlateKinematics {
            angular_velocities: vec![first.cross(-apart), second.cross(apart)],
        }
    }

    /// A one-cell plate inside a larger one, moving apart from or into it.
    fn two_plate_fixture(outward: f32, plate_classes: Vec<CrustClass>) -> EvolutionFixture {
        let (mesh, edge, partition) = two_plate_boundary_partition();
        let crust = CrustClassification { plate_classes };
        let kinematics = opposed_kinematics(&mesh, edge, outward);
        let boundaries = classify_boundaries(&mesh, &partition, &kinematics).unwrap();
        let birth_prior = derive_crust_birth_prior(
            &mesh,
            &partition,
            &crust,
            &boundaries,
            CrustBirthPriorConfig::default(),
        )
        .unwrap();
        EvolutionFixture {
            mesh,
            partition,
            crust,
            kinematics,
            boundaries,
            birth_prior,
        }
    }

    #[test]
    fn multi_step_evolution_is_deterministic_and_has_stable_aggregates() {
        let fixture = evolution_fixture();
        let config = reference_evolution_config();
        let first = fixture.evolve(config);

        assert_eq!(first, fixture.evolve(config));
        first.validate(&fixture.mesh).unwrap();
        assert_eq!(first.diagnostics.active_step_count, config.step_count);
        assert!(first.diagnostics.proposal_count >= first.diagnostics.migrated_cell_count);
        assert_eq!(first.diagnostics.proposal_count, 174);
        assert_eq!(first.diagnostics.contested_cell_count, 27);
        assert_eq!(first.diagnostics.migrated_cell_count, 145);
        assert_eq!(first.diagnostics.born_cell_count, 51);
        // Convergence is a float reduction, so machines differ in the last bits.
        assert!((first.diagnostics.maximum_convergence - 1.579_952_7).abs() < 1.0e-3);
        assert_eq!(ownership_fingerprint(&first), 3_267_391_620_510_768_872);
        assert_eq!(birth_fingerprint(&first), 8_599_187_947_100_659_055);
    }

    #[test]
    fn zero_steps_returns_the_initial_state_and_the_prior() {
        let fixture = evolution_fixture();
        let evolution = fixture.evolve(PlateEvolutionConfig {
            step_count: 0,
            ..reference_evolution_config()
        });

        assert_eq!(evolution.partition, fixture.partition);
        assert_eq!(evolution.boundaries, fixture.boundaries);
        assert_eq!(evolution.cell_birth, fixture.birth_prior.cell_birth);
        assert_eq!(evolution.diagnostics, PlateEvolutionDiagnostics::default());
    }

    #[test]
    fn a_zero_length_step_freezes_the_world() {
        let fixture = evolution_fixture();
        let evolution = fixture.evolve(PlateEvolutionConfig {
            step_duration: 0.0,
            ..reference_evolution_config()
        });

        assert_eq!(evolution.partition, fixture.partition);
        assert_eq!(evolution.cell_birth, fixture.birth_prior.cell_birth);
        assert_eq!(evolution.diagnostics, PlateEvolutionDiagnostics::default());
    }

    /// A step long enough to travel one cell width on the 32-cell mesh, with
    /// migration off: the one-cell plate this fixture rifts from would
    /// otherwise be swallowed by its own convergent edge before it opened
    /// anything.
    fn rift_config(step_count: usize) -> PlateEvolutionConfig {
        PlateEvolutionConfig {
            step_count,
            step_duration: 0.7,
            migration: PlateMigrationConfig {
                minimum_convergence: f32::MAX,
            },
        }
    }

    #[test]
    fn a_rifting_plate_grows_oceanic_crust_along_its_trailing_edge() {
        let fixture = two_plate_fixture(1.0, vec![CrustClass::Continental; 2]);
        let config = rift_config(4);
        let evolution = fixture.evolve(config);

        assert!(fixture.birth_prior.cell_birth.iter().all(Option::is_none));
        assert!(evolution.diagnostics.born_cell_count > 0);
        let born: Vec<_> = (0..fixture.mesh.cell_count())
            .filter(|&cell| evolution.cell_birth[cell].is_some())
            .collect();
        assert!(!born.is_empty());
        for &cell in &born {
            let birth = evolution.cell_birth[cell].unwrap();
            assert!(
                (0..config.step_count as i32).contains(&birth),
                "cell {cell} was born during the run"
            );
            assert_eq!(evolution.cell_class(cell), CrustClass::Oceanic);
            assert!(
                fixture.mesh.cell_corners(cell).iter().any(|corner| {
                    evolution.partition.cell_plates[corner.neighbor]
                        != evolution.partition.cell_plates[cell]
                }),
                "cell {cell} sits on a plate boundary"
            );
        }
    }

    /// The first step at which a convergent edge closing at `convergence` has
    /// paid for a whole cell width.
    fn paying_step(cell_width: f32, convergence: f32, step_duration: f32) -> usize {
        (1..)
            .find(|steps| *steps as f32 * convergence * step_duration >= cell_width)
            .unwrap()
    }

    fn strongest_convergence(fixture: &EvolutionFixture) -> f32 {
        (0..fixture.mesh.edge_count())
            .map(|edge| fixture.boundaries.convergence(edge))
            .fold(f32::MIN, f32::max)
    }

    fn single_edge_convergent_fixture() -> (EvolutionFixture, PlateEvolutionConfig, usize) {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental, CrustClass::Oceanic]);
        let convergence = strongest_convergence(&fixture);
        // Only the strongest edge clears the minimum, so exactly one cell can
        // ever migrate and its debt is the whole schedule.
        let config = PlateEvolutionConfig {
            step_count: 0,
            step_duration: 0.1,
            migration: PlateMigrationConfig {
                minimum_convergence: convergence * 0.999,
            },
        };
        let qualifying = (0..fixture.mesh.edge_count())
            .filter(|&edge| fixture.boundaries.convergence(edge) >= convergence * 0.999)
            .count();
        assert_eq!(qualifying, 1);
        let steps = paying_step(
            crate::field::mean_cell_width(&fixture.mesh),
            convergence,
            config.step_duration,
        );
        assert!(steps > 1, "the debt must take more than one step to pay");
        (fixture, config, steps)
    }

    #[test]
    fn a_convergent_edge_migrates_at_the_step_its_debt_predicts() {
        let (fixture, config, steps) = single_edge_convergent_fixture();

        let waiting = fixture.evolve(PlateEvolutionConfig {
            step_count: steps - 1,
            ..config
        });
        assert_eq!(waiting.diagnostics.migrated_cell_count, 0);
        assert_eq!(waiting.partition, fixture.partition);

        let moved = fixture.evolve(PlateEvolutionConfig {
            step_count: steps,
            ..config
        });
        assert_eq!(moved.diagnostics.migrated_cell_count, 1);
    }

    #[test]
    fn a_migrating_cell_takes_the_advancing_cells_crust() {
        let (fixture, config, steps) = single_edge_convergent_fixture();
        let before = fixture.evolve(PlateEvolutionConfig {
            step_count: steps - 1,
            ..config
        });
        let after = fixture.evolve(PlateEvolutionConfig {
            step_count: steps,
            ..config
        });

        // Nothing travelled a whole cell width in these steps, so migration is
        // the only thing that can have changed the birth field.
        assert_eq!(after.diagnostics.born_cell_count, 0);
        let migrated = (0..fixture.mesh.cell_count())
            .find(|&cell| after.partition.cell_plates[cell] != before.partition.cell_plates[cell])
            .unwrap();
        let advancing = fixture
            .mesh
            .cell_corners(migrated)
            .iter()
            .map(|corner| corner.neighbor)
            .find(|&neighbor| {
                before.partition.cell_plates[neighbor] == after.partition.cell_plates[migrated]
            })
            .unwrap();

        assert_eq!(after.cell_birth[migrated], before.cell_birth[advancing]);
        assert_eq!(
            after.cell_class(migrated),
            before.cell_class(advancing),
            "the overriding plate's material now covers the cell"
        );
        assert_ne!(after.cell_birth[migrated], before.cell_birth[migrated]);
    }

    #[test]
    fn cell_crust_follows_birth_and_plate_classes_stay_fixed() {
        let fixture = two_plate_fixture(1.0, vec![CrustClass::Continental; 2]);
        let original_classes = fixture.crust.plate_classes.clone();
        let evolution = fixture.evolve(rift_config(4));

        assert_eq!(fixture.crust.plate_classes, original_classes);
        assert!(
            (0..fixture.mesh.cell_count()).any(|cell| {
                evolution.cell_class(cell)
                    != fixture.crust.plate_classes[evolution.partition.cell_plates[cell]]
            }),
            "a rifted cell's crust must not follow its plate's class"
        );
        for (cell, birth) in evolution.cell_birth.iter().enumerate() {
            let expected = match birth {
                Some(_) => CrustClass::Oceanic,
                None => CrustClass::Continental,
            };
            assert_eq!(evolution.cell_class(cell), expected);
        }
    }

    #[test]
    fn steps_run_even_when_no_boundary_qualifies() {
        let fixture = evolution_fixture();
        let evolution = fixture.evolve(PlateEvolutionConfig {
            step_count: 4,
            migration: PlateMigrationConfig {
                minimum_convergence: f32::MAX,
            },
            ..reference_evolution_config()
        });

        assert_eq!(evolution.partition, fixture.partition);
        assert_eq!(evolution.diagnostics.proposal_count, 0);
        assert_eq!(evolution.diagnostics.migrated_cell_count, 0);
    }

    #[test]
    fn rejects_invalid_configuration_and_mismatched_inputs() {
        let fixture = evolution_fixture();
        for step_duration in [f32::NAN, -1.0] {
            assert_eq!(
                evolve_plate_ownership(
                    &fixture.mesh,
                    fixture.inputs(),
                    PlateEvolutionConfig {
                        step_duration,
                        ..reference_evolution_config()
                    }
                ),
                Err(PlateEvolutionError::InvalidStepDuration)
            );
        }
        assert_eq!(
            evolve_plate_ownership(
                &fixture.mesh,
                fixture.inputs(),
                PlateEvolutionConfig {
                    migration: PlateMigrationConfig {
                        minimum_convergence: f32::NAN,
                    },
                    ..reference_evolution_config()
                }
            ),
            Err(PlateEvolutionError::InvalidMinimumConvergence)
        );

        let short_prior = CrustBirthPrior {
            cell_birth: fixture.birth_prior.cell_birth[1..].to_vec(),
            diagnostics: fixture.birth_prior.diagnostics,
        };
        assert_eq!(
            evolve_plate_ownership(
                &fixture.mesh,
                PlateEvolutionInputs {
                    birth_prior: &short_prior,
                    ..fixture.inputs()
                },
                reference_evolution_config()
            ),
            Err(PlateEvolutionError::Input(StageInputError::CrustBirth))
        );

        let short_boundaries = empty_boundaries(&fixture.mesh);
        let mut short_boundaries = short_boundaries;
        short_boundaries.edge_classes.pop();
        assert_eq!(
            evolve_plate_ownership(
                &fixture.mesh,
                PlateEvolutionInputs {
                    boundaries: &short_boundaries,
                    ..fixture.inputs()
                },
                reference_evolution_config()
            ),
            Err(PlateEvolutionError::Input(StageInputError::Boundaries))
        );
    }

    #[test]
    fn velocity_helper_moves_the_two_plates_apart() {
        let (mesh, edge, _) = two_plate_boundary_partition();
        let kinematics = opposed_kinematics(&mesh, edge, 1.0);
        let [first, second] = mesh.edges[edge].cells.map(|cell| mesh.cell_centers[cell]);
        let apart: Vec3 = (second - first).normalized();

        assert!(kinematics.velocity_at(1, second).dot(apart) > 0.5);
        assert!(kinematics.velocity_at(0, first).dot(apart) < -0.5);
    }
}
