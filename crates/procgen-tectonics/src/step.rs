//! One evolution step: the state it advances and the five moves it makes.
//!
//! A step raises deformation along the boundaries it starts from, moves
//! ownership across the edges whose closing debt has paid for a cell, pulls
//! what the cells carry one cell upstream, drifts every plate's rotation
//! vector, and finally rifts and sutures plates. The first happens before the
//! next two because uplift happens at the boundary and the material moves
//! afterwards; the drift happens fourth, so a step's three moves all read the
//! motion the step began with and the next classification reads the drifted
//! motion; the lifecycle in [`crate::lifecycle`] happens last, so the next
//! classification also reads the new plate set.
//!
//! Kinematics is therefore per-step state like ownership and the carried
//! fields, not a fixed input. Without the drift every step would classify the
//! same relative motion at a boundary that has not moved, and the accumulated
//! fields would record a scaled copy of the final state.
//!
//! Two per-cell fields travel with the crust rather than being recomputed from
//! the current state: the step a cell's crust was created, and the deformation
//! the boundaries have raised on it. Both move under one rule, so
//! [`CarriedFields`] holds them together and one decision per cell applies
//! to both: migration moves what the advancing cell carries onto the cell it
//! overrides, advection moves what the upstream neighbour carries, and new
//! crust starts flat and born now.

use crate::{
    BoundaryClass, BoundaryClassification, CellCrust, CrustClassification, PlateEvolutionConfig,
    PlateEvolutionInputs, PlateKinematics, PlateMigration, PlateMigrationError, PlatePartition,
    deformation::accumulate_boundary_deformation, field::mean_cell_width, migrate_plates_once,
    migration::accumulate_closing_distances,
};
use procgen_core::{RandomStream, Vec3, random_streams::PLATE_POLE_DRIFT};
use procgen_sphere_mesh::SphereMesh;
use std::collections::BTreeMap;

/// Draws one plate takes from the drift stream in one step: three for the
/// hashed direction the axis turns toward and one for the change of speed.
/// The item coordinate names the plate, so the step takes its own block of
/// four samples.
const DRIFT_DRAWS_PER_STEP: u64 = 4;

/// How far a plate's rotation vector moves per unit of model time, and how
/// far from the motion it started with it may end up.
///
/// Both rates are per unit time rather than per step, so changing the step
/// duration changes how many steps a given amount of wander takes rather than
/// how much wander a run produces. Zero means no drift.
///
/// The defaults are modest on purpose. Drift is what makes a boundary change
/// regime during a run, which is the whole point of carrying accumulated
/// fields, but drift that is too large makes boundaries flicker between
/// regimes from step to step and blurs the fields into an average instead of
/// a record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoleDriftConfig {
    /// Angle in model radians per unit time by which the rotation axis turns,
    /// toward a fresh hashed direction perpendicular to it each step. The
    /// angle per step is fixed and only the direction is hashed, so an axis
    /// takes a random walk on the sphere of directions: over `n` steps of
    /// angle `theta` the expected total wander is roughly `theta * sqrt(n)`.
    pub axis_drift_rate: f32,
    /// Fractional change of angular speed per unit time. One step multiplies
    /// the speed by `1 + s * rate * step_duration` for a hashed `s` in
    /// `[-1, 1)`.
    pub speed_drift_rate: f32,
    /// Fraction either side of the speed a plate began the run with that its
    /// drifted speed may reach. The band is relative to the fitted motion
    /// rather than to the global angular-speed range, because a random walk
    /// against fixed global limits eventually piles every plate against one
    /// of them, where a band around the fitted speed keeps drift the
    /// perturbation of the flow field's answer that it is meant to be. At one
    /// the band reaches zero; a plate can slow to a stop but never reverse.
    pub speed_drift_limit: f32,
}

impl Default for PoleDriftConfig {
    fn default() -> Self {
        Self {
            // A default fifteen-step run at `DEFAULT_STEP_DURATION` turns an
            // axis by `15 * 0.014 = 0.21` radians per step, and
            // `0.21 * sqrt(15)` is 0.81 radians: about forty-seven degrees of
            // expected total wander, enough for boundaries to change regime
            // several times and little enough that they do not flicker.
            axis_drift_rate: 15.0,
            // `7.5 * 0.014` is 0.105, so a step changes a plate's speed by at
            // most about a tenth.
            speed_drift_rate: 7.5,
            // A step's expected change is `0.105 / sqrt(3)`, so a default
            // fifteen-step walk is expected to stray about a quarter. At a
            // half the band bounds the tail of that walk without shaping the
            // bulk of it: five of the 111 plates at the viewer's defaults
            // reach the edge over fifteen steps, and a long run stays inside.
            speed_drift_limit: 0.5,
        }
    }
}

/// Everything a cell carries across a step, held column by column so that
/// consumers can borrow the birth field on its own.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CarriedFields {
    /// Step at which each cell's crust was created; `None` is original
    /// continental crust that evolution never re-made.
    pub(crate) birth: Vec<Option<i32>>,
    /// Signed deformation accumulated on each cell at the boundaries current
    /// in each step it has lived through.
    pub(crate) deformation: Vec<f32>,
}

impl CarriedFields {
    /// The fields a run starts from: the birth prior, and crust nothing has
    /// deformed yet.
    pub(crate) fn new(birth: Vec<Option<i32>>) -> Self {
        Self {
            deformation: vec![0.0; birth.len()],
            birth,
        }
    }

    /// Moves everything `from` carried in `previous` onto `to`. Migration and
    /// advection are this one move; they differ only in which cell they read.
    fn move_onto(&mut self, previous: &Self, to: usize, from: usize) {
        self.birth[to] = previous.birth[from];
        self.deformation[to] = previous.deformation[from];
    }

    /// Replaces a cell with crust made this step: new crust is flat.
    fn reborn(&mut self, cell: usize, step: i32) {
        self.birth[cell] = Some(step);
        self.deformation[cell] = 0.0;
    }
}

/// The state one step advances, together with the inputs every step reads.
///
/// Ownership, what the cells carry, and the two debts move together once per
/// step and nothing outside evolution owns a step, so holding them in one
/// place keeps the per-step solves in `migration.rs` pure functions of the
/// current state rather than functions that also return several vectors.
pub(crate) struct EvolvingWorld<'a> {
    pub(crate) mesh: &'a SphereMesh,
    pub(crate) config: PlateEvolutionConfig,
    /// Angular speed each plate began with, kept beside the motion that
    /// drifts: a plate's drifted speed is bounded relative to this rather
    /// than to the kinematics config's global range. "Began" is the run's
    /// start for a plate the partition made, and the step it came into being
    /// for one a rift or a suture made, so a plate the lifecycle creates
    /// drifts around the speed it was created with.
    pub(crate) starting_speeds: Vec<f32>,
    /// The one distance every accumulated displacement is measured against.
    cell_width: f32,
    /// Read between steps to reclassify, and taken when the run ends. The
    /// initial motion belongs to the caller; a run drifts its own copy.
    pub(crate) kinematics: PlateKinematics,
    /// Read between steps to reclassify, and taken when the run ends.
    pub(crate) partition: PlatePartition,
    /// The plate classes the run reads and grows. Rifting appends one and
    /// suturing empties one, so the classes the input handed over belong to a
    /// plate set the run has left behind; migration precedence and the
    /// lifecycle both read this copy.
    pub(crate) crust: CrustClassification,
    /// Taken when the run ends; the two fields it holds are the run's output.
    pub(crate) carried: CarriedFields,
    /// Model time each adjacent continental pair has spent in collision,
    /// keyed by the pair in ascending id order. A pair that stops colliding
    /// drops out and starts over.
    pub(crate) collisions: BTreeMap<(usize, usize), f32>,
    /// Closing distance accumulated per boundary edge, in model units.
    edge_closing: Vec<f32>,
    /// Distance travelled per cell since its last pull, in model units.
    cell_travel: Vec<f32>,
}

impl<'a> EvolvingWorld<'a> {
    /// The world before step zero: the given ownership and birth prior, no
    /// deformation, and no debt. `inputs` must already have been validated
    /// against `mesh`.
    pub(crate) fn new(
        mesh: &'a SphereMesh,
        inputs: PlateEvolutionInputs<'a>,
        config: PlateEvolutionConfig,
    ) -> Self {
        Self {
            mesh,
            config,
            starting_speeds: inputs
                .kinematics
                .angular_velocities
                .iter()
                .map(|rotation| rotation.length())
                .collect(),
            kinematics: inputs.kinematics.clone(),
            cell_width: mean_cell_width(mesh),
            partition: inputs.partition.clone(),
            crust: inputs.crust.clone(),
            carried: CarriedFields::new(inputs.birth_prior.cell_birth.clone()),
            collisions: BTreeMap::new(),
            edge_closing: vec![0.0; mesh.edge_count()],
            cell_travel: vec![0.0; mesh.cell_count()],
        }
    }

    /// Raises this step's share of the boundary profiles onto the cells the
    /// current boundaries run through, and returns how many cells sourced one.
    ///
    /// A step spends `step_duration` of the full deformation time, so a
    /// boundary that stays saturated for that whole time reaches the full
    /// profile offset and one that passes through leaves a fraction of it.
    pub(crate) fn deform(&mut self, boundaries: &BoundaryClassification) -> usize {
        let config = self.config.deformation;
        accumulate_boundary_deformation(
            self.mesh,
            &self.partition,
            CellCrust {
                cell_birth: &self.carried.birth,
            },
            boundaries,
            &config,
            self.config.step_duration / config.full_deformation_time,
            &mut self.carried.deformation,
        )
    }

    /// Advances every edge's closing debt and applies the migrations the debts
    /// have paid for. A migrating cell takes everything the advancing cell
    /// carries across the edge, and the winning edge's debt drops by one cell
    /// width.
    pub(crate) fn migrate(
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
            &self.crust,
            boundaries,
            &self.edge_closing,
            self.cell_width,
        )?;

        let previous = self.carried.clone();
        for (cell, change) in migration.cell_changes.iter().enumerate() {
            let Some(change) = change else { continue };
            let cells = self.mesh.edges[change.boundary_edge].cells;
            let advancing = if cells[0] == cell { cells[1] } else { cells[0] };
            self.carried.move_onto(&previous, cell, advancing);
            self.edge_closing[change.boundary_edge] -= self.cell_width;
        }
        self.partition.clone_from(&migration.partition);
        Ok(migration)
    }

    /// Advances every cell's travel debt and pulls what the cells that have
    /// paid for it carry upstream by one cell.
    ///
    /// A cell's upstream neighbour is the same-plate neighbour lying most
    /// nearly opposite its velocity. A cell with none is at the plate's
    /// trailing edge: if the boundary behind it is a ridge it is new crust
    /// born this step, flat because nothing has deformed it yet, and otherwise
    /// it keeps what it has. Every pull reads the fields as they stood before
    /// this substep, so the update is simultaneous.
    pub(crate) fn advect(&mut self, boundaries: &BoundaryClassification, step: i32) -> usize {
        let mesh = self.mesh;
        let previous = self.carried.clone();
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
                Some((_, neighbor)) => self.carried.move_onto(&previous, cell, neighbor),
                None => {
                    let opening = behind.is_some_and(|(_, edge)| {
                        boundaries.edge_classes[edge] == BoundaryClass::Divergent
                    });
                    if opening {
                        self.carried.reborn(cell, step);
                        born_cell_count += 1;
                    }
                }
            }
        }
        born_cell_count
    }

    /// Steps every plate's rotation vector once: a turn of the axis through a
    /// fixed angle toward a fresh hashed perpendicular direction, and a
    /// hashed change of speed bounded to a band around the speed the plate
    /// started the run with.
    ///
    /// The turn is the half-angle tangent form, so it costs only add,
    /// multiply, and divide. Half the intended angle stands in for its
    /// tangent, which is the same number to third order: at the default rate
    /// the realized turn is four parts in a thousand short of the configured
    /// one, and nothing downstream resolves that. Taking the tangent for real
    /// would put libm back on the path the boundary classes come off.
    pub(crate) fn drift(&mut self, step: i32) {
        let config = self.config.pole_drift;
        // Returning rather than multiplying by one leaves a run with no drift
        // bit-identical to one from before this substep existed.
        if config.axis_drift_rate == 0.0 && config.speed_drift_rate == 0.0 {
            return;
        }
        let half_tangent = 0.5 * config.axis_drift_rate * self.config.step_duration;
        let speed_span = config.speed_drift_rate * self.config.step_duration;
        let stream = RandomStream::new(self.config.seed, PLATE_POLE_DRIFT);
        let starting = &self.starting_speeds;
        for (plate, rotation) in self.kinematics.angular_velocities.iter_mut().enumerate() {
            let speed = rotation.length();
            if speed == 0.0 {
                // A plate that started the run at rest, or that the band let
                // slow to one, has no axis to turn and no speed to scale.
                continue;
            }
            let item = plate as u64;
            let sample = step as u64 * DRIFT_DRAWS_PER_STEP;
            let axis = *rotation * speed.recip();
            let hashed = Vec3::new(
                stream.signed_f32(item, sample),
                stream.signed_f32(item, sample + 1),
                stream.signed_f32(item, sample + 2),
            );
            // Perpendicular to the axis and of the rotation vector's own
            // length, which is what makes the turn preserve that length.
            let perpendicular = (hashed - axis * hashed.dot(axis)).normalized() * speed;
            let turned = rotation.rotated_toward(perpendicular, half_tangent);
            let started = starting[plate];
            let drifted = (speed * (1.0 + stream.signed_f32(item, sample + 3) * speed_span)).clamp(
                started * (1.0 - config.speed_drift_limit),
                started * (1.0 + config.speed_drift_limit),
            );
            *rotation = turned * (drifted / speed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CrustClass;
    use crate::test_support::{drift_config, opening_config, two_plate_fixture};

    /// Birth out of every cell's reach, so a cell carrying it was reborn rather
    /// than handed it by a neighbour.
    const REBIRTH_STEP: i32 = 1_000;

    #[test]
    fn advection_pulls_deformation_from_the_cell_it_pulls_birth_from() {
        // Driven a substep at a time: the rule is about what one advection does
        // to one cell, which a whole run's boundaries would bury.
        let fixture = two_plate_fixture(1.0, vec![CrustClass::Continental; 2]);
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), opening_config(1));
        // Stamp each cell with its own index in both fields, so where a value
        // ends up names the cell it came from.
        for cell in 0..fixture.mesh.cell_count() {
            world.carried.birth[cell] = Some(cell as i32);
            world.carried.deformation[cell] = cell as f32;
        }

        let born_cell_count = world.advect(&fixture.boundaries, REBIRTH_STEP);
        assert!(born_cell_count > 0, "the fixture must rift somewhere");
        let mut pulled_cell_count = 0;
        for cell in 0..fixture.mesh.cell_count() {
            match world.carried.birth[cell].unwrap() {
                REBIRTH_STEP => assert_eq!(
                    world.carried.deformation[cell], 0.0,
                    "cell {cell} is new crust and nothing has deformed it"
                ),
                source => {
                    pulled_cell_count += usize::from(source as usize != cell);
                    assert_eq!(
                        world.carried.deformation[cell], source as f32,
                        "cell {cell} took its deformation from a different cell than its birth"
                    );
                }
            }
        }
        assert!(pulled_cell_count > 0, "the fixture must advect somewhere");
    }

    /// The angle a drift turns an axis through, recovered from the half-angle
    /// tangent the substep is written in. Scaffolding: the substep itself
    /// never calls libm.
    fn per_step_angle(config: PlateEvolutionConfig) -> f32 {
        2.0 * (0.5 * config.pole_drift.axis_drift_rate * config.step_duration).atan()
    }

    #[test]
    fn one_drift_turns_every_axis_through_the_configured_angle_and_keeps_its_speed() {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental, CrustClass::Oceanic]);
        // Speed held still, so the turn is the only thing that can change a
        // rotation vector and its length is the invariant to check.
        let config = PlateEvolutionConfig {
            pole_drift: PoleDriftConfig {
                speed_drift_rate: 0.0,
                ..PoleDriftConfig::default()
            },
            ..drift_config(1)
        };
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), config);
        let before = world.kinematics.clone();

        world.drift(0);

        let expected = per_step_angle(config);
        assert!(expected > 0.0);
        for (plate, &rotation) in world.kinematics.angular_velocities.iter().enumerate() {
            let was = before.angular_velocities[plate];
            assert!(
                (rotation.length() - was.length()).abs() < 1.0e-6,
                "plate {plate} changed speed"
            );
            let turned = rotation
                .normalized()
                .dot(was.normalized())
                .clamp(-1.0, 1.0)
                .acos();
            assert!(
                (turned - expected).abs() < 1.0e-6,
                "plate {plate} turned {turned} rather than {expected}"
            );
        }
    }

    #[test]
    fn a_fresh_direction_each_step_keeps_the_axis_off_a_straight_line() {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental, CrustClass::Oceanic]);
        let config = drift_config(1);
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), config);
        let start = world.kinematics.clone();

        world.drift(0);
        let after_one = world.kinematics.clone();
        world.drift(1);

        for (plate, &rotation) in world.kinematics.angular_velocities.iter().enumerate() {
            let axis = rotation.normalized();
            let first = start.angular_velocities[plate].normalized();
            let second = after_one.angular_velocities[plate].normalized();
            assert_ne!(
                axis, second,
                "plate {plate} did not move on its second step"
            );
            let total = axis.dot(first).clamp(-1.0, 1.0).acos();
            assert!(
                total < 2.0 * per_step_angle(config),
                "plate {plate} walked a straight line: {total}"
            );
        }
    }
}
