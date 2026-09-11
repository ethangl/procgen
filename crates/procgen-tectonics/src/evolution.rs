//! Deterministic multi-step plate evolution.
//!
//! A run classifies the boundaries of the current ownership, advances one step
//! over them, reclassifies, and repeats. What a step does lives in `step.rs`;
//! this module owns the run: its config, its inputs, the totals it keeps, and
//! the state it hands on.
//!
//! What survives a step: ownership, the plate set itself — its count and its
//! motion — the two fields each cell carries, its birth step and the
//! deformation the boundaries have raised on it, the closing and travel
//! debts, and how long each continental pair has been colliding. Crust is not
//! one of them. It is read from birth, so a continental plate that rifts grows
//! an oceanic margin and an overridden cell takes the overriding material's
//! class without anything storing a second answer to the question. A run reads
//! no crust classification at all: the birth prior has already turned the
//! initial one into the birth field it starts from.
//!
//! Deformation therefore records where the boundaries have been as well as
//! where they are: belts widen where a boundary converged for many steps, a
//! suture stays behind where a boundary used to be, and a boundary that
//! changed regime leaves both marks.

use crate::{
    BoundaryClassification, BoundaryClassificationError, BoundaryDeformation,
    BoundaryDeformationConfig, BoundaryDeformationDiagnostics, BoundaryDeformationError, CellCrust,
    CrustBirthPrior, PlateKinematics, PlateLifecycleConfig, PlateMigration, PlateMigrationConfig,
    PlateMigrationError, PlatePartition, PoleDriftConfig, StageInputError, classify_boundaries,
    deformation::validate_config,
    field::DEFAULT_STEP_DURATION,
    lifecycle::{self, LifecycleEvents},
    step::{CarriedFields, EvolvingWorld},
};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateEvolutionConfig {
    /// Seed for everything a run hashes for itself, which today is the drift
    /// of the plate poles. It is evolution's own seed rather than the
    /// kinematics seed so that re-rolling the drift does not re-roll the
    /// motion it starts from.
    pub seed: u64,
    /// Number of complete boundary-classification, deformation, migration,
    /// advection, and pole-drift transitions.
    pub step_count: usize,
    /// Model time advanced per step. Every displacement is a speed times this
    /// duration measured against the mesh's cell width, so a finer mesh has a
    /// smaller cell width and moves more cells per step for the same motion,
    /// which is what a fixed model time per step should do. Zero freezes the
    /// world. See [`DEFAULT_STEP_DURATION`] for where the default sits.
    pub step_duration: f32,
    pub migration: PlateMigrationConfig,
    /// Profiles the boundaries current in each step raise into the carried
    /// deformation field. It sits here rather than beside evolution because
    /// deformation is a substage of a step exactly as migration is: a config
    /// evolution reads, not a result it is handed.
    pub deformation: BoundaryDeformationConfig,
    /// How far each plate's rotation vector moves at the end of a step.
    pub pole_drift: PoleDriftConfig,
    /// How the plate set itself changes: rifting of large continental plates
    /// and suturing of the continental pairs that have collided long enough.
    pub lifecycle: PlateLifecycleConfig,
}

impl Default for PlateEvolutionConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            step_count: 5,
            step_duration: DEFAULT_STEP_DURATION,
            migration: PlateMigrationConfig::default(),
            deformation: BoundaryDeformationConfig::default(),
            pole_drift: PoleDriftConfig::default(),
            lifecycle: PlateLifecycleConfig::default(),
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
    /// Continental plates that split in two across all steps.
    pub rift_count: usize,
    /// Rift arcs that failed to separate a plate into two pieces, which left
    /// the plate exactly as it was.
    pub failed_rift_count: usize,
    /// Continental pairs that merged across all steps.
    pub suture_count: usize,
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

    fn record_lifecycle(&mut self, events: LifecycleEvents) {
        self.rift_count += events.rift_count;
        self.failed_rift_count += events.failed_rift_count;
        self.suture_count += events.suture_count;
    }
}

/// Everything one evolution run reads that it does not produce.
#[derive(Clone, Copy, Debug)]
pub struct PlateEvolutionInputs<'a> {
    pub partition: &'a PlatePartition,
    /// Plate motion before step zero. A run drifts its own copy of it,
    /// bounds each plate's speed relative to the speed it holds here, and
    /// returns the motion it ended on.
    pub kinematics: &'a PlateKinematics,
    /// Boundaries classified from `partition`, which the first step reads.
    pub boundaries: &'a BoundaryClassification,
    /// Crust birth from before step zero.
    pub birth_prior: &'a CrustBirthPrior,
}

/// Final ownership, boundary state, and the fields the run's cells carry.
#[derive(Clone, Debug, PartialEq)]
pub struct PlateEvolution {
    pub partition: PlatePartition,
    /// Plate motion after the last step's drift. Every consumer that reads
    /// plate motion after evolution reads this rather than the initial
    /// motion, because the boundaries and the fields the run produced were
    /// classified and raised against motion that had already drifted.
    pub kinematics: PlateKinematics,
    pub boundaries: BoundaryClassification,
    /// Step at which each cell's crust was created. Negative steps come from
    /// the prior for crust that predates step zero; `None` is original
    /// continental crust that evolution never re-made.
    pub cell_birth: Vec<Option<i32>>,
    /// Deformation summed over every step's boundaries and carried with the
    /// crust, so it records where boundaries were as well as where they are.
    pub deformation: BoundaryDeformation,
    pub diagnostics: PlateEvolutionDiagnostics,
}

impl PlateEvolution {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), StageInputError> {
        self.partition.validate(mesh)?;
        self.kinematics.validate(&self.partition)?;
        self.boundaries.validate(mesh)?;
        self.deformation.validate(mesh)?;
        self.cell_crust().validate(mesh)
    }

    /// The one per-cell answer to what crust a cell carries after evolution.
    pub fn cell_crust(&self) -> CellCrust<'_> {
        CellCrust {
            cell_birth: &self.cell_birth,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlateEvolutionError {
    InvalidStepDuration,
    InvalidMinimumConvergence,
    InvalidPoleDriftRate,
    InvalidSpeedDriftLimit,
    InvalidRiftRate,
    InvalidRiftAreaFraction,
    InvalidRiftCurvature,
    InvalidRiftOpeningSpeed,
    InvalidSutureTime,
    Deformation(BoundaryDeformationError),
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
            Self::InvalidPoleDriftRate => {
                formatter.write_str("pole drift rates must be finite and non-negative")
            }
            Self::InvalidSpeedDriftLimit => {
                formatter.write_str("speed drift limit must be finite and between 0 and 1")
            }
            Self::InvalidRiftRate => {
                formatter.write_str("rift rate must be finite and non-negative")
            }
            Self::InvalidRiftAreaFraction => {
                formatter.write_str("minimum rift area fraction must lie in [0, 1]")
            }
            Self::InvalidRiftCurvature => {
                formatter.write_str("rift curvature must be finite and non-negative")
            }
            Self::InvalidRiftOpeningSpeed => {
                formatter.write_str("rift opening speed must be finite and non-negative")
            }
            Self::InvalidSutureTime => {
                formatter.write_str("suture time must be non-negative; infinity disables suturing")
            }
            Self::Deformation(error) => error.fmt(formatter),
            Self::Input(error) => error.fmt(formatter),
            Self::Boundary(error) => error.fmt(formatter),
            Self::Migration(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PlateEvolutionError {}

impl From<BoundaryDeformationError> for PlateEvolutionError {
    fn from(error: BoundaryDeformationError) -> Self {
        Self::Deformation(error)
    }
}

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

/// Repeatedly classifies current boundaries, raises deformation along them,
/// applies one simultaneous migration transition, advects what the cells
/// carry one cell width at a time, and drifts every plate's rotation vector
/// before the next classification.
///
/// [`PlateEvolution::cell_crust`] is what a cell carries when the run ends.
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
    let drift = config.pole_drift;
    if [drift.axis_drift_rate, drift.speed_drift_rate]
        .iter()
        .any(|rate| !rate.is_finite() || *rate < 0.0)
    {
        return Err(PlateEvolutionError::InvalidPoleDriftRate);
    }
    if !drift.speed_drift_limit.is_finite() || !(0.0..=1.0).contains(&drift.speed_drift_limit) {
        return Err(PlateEvolutionError::InvalidSpeedDriftLimit);
    }
    validate_config(config.deformation)?;
    lifecycle::validate_config(config.lifecycle)?;
    inputs.partition.validate(mesh)?;
    inputs.kinematics.validate(inputs.partition)?;
    inputs.boundaries.validate(mesh)?;
    inputs.birth_prior.validate(mesh)?;

    let mut world = EvolvingWorld::new(mesh, inputs, config);
    let mut boundaries = inputs.boundaries.clone();
    let mut diagnostics = PlateEvolutionDiagnostics::default();
    let mut source_cell_count = 0;

    for step in 0..config.step_count {
        source_cell_count += world.deform(&boundaries);
        let migrated_cell_count = diagnostics.record_migration(&world.migrate(&boundaries)?);
        let born_cell_count = world.advect(&boundaries, step as i32);
        diagnostics.born_cell_count += born_cell_count;
        diagnostics.active_step_count +=
            usize::from(migrated_cell_count > 0 || born_cell_count > 0);
        world.drift(step as i32);
        diagnostics.record_lifecycle(world.lifecycle(&boundaries, step as i32));
        boundaries = classify_boundaries(mesh, &world.partition, &world.kinematics)?;
    }
    // Compaction is a bijection on the ids that own cells and carries each
    // plate's motion with it, so the boundaries the loop left behind describe
    // the same edges either side of it and are not reclassified.
    world.compact();

    let CarriedFields {
        birth: cell_birth,
        deformation: cell_deformation,
    } = world.carried;
    Ok(PlateEvolution {
        partition: world.partition,
        kinematics: world.kinematics,
        boundaries,
        cell_birth,
        deformation: BoundaryDeformation {
            diagnostics: BoundaryDeformationDiagnostics::summarize(
                &cell_deformation,
                source_cell_count,
            ),
            cell_deformation,
        },
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        NO_LIFECYCLE, NO_POLE_DRIFT, drift_config, empty_boundaries, evolution_fixture,
        fingerprint, opening_config, opposed_kinematics, reference_evolution_config,
        single_edge_convergent_fixture, two_plate_boundary_partition, two_plate_fixture,
    };
    use crate::{BoundaryClass, CrustClass};
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

    #[test]
    fn multi_step_evolution_is_deterministic_and_has_stable_aggregates() {
        let fixture = evolution_fixture();
        let config = reference_evolution_config();
        let first = fixture.evolve(config);

        assert_eq!(first, fixture.evolve(config));
        first.validate(&fixture.mesh).unwrap();
        assert_eq!(first.diagnostics.active_step_count, config.step_count);
        assert!(first.diagnostics.proposal_count >= first.diagnostics.migrated_cell_count);
        assert_eq!(first.diagnostics.proposal_count, 264);
        assert_eq!(first.diagnostics.contested_cell_count, 36);
        assert_eq!(first.diagnostics.migrated_cell_count, 228);
        assert_eq!(first.diagnostics.born_cell_count, 102);
        // Plates of the reference world now clear the minimum continental
        // area a rift needs, where none did before the retune: two draws pass
        // over the run, one splitting its plate and one leaving it in a
        // single piece. One pair merges, and migration empties another four
        // of the thirty-three plates the run started with. Compaction removes
        // every id left owning nothing.
        assert_eq!(first.diagnostics.rift_count, 1);
        assert_eq!(first.diagnostics.failed_rift_count, 1);
        assert_eq!(first.diagnostics.suture_count, 1);
        assert_eq!(first.partition.plate_count, 29);
        // Convergence is a float reduction, so machines differ in the last bits.
        assert!((first.diagnostics.maximum_convergence - 1.875_788).abs() < 1.0e-3);
        assert_eq!(ownership_fingerprint(&first), 9_936_074_511_533_659_768);
        assert_eq!(birth_fingerprint(&first), 11_715_277_558_234_678_678);

        // Float, so it is never pinned; equality above already covers the whole
        // result including this field.
        let deformation = &first.deformation;
        assert!(deformation.diagnostics.affected_cell_count() > 0);
        assert!(deformation.diagnostics.source_cell_count > 0);
        assert!(
            deformation
                .cell_deformation
                .iter()
                .all(|value| value.abs() <= config.deformation.maximum_magnitude)
        );
    }

    /// The reference run with nothing in it that can renumber a plate: no
    /// lifecycle to split or merge one and no migration to empty one, so an id
    /// means the same plate either side of the run and a per-plate comparison
    /// is well defined.
    fn fixed_plate_set_config() -> PlateEvolutionConfig {
        PlateEvolutionConfig {
            migration: PlateMigrationConfig {
                minimum_convergence: f32::MAX,
            },
            lifecycle: NO_LIFECYCLE,
            ..reference_evolution_config()
        }
    }

    /// The angle one step turns an axis through, recovered from the
    /// half-angle tangent the drift is written in.
    fn per_step_angle(config: PlateEvolutionConfig) -> f32 {
        2.0 * (0.5 * config.pole_drift.axis_drift_rate * config.step_duration).atan()
    }

    fn angle_between(first: Vec3, second: Vec3) -> f32 {
        first
            .normalized()
            .dot(second.normalized())
            .clamp(-1.0, 1.0)
            .acos()
    }

    #[test]
    fn without_drift_or_lifecycle_the_run_keeps_the_motion_it_started_from() {
        let fixture = evolution_fixture();
        let evolution = fixture.evolve(PlateEvolutionConfig {
            pole_drift: NO_POLE_DRIFT,
            lifecycle: NO_LIFECYCLE,
            ..reference_evolution_config()
        });

        // Nothing drifts and nothing splits, so every plate that survived the
        // run holds exactly the rotation vector it started with. Compaction
        // drops the ids migration emptied, so the list is shorter rather than
        // equal.
        assert_eq!(
            evolution.kinematics.angular_velocities.len(),
            evolution.partition.plate_count
        );
        assert!(
            evolution
                .kinematics
                .angular_velocities
                .iter()
                .all(|rotation| fixture.kinematics.angular_velocities.contains(rotation))
        );
        // Both are re-pinned by the margin taper's `continental_fraction`
        // retune: the initial mask the birth prior reads decides which cells
        // are oceanic, and migration precedence reads the crust each cell
        // carries.
        assert_eq!(
            ownership_fingerprint(&evolution),
            13_576_351_451_921_853_964
        );
        assert_eq!(birth_fingerprint(&evolution), 17_689_040_188_053_531_059);
    }

    #[test]
    fn every_step_leaves_every_speed_inside_the_configured_bounds() {
        let fixture = evolution_fixture();
        let config = fixed_plate_set_config();
        // The drifted speed is realized by scaling the rotation vector by a
        // ratio, so it lands a few ulps either side of the clamped value.
        let slack = 1.0e-6;
        let limit = config.pole_drift.speed_drift_limit;

        for step_count in [1, 2, 5, 13] {
            let evolution = fixture.evolve(PlateEvolutionConfig {
                step_count,
                ..config
            });
            for (plate, rotation) in evolution.kinematics.angular_velocities.iter().enumerate() {
                let started = fixture.kinematics.angular_velocities[plate].length();
                let bounds = started * (1.0 - limit) - slack..=started * (1.0 + limit) + slack;
                assert!(
                    bounds.contains(&rotation.length()),
                    "plate {plate} left its band after {step_count} steps: {} against {bounds:?}",
                    rotation.length()
                );
            }
        }
    }

    #[test]
    fn a_run_moves_no_axis_further_than_its_steps_could_carry_it() {
        let fixture = evolution_fixture();
        let config = fixed_plate_set_config();
        let per_step = per_step_angle(config);

        for step_count in [1, 3, 9] {
            let evolution = fixture.evolve(PlateEvolutionConfig {
                step_count,
                ..config
            });
            let mut moved = false;
            for (plate, &rotation) in evolution.kinematics.angular_velocities.iter().enumerate() {
                let wander = angle_between(fixture.kinematics.angular_velocities[plate], rotation);
                assert!(
                    wander <= step_count as f32 * per_step + 1.0e-4,
                    "plate {plate} wandered {wander} over {step_count} steps"
                );
                moved |= wander > 0.0;
            }
            assert!(moved, "{step_count} steps of drift moved nothing");
        }
    }

    #[test]
    fn drift_changes_the_regime_of_a_boundary_that_started_convergent() {
        // Nothing here migrates or advects, so the drifting motion is the only
        // thing that can reclassify an edge.
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental, CrustClass::Oceanic]);
        let config = drift_config(24);
        let convergent: Vec<_> = (0..fixture.mesh.edge_count())
            .filter(|&edge| fixture.boundaries.edge_classes[edge] == BoundaryClass::Convergent)
            .collect();
        assert!(!convergent.is_empty(), "the fixture must start convergent");

        // A regime the run passes through, not the one it happens to stop on:
        // the walk is free to bring an axis back to where it started, so the
        // end state alone would be a coin toss.
        let mut changed = false;
        for step_count in 1..=config.step_count {
            let run = fixture.evolve(PlateEvolutionConfig {
                step_count,
                ..config
            });
            assert_eq!(run.partition, fixture.partition);
            changed |= convergent
                .iter()
                .any(|&edge| run.boundaries.edge_classes[edge] != BoundaryClass::Convergent);
        }
        assert!(
            changed,
            "no convergent edge held another regime over {} steps",
            config.step_count
        );
    }

    #[test]
    fn zero_steps_returns_the_initial_state_and_the_prior() {
        let fixture = evolution_fixture();
        let evolution = fixture.evolve(PlateEvolutionConfig {
            step_count: 0,
            ..reference_evolution_config()
        });

        assert_eq!(evolution.partition, fixture.partition);
        assert_eq!(evolution.kinematics, fixture.kinematics);
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
        assert!(
            evolution
                .deformation
                .cell_deformation
                .iter()
                .all(|&value| value == 0.0),
            "a step that advances no time raises no deformation"
        );
    }

    #[test]
    fn a_rifting_plate_grows_oceanic_crust_along_its_trailing_edge() {
        let fixture = two_plate_fixture(1.0, vec![CrustClass::Continental; 2]);
        let config = opening_config(4);
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
            assert_eq!(evolution.cell_crust().class(cell), CrustClass::Oceanic);
            assert!(
                fixture.mesh.cell_corners(cell).iter().any(|corner| {
                    evolution.partition.cell_plates[corner.neighbor]
                        != evolution.partition.cell_plates[cell]
                }),
                "cell {cell} sits on a plate boundary"
            );
        }
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
            after.cell_crust().class(migrated),
            before.cell_crust().class(advancing),
            "the overriding plate's material now covers the cell"
        );
        assert_ne!(after.cell_birth[migrated], before.cell_birth[migrated]);
    }

    #[test]
    fn cell_crust_follows_birth_and_leaves_the_initial_mask_behind() {
        let fixture = two_plate_fixture(1.0, vec![CrustClass::Continental; 2]);
        let evolution = fixture.evolve(opening_config(4));

        assert!(
            (0..fixture.mesh.cell_count())
                .any(|cell| evolution.cell_crust().class(cell) != fixture.crust.class(cell)),
            "a rifted cell's crust must not follow the initial classification"
        );
        for (cell, birth) in evolution.cell_birth.iter().enumerate() {
            let expected = match birth {
                Some(_) => CrustClass::Oceanic,
                None => CrustClass::Continental,
            };
            assert_eq!(evolution.cell_crust().class(cell), expected);
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
            // Migration is the subject; a rift would move ownership too.
            lifecycle: NO_LIFECYCLE,
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
        for pole_drift in [
            PoleDriftConfig {
                axis_drift_rate: -1.0,
                ..PoleDriftConfig::default()
            },
            PoleDriftConfig {
                speed_drift_rate: f32::NAN,
                ..PoleDriftConfig::default()
            },
        ] {
            assert_eq!(
                evolve_plate_ownership(
                    &fixture.mesh,
                    fixture.inputs(),
                    PlateEvolutionConfig {
                        pole_drift,
                        ..reference_evolution_config()
                    }
                ),
                Err(PlateEvolutionError::InvalidPoleDriftRate),
                "{pole_drift:?}"
            );
        }
        for speed_drift_limit in [-0.1, 1.5, f32::NAN] {
            assert_eq!(
                evolve_plate_ownership(
                    &fixture.mesh,
                    fixture.inputs(),
                    PlateEvolutionConfig {
                        pole_drift: PoleDriftConfig {
                            speed_drift_limit,
                            ..PoleDriftConfig::default()
                        },
                        ..reference_evolution_config()
                    }
                ),
                Err(PlateEvolutionError::InvalidSpeedDriftLimit),
                "{speed_drift_limit}"
            );
        }
        assert_eq!(
            evolve_plate_ownership(
                &fixture.mesh,
                fixture.inputs(),
                PlateEvolutionConfig {
                    deformation: BoundaryDeformationConfig {
                        full_deformation_time: 0.0,
                        ..BoundaryDeformationConfig::default()
                    },
                    ..reference_evolution_config()
                }
            ),
            Err(PlateEvolutionError::Deformation(
                BoundaryDeformationError::InvalidConfig
            ))
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
