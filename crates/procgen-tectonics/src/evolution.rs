//! Deterministic multi-step plate evolution.
//!
//! A run classifies the boundaries of the current ownership, advances one step
//! over them, reclassifies, and repeats. What a step does lives in `step.rs`;
//! this module owns the run: its config, its inputs, the totals it keeps, and
//! the state it hands on.
//!
//! What survives a step: the material itself — one particle per parcel of
//! crust, each holding its birth step and the deformation the boundaries have
//! raised on it — the ownership and the two fields the cells read off it, the
//! plate set itself with its count and its motion, and how long each
//! continental pair has been colliding. Crust is not one of them. It is read
//! from birth, so a continental plate that rifts grows an oceanic margin and
//! an overridden cell takes the overriding material's class without anything
//! storing a second answer to the question. A run reads no crust
//! classification at all: the birth prior has already turned the initial one
//! into the birth field it starts from.
//!
//! Deformation therefore records where the boundaries have been as well as
//! where they are: belts widen where a boundary converged for many steps, a
//! suture stays behind where a boundary used to be, and a boundary that
//! changed regime leaves both marks.

use crate::{
    BoundaryClassification, BoundaryClassificationError, BoundaryDeformation,
    BoundaryDeformationConfig, BoundaryDeformationDiagnostics, BoundaryDeformationError, CellCrust,
    CrustBirthPrior, MAX_GAP_RADIUS, MaterialTransportConfig, PlateKinematics,
    PlateLifecycleConfig, PlatePartition, PoleDriftConfig, StageInputError, classify_boundaries,
    deformation::validate_config,
    field::DEFAULT_STEP_DURATION,
    lifecycle::{self, LifecycleEvents},
    step::EvolvingWorld,
    transport::TransportCounts,
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
    /// Number of complete boundary-classification, deformation, transport,
    /// and pole-drift transitions.
    pub step_count: usize,
    /// Model time advanced per step. Every displacement is a speed times this
    /// duration measured against the mesh's cell width, so a finer mesh has a
    /// smaller cell width and moves more cells per step for the same motion,
    /// which is what a fixed model time per step should do. Zero freezes the
    /// world. See [`DEFAULT_STEP_DURATION`] for where the default sits.
    pub step_duration: f32,
    pub transport: MaterialTransportConfig,
    /// Profiles the boundaries current in each step raise into the carried
    /// deformation field. It sits here rather than beside evolution because
    /// deformation is a substage of a step exactly as transport is: a config
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
            transport: MaterialTransportConfig::default(),
            deformation: BoundaryDeformationConfig::default(),
            pole_drift: PoleDriftConfig::default(),
            lifecycle: PlateLifecycleConfig::default(),
        }
    }
}

/// Totals accumulated without retaining per-step history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlateEvolutionDiagnostics {
    /// Steps that changed ownership or made ocean floor.
    pub active_step_count: usize,
    /// Cells whose owner changed, summed over steps. A cell can change more
    /// than once.
    pub owner_change_count: usize,
    /// Particles a trench destroyed across all steps.
    pub subducted_particle_count: usize,
    /// Particles made in cells no material reached across all steps. This is
    /// the only place a run creates material.
    pub born_particle_count: usize,
    /// Cells holding material of more than one plate after subduction,
    /// summed over steps. A collision stacks rather than destroys, and this
    /// counts only that: two parcels of one plate are lattice noise that
    /// spreads back out next step.
    pub collided_cell_count: usize,
    /// Deepest such column over all steps, the foreign material plus the
    /// parcel that won the cell. Zero for a run in which nothing collided.
    pub maximum_collision_stack: usize,
    /// Cells that held no particle and read one nearby, summed over steps.
    pub sampled_cell_count: usize,
    /// Continental particles before step zero and after the last step. The
    /// two are equal for every run: continental material is neither created
    /// nor destroyed. The continental *cell* count is a raster of that
    /// material and is not.
    pub starting_continental_particle_count: usize,
    pub final_continental_particle_count: usize,
    /// Continental plates that split in two across all steps.
    pub rift_count: usize,
    /// Rift arcs that failed to separate a plate into two pieces, which left
    /// the plate exactly as it was.
    pub failed_rift_count: usize,
    /// Continental pairs that merged across all steps.
    pub suture_count: usize,
}

impl PlateEvolutionDiagnostics {
    /// Records one step's transport and returns whether it changed anything.
    fn record_transport(&mut self, counts: TransportCounts) -> bool {
        self.owner_change_count += counts.owner_change_count;
        self.subducted_particle_count += counts.subducted_particle_count;
        self.born_particle_count += counts.born_particle_count;
        self.collided_cell_count += counts.collided_cell_count;
        self.maximum_collision_stack = self
            .maximum_collision_stack
            .max(counts.maximum_collision_stack);
        self.sampled_cell_count += counts.sampled_cell_count;
        counts.owner_change_count > 0 || counts.born_particle_count > 0
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
    InvalidGapRadius,
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
}

impl fmt::Display for PlateEvolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidStepDuration => {
                formatter.write_str("step duration must be finite and non-negative")
            }
            Self::InvalidGapRadius => {
                write!(formatter, "gap radius must lie in [0, {MAX_GAP_RADIUS}]")
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

/// Repeatedly classifies current boundaries, raises deformation along them,
/// rotates every particle of crust with its plate and resolves what each cell
/// then holds, and drifts every plate's rotation vector before the next
/// classification.
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
    if !(0.0..=MAX_GAP_RADIUS).contains(&config.transport.gap_radius) {
        return Err(PlateEvolutionError::InvalidGapRadius);
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
    let mut diagnostics = PlateEvolutionDiagnostics {
        starting_continental_particle_count: world.continental_particle_count(),
        ..PlateEvolutionDiagnostics::default()
    };
    let mut source_cell_count = 0;

    for step in 0..config.step_count {
        source_cell_count += world.deform(&boundaries);
        let active = diagnostics.record_transport(world.transport(&boundaries, step as i32));
        diagnostics.active_step_count += usize::from(active);
        world.drift(step as i32);
        diagnostics.record_lifecycle(world.lifecycle(&boundaries, step as i32));
        boundaries = classify_boundaries(mesh, &world.partition, &world.kinematics)?;
    }
    // Read before compaction, which drops the material of a plate the run
    // left owning no cell at all: that plate's particles are stacked under
    // other plates' cells, and nothing moves them again.
    diagnostics.final_continental_particle_count = world.continental_particle_count();
    // Compaction is a bijection on the ids that own cells and carries each
    // plate's motion with it, so the boundaries the loop left behind describe
    // the same edges either side of it and are not reclassified.
    world.compact();

    let cell_deformation = world.cell_deformation;
    Ok(PlateEvolution {
        partition: world.partition,
        kinematics: world.kinematics,
        boundaries,
        cell_birth: world.cell_birth,
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
    use crate::test_support::REFERENCE_STEP_DURATION;
    use crate::test_support::{
        EvolutionFixture, NO_LIFECYCLE, NO_POLE_DRIFT, convergent_fixture, drift_config,
        empty_boundaries, evolution_fixture, fingerprint, forced_rift_fixture, opposed_kinematics,
        reference_evolution_config, two_plate_boundary_partition, two_plate_fixture,
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
        assert_eq!(first.diagnostics.owner_change_count, 433);
        assert_eq!(first.diagnostics.subducted_particle_count, 127);
        assert_eq!(first.diagnostics.born_particle_count, 23);
        // Cell areas vary, so a rigid rotation alone leaves a third of the
        // cells empty at any moment, and the run makes ocean floor in only 23
        // of the 881 cells that found nothing of their own. The collision
        // count excludes the doubling that same variance causes, so it is
        // small beside them.
        assert_eq!(first.diagnostics.collided_cell_count, 191);
        assert_eq!(first.diagnostics.maximum_collision_stack, 4);
        assert_eq!(first.diagnostics.sampled_cell_count, 881);
        assert_eq!(first.diagnostics.starting_continental_particle_count, 174);
        assert_eq!(first.diagnostics.final_continental_particle_count, 174);
        // Plates of the reference world clear the minimum continental area a
        // rift needs: three draws pass over the run, two splitting their
        // plate and one leaving it in a single piece. Four pairs merge, and
        // transport empties others of the thirty-three plates the run started
        // with. Compaction removes every id left owning nothing.
        assert_eq!(first.diagnostics.rift_count, 2);
        assert_eq!(first.diagnostics.failed_rift_count, 1);
        assert_eq!(first.diagnostics.suture_count, 4);
        assert_eq!(first.partition.plate_count, 25);
        assert_eq!(ownership_fingerprint(&first), 2_496_247_802_890_013_671);
        assert_eq!(birth_fingerprint(&first), 10_835_211_203_217_484_955);

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

    /// The reference run with no lifecycle to split or merge a plate, and a
    /// step short enough that no plate loses its last cell, so an id means the
    /// same plate either side of the run and a per-plate comparison is well
    /// defined.
    fn fixed_plate_set_config() -> PlateEvolutionConfig {
        PlateEvolutionConfig {
            step_duration: REFERENCE_STEP_DURATION * 0.01,
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
        // drops the ids transport emptied, so the list is shorter rather than
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
        // are oceanic, and a cell's resolution reads the crust each particle
        // carries.
        assert_eq!(
            ownership_fingerprint(&evolution),
            12_957_810_193_975_737_552
        );
        assert_eq!(birth_fingerprint(&evolution), 5_122_638_643_250_736_967);
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
        // Nothing here moves material, so the drifting motion is the only
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
        assert_eq!(evolution.diagnostics, still_world_diagnostics(&fixture));
    }

    /// What a run that moved nothing reports: the material it started with,
    /// and no event of any kind.
    fn still_world_diagnostics(fixture: &EvolutionFixture) -> PlateEvolutionDiagnostics {
        let continental = fixture
            .birth_prior
            .cell_birth
            .iter()
            .filter(|birth| birth.is_none())
            .count();
        PlateEvolutionDiagnostics {
            starting_continental_particle_count: continental,
            final_continental_particle_count: continental,
            ..PlateEvolutionDiagnostics::default()
        }
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
        assert_eq!(evolution.diagnostics, still_world_diagnostics(&fixture));
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
    fn a_parting_boundary_makes_ocean_floor_and_creates_no_continental_material() {
        let (fixture, config) = forced_rift_fixture();
        let evolution = fixture.evolve(config);

        // Where the floor appears is `lifecycle`'s statement; what it is made
        // of is this one's. A one-cell plate inside a larger one cannot stand
        // in for it: the plate around it wraps the sphere, so its own
        // material rotates straight into whatever the small plate vacates and
        // no gap ever opens.
        assert!(evolution.diagnostics.born_particle_count > 0);
        assert_eq!(
            evolution.diagnostics.final_continental_particle_count,
            evolution.diagnostics.starting_continental_particle_count
        );
        for (cell, birth) in evolution.cell_birth.iter().enumerate() {
            if birth.is_some_and(|birth| birth >= 0) {
                assert!(
                    birth.is_some_and(|birth| birth < config.step_count as i32),
                    "cell {cell} was born during the run"
                );
                assert_eq!(evolution.cell_crust().class(cell), CrustClass::Oceanic);
            }
        }
    }

    #[test]
    fn converging_material_crosses_the_boundary_at_the_step_its_speed_predicts() {
        let (fixture, config, steps) = convergent_fixture();

        let waiting = fixture.evolve(PlateEvolutionConfig {
            step_count: steps - 1,
            ..config
        });
        assert_eq!(waiting.diagnostics.owner_change_count, 0);
        assert_eq!(waiting.partition, fixture.partition);

        let moved = fixture.evolve(PlateEvolutionConfig {
            step_count: steps,
            ..config
        });
        assert_eq!(moved.diagnostics.owner_change_count, 1);
    }

    #[test]
    fn an_overridden_cell_takes_the_arriving_materials_crust() {
        let (fixture, config, steps) = convergent_fixture();
        let before = fixture.evolve(PlateEvolutionConfig {
            step_count: steps - 1,
            ..config
        });
        let after = fixture.evolve(PlateEvolutionConfig {
            step_count: steps,
            ..config
        });

        let overridden = (0..fixture.mesh.cell_count())
            .find(|&cell| after.partition.cell_plates[cell] != before.partition.cell_plates[cell])
            .unwrap();
        assert_eq!(
            before.cell_crust().class(overridden),
            CrustClass::Oceanic,
            "the cell the continental plate overrides is the oceanic one"
        );
        assert_eq!(
            after.cell_crust().class(overridden),
            CrustClass::Continental,
            "the arriving plate's own material now covers the cell"
        );
        assert_eq!(after.cell_birth[overridden], None);
    }

    #[test]
    fn a_run_with_real_trenches_subducts_ocean_floor_and_nothing_else() {
        let fixture = evolution_fixture();
        let run = fixture.evolve(reference_evolution_config());

        assert!(run.diagnostics.subducted_particle_count > 0);
        assert_eq!(
            run.diagnostics.final_continental_particle_count,
            run.diagnostics.starting_continental_particle_count,
            "subduction takes ocean floor and nothing else"
        );
    }

    #[test]
    fn cell_crust_follows_birth_and_leaves_the_initial_mask_behind() {
        let (fixture, config) = forced_rift_fixture();
        let evolution = fixture.evolve(config);

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
    fn a_step_too_short_to_leave_a_cell_changes_no_owner() {
        let fixture = evolution_fixture();
        let evolution = fixture.evolve(PlateEvolutionConfig {
            step_count: 4,
            // A hundredth of a cell width per step: every particle stays in
            // the cell it started in, so nothing can change hands.
            step_duration: REFERENCE_STEP_DURATION * 0.01,
            // Transport is the subject; a rift would move ownership too.
            lifecycle: NO_LIFECYCLE,
            ..reference_evolution_config()
        });

        assert_eq!(evolution.partition, fixture.partition);
        assert_eq!(evolution.diagnostics.owner_change_count, 0);
        assert_eq!(evolution.diagnostics.born_particle_count, 0);
        assert_eq!(evolution.diagnostics.subducted_particle_count, 0);
    }

    #[test]
    fn a_run_conserves_its_continental_material() {
        let fixture = evolution_fixture();
        let starting = fixture
            .birth_prior
            .cell_birth
            .iter()
            .filter(|birth| birth.is_none())
            .count();
        assert!(starting > 0);

        for step_count in [1, 2, 5, 13] {
            let evolution = fixture.evolve(PlateEvolutionConfig {
                step_count,
                ..reference_evolution_config()
            });
            let diagnostics = evolution.diagnostics;
            assert_eq!(diagnostics.starting_continental_particle_count, starting);
            assert_eq!(
                diagnostics.final_continental_particle_count, starting,
                "{step_count} steps changed how much continental material exists"
            );
        }
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
        for gap_radius in [f32::NAN, -1.0, MAX_GAP_RADIUS + 0.1] {
            assert_eq!(
                evolve_plate_ownership(
                    &fixture.mesh,
                    fixture.inputs(),
                    PlateEvolutionConfig {
                        transport: MaterialTransportConfig { gap_radius },
                        ..reference_evolution_config()
                    }
                ),
                Err(PlateEvolutionError::InvalidGapRadius)
            );
        }
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
