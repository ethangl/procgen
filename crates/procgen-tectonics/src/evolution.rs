//! Deterministic multi-step plate evolution.
//!
//! A run classifies the boundaries of the current ownership, advances one step
//! over them, reclassifies, and repeats. What a step does lives in `step.rs`;
//! this module owns the run: its config, its inputs, the totals it keeps, and
//! the state it hands on.
//!
//! What survives a step: the material itself — one column per parcel of
//! crust, each holding its birth time, the deformation the boundaries have
//! raised on it, and how many original parcels it has merged — the ownership
//! and the three fields the cells read off it, the

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
    BoundaryClassification, BoundaryDeformation, BoundaryDeformationDiagnostics, CellCrust,
    CrustBirthPrior, PlateEvolutionConfig, PlateEvolutionDiagnostics, PlateEvolutionError,
    PlateKinematics, PlatePartition, StageInputError, classify_boundaries, evolution_config,
    maximum_step_duration, step::EvolvingWorld,
};
use procgen_sphere_mesh::{SphereMesh, mean_cell_width};

/// Everything one evolution run reads that it does not produce.
#[derive(Clone, Copy, Debug)]
pub struct PlateEvolutionInputs<'a> {
    pub partition: &'a PlatePartition,
    /// Plate motion before step zero. A run drifts its own copy of it,
    /// bounds each plate's speed relative to the speed it holds here, and
    /// returns the motion it ended on.
    pub kinematics: &'a PlateKinematics,
    /// Boundaries classified from `partition` and `kinematics`, which the
    /// first step reads.
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
    /// Model time at which each cell's crust was created. Negative times come
    /// from the prior for crust that predates step zero; `None` is original
    /// continental crust that evolution never re-made.
    pub cell_birth: Vec<Option<f32>>,
    /// Model time the run covered, the derived step count times the step. It
    /// is what the run actually advanced, which a run duration that is not a
    /// whole number of steps rounds away from. It rides with
    /// the result because every age read off `cell_birth` is measured against
    /// it, and a caller keeping its own copy of the step count could disagree
    /// with the run that produced these births.
    pub elapsed_time: f32,
    /// Deformation summed over every step's boundaries and carried with the
    /// crust, so it records where boundaries were as well as where they are.
    pub deformation: BoundaryDeformation,
    /// Original parcels the column under each cell holds, zero where the cell
    /// reads ocean floor. One is undeformed continent; more is crust a
    /// collision doubled, which base elevation floats as a plateau.
    pub cell_thickness: Vec<u32>,

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

/// Repeatedly classifies current boundaries, raises deformation along them,
/// rotates every particle of crust with its plate and resolves what each cell
/// then holds, and drifts every plate's rotation vector before the next
/// classification.
///
/// A run of no steps is the identity: it returns the ownership, the motion,
/// and the boundaries it was handed.
///
/// [`PlateEvolution::cell_crust`] is what a cell carries when the run ends.
pub fn evolve_plate_ownership(
    mesh: &SphereMesh,
    inputs: PlateEvolutionInputs<'_>,
    config: PlateEvolutionConfig,
) -> Result<PlateEvolution, PlateEvolutionError> {
    evolution_config::validate(&config)?;
    inputs.partition.validate(mesh)?;
    inputs.kinematics.validate(inputs.partition)?;
    inputs.boundaries.validate(mesh)?;
    inputs.birth_prior.validate(mesh)?;
    // Last, because it reads both a validated drift band and a validated
    // motion: a nonsensical band would otherwise report itself as a step that
    // outruns the reach it inflates. The bound is the fastest plate the run
    // actually starts from, which the drift band then inflates; nothing in a
    // run can make a plate faster than the band around where it began.
    let fastest_plate = inputs
        .kinematics
        .angular_velocities
        .iter()
        .map(|rotation| rotation.length())
        .fold(0.0, f32::max);
    if config.step_duration
        > maximum_step_duration(
            fastest_plate,
            mesh.radius,
            mean_cell_width(mesh.radius, mesh.cell_count()),
            &config,
        )
    {
        return Err(PlateEvolutionError::StepOutrunsReach);
    }
    // Beside the step bound, because it is the same kind of rule: a step that
    // is not shorter than the sink's time constant would keep none of the
    // relief it carries, or invert it, rather than decaying it.
    if config.step_duration >= config.deformation.erosion_time {
        return Err(PlateEvolutionError::StepOutrunsErosion);
    }

    let mut world = EvolvingWorld::new(mesh, inputs, config);
    let mut boundaries = inputs.boundaries.clone();
    let mut diagnostics = PlateEvolutionDiagnostics {
        starting_continental_thickness: world.continental_thickness(),
        ..PlateEvolutionDiagnostics::default()
    };
    let mut source_cell_count = 0;

    for step in 0..config.step_count() {
        source_cell_count += world.deform(&boundaries);
        let birth_time = step as f32 * config.step_duration;
        let active = diagnostics.record_transport(world.transport(&boundaries, birth_time));
        diagnostics.active_step_count += usize::from(active);
        world.drift(step as i32);
        diagnostics.record_lifecycle(world.lifecycle(&boundaries, step as i32));
        boundaries = classify_boundaries(mesh, &world.partition, &world.kinematics)?;
    }
    // Read before compaction, which merges the continent of a plate the run
    // left owning no cell into the column above it: after it, those parcels
    // are no longer separate ones to count as covered or foreign.

    diagnostics.covered_continental_particle_count = world.covered_continental_particle_count();
    diagnostics.foreign_continental_particle_count = world.foreign_continental_particle_count();
    // Compaction is a bijection on the ids that own cells and carries each
    // plate's motion with it, so the boundaries the loop left behind describe
    // the same edges either side of it and are not reclassified. What it does
    // to the material is accrete the continent of an emptied plate into the
    // column above it, so the thickness below is read after it rather than
    // before: that is the whole point of the accretion.
    diagnostics.accreted_particle_count += world.compact();
    diagnostics.final_continental_thickness = world.continental_thickness();
    diagnostics.maximum_thickness = world.maximum_thickness();
    diagnostics.thickened_cell_count = world.thickened_cell_count();

    let cell_deformation = world.cell_deformation;

    Ok(PlateEvolution {
        partition: world.partition,
        kinematics: world.kinematics,
        boundaries,
        cell_birth: world.cell_birth,
        cell_thickness: world.cell_thickness,
        elapsed_time: config.step_count() as f32 * config.step_duration,

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
    use crate::step::half_axis_turn;
    use crate::test_support::{
        EvolutionFixture, NO_EROSION, NO_LIFECYCLE, NO_POLE_DRIFT, birth_fingerprint,
        convergent_fixture, drift_config, empty_boundaries, evolution_fixture, fingerprint,
        forced_rift_fixture, opposed_kinematics, reference_evolution_config,
        two_plate_boundary_partition, two_plate_fixture,
    };
    use crate::test_support::{REFERENCE_STEP_COUNT, REFERENCE_STEP_DURATION};
    use crate::{
        BoundaryClass, BoundaryDeformationConfig, BoundaryDeformationError, CrustClass,
        MAX_GAP_RADIUS, MaterialTransportConfig, PoleDriftConfig,
    };
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

    #[test]
    fn multi_step_evolution_is_deterministic_and_has_stable_aggregates() {
        let fixture = evolution_fixture();
        let config = reference_evolution_config();
        let first = fixture.evolve(config);

        assert_eq!(first, fixture.evolve(config));
        first.validate(&fixture.mesh).unwrap();
        assert_eq!(first.diagnostics.active_step_count, config.step_count());
        // Every count here rose when slab pull went: it was a multiplier
        // below one for every plate short of saturation, and no plate of this
        // world is half trench, so dropping it speeds the whole world up and
        // the run moves about half again the material it did.
        assert_eq!(first.diagnostics.owner_change_count, 383);
        assert_eq!(first.diagnostics.subducted_particle_count, 188);
        assert_eq!(first.diagnostics.born_particle_count, 30);
        // Every foreign continental stack this world holds is a merge, which
        // is why the two collision counts read zero: the stacks they counted
        // were continental to a parcel. What is left for them to count is a
        // parcel of another plate that crossed a transform or a ridge by
        // lattice jitter, and this world has none.
        assert_eq!(first.diagnostics.accreted_particle_count, 36);
        assert_eq!(first.diagnostics.collided_cell_count, 0);
        assert_eq!(first.diagnostics.maximum_collision_stack, 0);
        // A merged parcel is one fewer parcel standing in the world, so a cell
        // it would have been near is now a gap that samples its neighbour
        // instead.
        assert_eq!(first.diagnostics.sampled_cell_count, 997);

        // Exact, and the whole of the conservation rule: a collision merges
        // two parcels into one column rather than losing one.
        assert_eq!(first.diagnostics.starting_continental_thickness, 174);
        assert_eq!(first.diagnostics.final_continental_thickness, 174);
        assert_eq!(first.diagnostics.maximum_thickness, 3);
        // Wider than the 27 cells accretion alone thickened: the flow pass
        // spreads a column into its neighbours, which is the whole point of
        // it.
        assert_eq!(first.diagnostics.thickened_cell_count, 67);
        assert_eq!(first.diagnostics.thickness_transfer_count, 16);

        // Plates of the reference world clear the minimum continental area a
        // rift needs, and every draw that took one cut it: a faster world
        // breaks itself into more pieces, so an arc has plate enough to cut
        // through where a slower one ran out of it.
        assert_eq!(first.diagnostics.rift_count, 3);
        assert_eq!(first.diagnostics.failed_rift_count, 0);
        assert_eq!(first.diagnostics.suture_count, 4);
        assert_eq!(first.partition.plate_count, 26);
        // Both moved with crustal thickness, and the reason is worth stating
        // because it is not obvious: within one step accretion only removes a
        // parcel that already lost its cell, so that step's owners and births
        // are untouched. Across steps it is not neutral. The parcel is gone
        // from every later step, so a cell it would have won in step three is
        // won by something else, and the run's floor is made in other places.
        assert_eq!(ownership_fingerprint(&first), 1_855_260_980_066_599_162);
        assert_eq!(
            birth_fingerprint(&first.cell_birth),
            17_664_711_403_482_833_826
        );

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

    /// The sink reaches the deformation field and nothing else. Ownership, the
    /// material, the plate set, and every count a run keeps read the crust and
    /// the motion rather than the relief on them, so turning the sink off
    /// leaves a run exactly as it was before the sink existed, and that is why
    /// every pin above is the pin it was.
    #[test]
    fn turning_the_sink_off_changes_the_deformation_field_alone() {
        let fixture = evolution_fixture();
        let config = reference_evolution_config();
        let eroded = fixture.evolve(config);
        let kept = fixture.evolve(PlateEvolutionConfig {
            deformation: BoundaryDeformationConfig {
                erosion_time: NO_EROSION,
                ..config.deformation
            },
            ..config
        });

        assert_ne!(kept.deformation, eroded.deformation);
        // Compared as whole results rather than field by field, so a field
        // added later is covered without being named here.
        let mut elsewhere = kept;
        elsewhere.deformation = eroded.deformation.clone();
        assert_eq!(elsewhere, eroded);
    }

    /// The reference run with no lifecycle to split or merge a plate, and a
    /// step short enough that no plate loses its last cell, so an id means the
    /// same plate either side of the run and a per-plate comparison is well
    /// defined.
    fn fixed_plate_set_config() -> PlateEvolutionConfig {
        PlateEvolutionConfig {
            lifecycle: NO_LIFECYCLE,
            ..reference_evolution_config()
        }
        .with_steps(REFERENCE_STEP_COUNT, REFERENCE_STEP_DURATION * 0.01)
    }

    /// The angle one step turns an axis through, recovered from the
    /// half-angle tangent the drift is written in.
    fn per_step_angle(config: PlateEvolutionConfig) -> f32 {
        2.0 * half_axis_turn(config.pole_drift, config.step_duration).atan()
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
        let config = PlateEvolutionConfig {
            pole_drift: NO_POLE_DRIFT,
            lifecycle: NO_LIFECYCLE,
            ..reference_evolution_config()
        }
        .with_steps(REFERENCE_STEP_COUNT, REFERENCE_STEP_DURATION * 0.01);
        let evolution = fixture.evolve(config);

        // Nothing drifts and nothing splits a plate, so every plate holds
        // exactly the rotation vector it started with. The step is short
        // enough that no plate loses its last cell, so an id means the same
        // plate either side.
        assert_eq!(
            evolution.kinematics.angular_velocities.len(),
            fixture.partition.plate_count
        );
        assert_eq!(
            evolution.kinematics.angular_velocities,
            fixture.kinematics.angular_velocities
        );
        // Both are re-pinned by the speed rule losing slab pull.
        assert_eq!(ownership_fingerprint(&evolution), 1_312_040_099_017_365_644);
        assert_eq!(
            birth_fingerprint(&evolution.cell_birth),
            13_907_807_829_833_123_813
        );
    }

    /// The speed no plate may pass, which is what
    /// [`crate::maximum_step_duration`] bounds a step against: the fastest
    /// plate the run starts from, widened by the drift band.
    #[test]
    fn no_step_leaves_a_plate_faster_than_the_step_bound_assumes() {
        let fixture = evolution_fixture();
        let config = reference_evolution_config();
        let fastest = fixture
            .kinematics
            .angular_velocities
            .iter()
            .map(|rotation| rotation.length())
            .fold(0.0, f32::max);
        // The drift realizes a speed by scaling the rotation vector by a
        // ratio, so it lands a few ulps either side of the value.
        let ceiling = fastest * (1.0 + config.pole_drift.speed_drift_limit) + 1.0e-6;

        for step_count in [1, 2, 5, 13] {
            let evolution = fixture.evolve(config.with_steps(step_count, config.step_duration));
            for (plate, rotation) in evolution.kinematics.angular_velocities.iter().enumerate() {
                assert!(
                    rotation.length() <= ceiling,
                    "plate {plate} outran the ceiling after {step_count} steps: {}",
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
            let evolution = fixture.evolve(config.with_steps(step_count, config.step_duration));
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
        for step_count in 1..=config.step_count() {
            let run = fixture.evolve(config.with_steps(step_count, config.step_duration));
            assert_eq!(run.partition, fixture.partition);
            changed |= convergent
                .iter()
                .any(|&edge| run.boundaries.edge_classes[edge] != BoundaryClass::Convergent);
        }
        assert!(
            changed,
            "no convergent edge held another regime over {} steps",
            config.step_count()
        );
    }

    /// A run of no steps is the identity: it moves nothing, creates nothing,
    /// and hands back the ownership, the motion, and the boundaries it was
    /// given.
    #[test]
    fn zero_steps_returns_the_initial_state_and_the_prior() {
        let fixture = evolution_fixture();
        let evolution = fixture.evolve(
            reference_evolution_config().with_steps(0, reference_evolution_config().step_duration),
        );

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
            .count() as u32;
        PlateEvolutionDiagnostics {
            starting_continental_thickness: continental,
            final_continental_thickness: continental,
            // Every column is one parcel deep: a still world collides with
            // nothing, so nothing accretes and nothing thickens.
            maximum_thickness: 1,
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
            evolution.diagnostics.final_continental_thickness,
            evolution.diagnostics.starting_continental_thickness
        );
        for (cell, birth) in evolution.cell_birth.iter().enumerate() {
            if birth.is_some_and(|birth| birth >= 0.0) {
                assert!(
                    birth.is_some_and(|birth| birth < evolution.elapsed_time),
                    "cell {cell} was born during the run"
                );
                assert_eq!(evolution.cell_crust().class(cell), CrustClass::Oceanic);
            }
        }
    }

    #[test]
    fn converging_material_crosses_the_boundary_at_the_step_its_speed_predicts() {
        let (fixture, config, steps) = convergent_fixture();

        let waiting = fixture.evolve(config.with_steps(steps - 1, config.step_duration));
        assert_eq!(waiting.diagnostics.owner_change_count, 0);
        assert_eq!(waiting.partition, fixture.partition);

        let moved = fixture.evolve(config.with_steps(steps, config.step_duration));
        assert_eq!(moved.diagnostics.owner_change_count, 1);
    }

    #[test]
    fn an_overridden_cell_takes_the_arriving_materials_crust() {
        let (fixture, config, steps) = convergent_fixture();
        let before = fixture.evolve(config.with_steps(steps - 1, config.step_duration));
        let after = fixture.evolve(config.with_steps(steps, config.step_duration));

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
            run.diagnostics.final_continental_thickness,
            run.diagnostics.starting_continental_thickness,
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
        let evolution = fixture.evolve(
            PlateEvolutionConfig {
                // Transport is the subject; a rift would move ownership too.
                lifecycle: NO_LIFECYCLE,
                ..reference_evolution_config()
            }
            // A hundredth of a cell width per step: every particle stays in
            // the cell it started in, so nothing can change hands.
            .with_steps(4, REFERENCE_STEP_DURATION * 0.01),
        );

        assert_eq!(evolution.partition, fixture.partition);
        assert_eq!(evolution.diagnostics.owner_change_count, 0);
        assert_eq!(evolution.diagnostics.born_particle_count, 0);
        assert_eq!(evolution.diagnostics.subducted_particle_count, 0);
    }

    /// Thickness rather than a particle count, because a collision merges two
    /// parcels into one column: the count falls where the sum does not, and
    /// the sum is what the material actually is.
    #[test]
    fn a_run_conserves_its_continental_material() {
        let fixture = evolution_fixture();
        let starting = fixture
            .birth_prior
            .cell_birth
            .iter()
            .filter(|birth| birth.is_none())
            .count() as u32;
        assert!(starting > 0);

        for step_count in [1, 2, 5, 13] {
            let evolution = fixture.evolve(
                reference_evolution_config()
                    .with_steps(step_count, reference_evolution_config().step_duration),
            );
            let diagnostics = evolution.diagnostics;
            assert_eq!(diagnostics.starting_continental_thickness, starting);
            assert_eq!(
                diagnostics.final_continental_thickness, starting,
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
        for erosion_time in [0.0, -1.0, f32::NAN] {
            assert_eq!(
                evolve_plate_ownership(
                    &fixture.mesh,
                    fixture.inputs(),
                    PlateEvolutionConfig {
                        deformation: BoundaryDeformationConfig {
                            erosion_time,
                            ..BoundaryDeformationConfig::default()
                        },
                        ..reference_evolution_config()
                    }
                ),
                Err(PlateEvolutionError::Deformation(
                    BoundaryDeformationError::InvalidErosionTime
                )),
                "{erosion_time}"
            );
        }
        // The step and the sink are each valid alone; what is rejected is a
        // step that is not shorter than the time constant, which would keep
        // none of the relief it carries or invert it.
        for erosion_time in [REFERENCE_STEP_DURATION, REFERENCE_STEP_DURATION * 0.5] {
            assert_eq!(
                evolve_plate_ownership(
                    &fixture.mesh,
                    fixture.inputs(),
                    PlateEvolutionConfig {
                        deformation: BoundaryDeformationConfig {
                            erosion_time,
                            ..reference_evolution_config().deformation
                        },
                        ..reference_evolution_config()
                    }
                ),
                Err(PlateEvolutionError::StepOutrunsErosion),
                "{erosion_time}"
            );
        }

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
