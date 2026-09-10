//! Deterministic multi-step plate evolution.
//!
//! A run classifies the boundaries of the current ownership, advances one step
//! over them, reclassifies, and repeats. What a step does lives in `step.rs`;
//! this module owns the run: its config, its inputs, the totals it keeps, and
//! the state it hands on.
//!
//! Five things survive a step: ownership, the two fields each cell carries —
//! its birth step and the deformation the boundaries have raised on it — and
//! the closing and travel debts. Crust class is not one of them. It is read
//! from birth, so a continental plate that rifts grows an oceanic margin and an
//! overridden cell takes the overriding material's class without anything
//! storing a second answer to the question.
//!
//! Deformation therefore records where the boundaries have been as well as
//! where they are: belts widen where a boundary converged for many steps, a
//! suture stays behind where a boundary used to be, and a boundary that
//! changed regime leaves both marks.

use crate::{
    BoundaryClassification, BoundaryClassificationError, BoundaryDeformation,
    BoundaryDeformationConfig, BoundaryDeformationDiagnostics, BoundaryDeformationError, CellCrust,
    CrustBirthPrior, CrustClassification, PlateKinematics, PlateMigration, PlateMigrationConfig,
    PlateMigrationError, PlatePartition, StageInputError, classify_boundaries,
    deformation::validate_config,
    field::{DEFAULT_STEP_DURATION, mean_cell_width},
    step::{CarriedFields, EvolvingWorld},
};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateEvolutionConfig {
    /// Number of complete boundary-classification, deformation, migration,
    /// and advection transitions.
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
}

impl Default for PlateEvolutionConfig {
    fn default() -> Self {
        Self {
            step_count: 5,
            step_duration: DEFAULT_STEP_DURATION,
            migration: PlateMigrationConfig::default(),
            deformation: BoundaryDeformationConfig::default(),
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

/// Final ownership, boundary state, and the fields the run's cells carry.
#[derive(Clone, Debug, PartialEq)]
pub struct PlateEvolution {
    pub partition: PlatePartition,
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
/// applies one simultaneous migration transition, and advects what the cells
/// carry one cell width at a time.
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
    validate_config(config.deformation)?;
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
        carried: CarriedFields::new(inputs.birth_prior.cell_birth.clone()),
        edge_closing: vec![0.0; mesh.edge_count()],
        cell_travel: vec![0.0; mesh.cell_count()],
    };
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
        boundaries = classify_boundaries(mesh, &world.partition, inputs.kinematics)?;
    }

    let CarriedFields {
        birth: cell_birth,
        deformation: cell_deformation,
    } = world.carried;
    Ok(PlateEvolution {
        partition: world.partition,
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
    use crate::deformation::accumulate_boundary_deformation;
    use crate::test_support::{
        EvolutionFixture, empty_boundaries, evolution_fixture, fingerprint, opposed_kinematics,
        reference_evolution_config, rift_config, two_plate_boundary_partition, two_plate_fixture,
    };
    use crate::{BoundaryClass, BoundaryEffect, ContinentalRiftProfile, CrustClass};
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
        assert_eq!(first.diagnostics.proposal_count, 174);
        assert_eq!(first.diagnostics.contested_cell_count, 27);
        assert_eq!(first.diagnostics.migrated_cell_count, 145);
        assert_eq!(first.diagnostics.born_cell_count, 51);
        // Convergence is a float reduction, so machines differ in the last bits.
        assert!((first.diagnostics.maximum_convergence - 1.579_952_7).abs() < 1.0e-3);
        assert_eq!(ownership_fingerprint(&first), 3_267_391_620_510_768_872);
        assert_eq!(birth_fingerprint(&first), 8_599_187_947_100_659_055);

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
            ..PlateEvolutionConfig::default()
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
            after.cell_crust().class(migrated),
            before.cell_crust().class(advancing),
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
                evolution.cell_crust().class(cell)
                    != fixture.crust.plate_classes[evolution.partition.cell_plates[cell]]
            }),
            "a rifted cell's crust must not follow its plate's class"
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

    /// A convergent two-plate run whose boundary never moves: migration is off
    /// and the step is far too short for any cell to travel a cell width, so
    /// every step classifies the same boundaries and raises the same profile.
    fn static_boundary_fixture(
        step_count: usize,
        maximum_magnitude: f32,
    ) -> (EvolutionFixture, PlateEvolutionConfig) {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental, CrustClass::Oceanic]);
        let config = PlateEvolutionConfig {
            step_count,
            step_duration: 0.1,
            migration: PlateMigrationConfig {
                minimum_convergence: f32::MAX,
            },
            deformation: BoundaryDeformationConfig {
                // A tenth of the profile per step, and every boundary here
                // closes far faster than this, so every source saturates.
                full_deformation_time: 1.0,
                saturation_speed: 0.1,
                maximum_magnitude,
                ..BoundaryDeformationConfig::default()
            },
        };
        (fixture, config)
    }

    #[test]
    fn a_boundary_that_stays_put_adds_the_same_increment_every_step_until_the_clamp() {
        let limit = 0.12;
        let (fixture, config) = static_boundary_fixture(1, limit);
        let single = fixture.evolve(config).deformation.cell_deformation;
        assert_eq!(single[fixture.mesh.edges[0].cells[0]], 0.05);

        for step_count in 1..=6 {
            let run = fixture.evolve(PlateEvolutionConfig {
                step_count,
                ..config
            });
            assert_eq!(
                run.partition, fixture.partition,
                "nothing may move, or the increments would differ between steps"
            );
            assert_eq!(run.cell_birth, fixture.birth_prior.cell_birth);
            for (cell, &value) in run.deformation.cell_deformation.iter().enumerate() {
                let expected = (step_count as f32 * single[cell]).clamp(-limit, limit);
                // Summing an increment n times and multiplying it by n round
                // differently in the last bits.
                assert!(
                    (value - expected).abs() < 1.0e-6,
                    "cell {cell} after {step_count} steps: {value} against {expected}"
                );
            }
        }
    }

    #[test]
    fn deformation_reaches_no_further_than_the_profiles_propagate() {
        let depth = 2;
        let (fixture, config) = static_boundary_fixture(4, 1.0);
        let effect = BoundaryEffect { offset: 0.5, depth };
        // Every profile reaches exactly `depth` hops: a linear effect decays to
        // zero one hop past its own, and a rift one hop past its decay depth.
        let run = fixture.evolve(PlateEvolutionConfig {
            deformation: BoundaryDeformationConfig {
                convergent: effect,
                transform: effect,
                collision: effect,
                trench: BoundaryEffect {
                    offset: -0.2,
                    depth,
                },
                rift: ContinentalRiftProfile {
                    decay_depth: depth + 1,
                    ..config.deformation.rift
                },
                ..config.deformation
            },
            ..config
        });

        let mut reached: Vec<_> = (0..fixture.mesh.cell_count())
            .map(|cell| {
                fixture.mesh.cell_corners(cell).iter().any(|corner| {
                    fixture.partition.cell_plates[corner.neighbor]
                        != fixture.partition.cell_plates[cell]
                })
            })
            .collect();
        for _ in 0..depth {
            reached = (0..fixture.mesh.cell_count())
                .map(|cell| {
                    reached[cell]
                        || fixture
                            .mesh
                            .cell_corners(cell)
                            .iter()
                            .any(|corner| reached[corner.neighbor])
                })
                .collect();
        }

        assert!(
            reached.iter().any(|within| !within),
            "the test needs cells the profiles cannot reach"
        );
        for (cell, &value) in run.deformation.cell_deformation.iter().enumerate() {
            if !reached[cell] {
                assert_eq!(
                    value, 0.0,
                    "cell {cell} is further than {depth} hops from the boundary"
                );
            }
        }
    }

    #[test]
    fn a_boundary_that_has_moved_on_leaves_its_deformation_behind() {
        let (fixture, config, steps) = single_edge_convergent_fixture();
        let config = PlateEvolutionConfig {
            step_count: steps,
            deformation: BoundaryDeformationConfig {
                full_deformation_time: 1.0,
                ..BoundaryDeformationConfig::default()
            },
            ..config
        };
        let run = fixture.evolve(config);

        // The overridden cell was the whole of its plate, so the boundary that
        // deformed these cells no longer exists anywhere.
        assert_eq!(run.diagnostics.migrated_cell_count, 1);
        assert!(
            run.boundaries
                .edge_classes
                .iter()
                .all(|class| *class == BoundaryClass::Interior)
        );
        let mut current = vec![0.0; fixture.mesh.cell_count()];
        accumulate_boundary_deformation(
            &fixture.mesh,
            &run.partition,
            run.cell_crust(),
            &run.boundaries,
            &config.deformation,
            1.0,
            &mut current,
        );
        assert!(current.iter().all(|&value| value == 0.0));
        assert!(
            run.deformation
                .cell_deformation
                .iter()
                .any(|&value| value != 0.0),
            "the suture the vanished boundary left must survive it"
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
