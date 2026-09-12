//! One evolution step: the state it advances and the moves it makes.
//!
//! Deform, transport, drift, lifecycle, respeed, and then the run reclassifies.
//! A step raises deformation along the boundaries it starts from, moves every
//! particle of crust with its plate and resolves what each cell then holds,
//! drifts every plate's rotation vector, rifts and sutures plates, and finally
//! sets every plate's speed from the world the step has left. The deformation
//! happens first because uplift happens at the boundary and the material moves
//! afterwards; the drift happens third, so the step's two moves both read the
//! motion the step began with and the next classification reads the drifted
//! motion; the lifecycle in [`crate::lifecycle`] happens fourth, so the
//! respeed and the next classification both read the new plate set.
//!
//! Kinematics is therefore per-step state like ownership and the material,
//! not a fixed input. Without the drift every step would classify the same
//! relative motion at a boundary that has not moved, and the accumulated
//! fields would record a scaled copy of the final state.
//!
//! The two per-cell fields a run produces — the model time at which a parcel
//! of crust was created, and the deformation the boundaries have raised on it
//! — belong to the particles in [`crate::transport`] rather than to the
//! cells. A cell's answer is whichever particle won it, so the fields move
//! with the material by construction instead of by a rule that copies them
//! between cells.

use crate::{
    BoundaryClassification, CellCrust, PlateEvolutionConfig, PlateEvolutionInputs, PlateKinematics,
    PlateKinematicsConfig, PlatePartition,
    deformation::boundary_deformation_increment,
    field::mean_cell_width,
    plate_speed, subducting_fractions,
    transport::{Particle, initial_particles},
};
use procgen_core::{RandomStream, Vec3, random_streams::PLATE_POLE_DRIFT};
use procgen_sphere_mesh::SphereMesh;
use std::collections::BTreeMap;

/// Draws one plate takes from the drift stream in one step: three for the
/// hashed direction the axis turns toward and one for the change of speed.
/// The item coordinate names the plate, so the step takes its own block of
/// four samples.
const DRIFT_DRAWS_PER_STEP: u64 = 4;

/// How far a plate's rotation vector moves per unit of root model time, and
/// how far from the motion it started with it may end up.
///
/// Both rates are per unit root time rather than per step or per unit time,
/// because both drifts are random walks and a random walk's spread grows with
/// the root of the time it takes. A rate against `sqrt(step_duration)` is
/// therefore the one that leaves a run's total wander where it is when the
/// run is sliced more finely: a rate against the step itself would halve the
/// variance every time the step halved. Zero means no drift.
///
/// The defaults are modest on purpose. Drift is what makes a boundary change
/// regime during a run, which is the whole point of carrying accumulated
/// fields, but drift that is too large makes boundaries flicker between
/// regimes from step to step and blurs the fields into an average instead of
/// a record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoleDriftConfig {
    /// Angle in model radians per unit root time by which the rotation axis
    /// turns, toward a fresh hashed direction perpendicular to it each step.
    /// The angle per step is fixed and only the direction is hashed, so an
    /// axis takes a random walk on the sphere of directions: over `n` steps of
    /// angle `theta` the expected total wander is roughly `theta * sqrt(n)`.
    pub axis_drift_rate: f32,
    /// Fractional change of angular speed per unit root time. One step
    /// multiplies the plate's drift factor by `1 + s * rate *
    /// sqrt(step_duration)` for a hashed `s` in `[-1, 1)`.
    pub speed_drift_rate: f32,
    /// Fraction either side of one that a plate's drift factor may reach, and
    /// so the fraction either side of the speed the slab rule gives it that
    /// its drifted speed may reach. The band is relative to that speed rather
    /// than to the global angular-speed range, because a random walk against
    /// fixed global limits eventually piles every plate against one of them,
    /// where a band around the rule's own answer keeps drift the perturbation
    /// of it that it is meant to be. At one the band reaches zero; a plate can
    /// slow to a stop but never reverse.
    pub speed_drift_limit: f32,
}

impl Default for PoleDriftConfig {
    fn default() -> Self {
        Self {
            // A default fifteen-step run at `DEFAULT_STEP_DURATION` turns an
            // axis by `1.8 * sqrt(0.014) = 0.213` radians per step, and
            // `0.213 * sqrt(15)` is 0.83 radians: about forty-seven degrees of
            // expected total wander, enough for boundaries to change regime
            // several times and little enough that they do not flicker.
            axis_drift_rate: 1.8,
            // `0.9 * sqrt(0.014)` is 0.106, so a step changes a plate's drift
            // factor by at most about a tenth.
            speed_drift_rate: 0.9,
            // A step's expected change is `0.106 / sqrt(3)`, so a default
            // fifteen-step walk is expected to stray about a quarter. At a
            // half the band bounds the tail of that walk without shaping the
            // bulk of it: five of the 111 plates at the viewer's defaults
            // reach the edge over fifteen steps, and a long run stays inside.
            speed_drift_limit: 0.5,
        }
    }
}

/// Half the angle one drift turns a plate's axis through, which is the number
/// the half-angle tangent form in [`EvolvingWorld::drift`] takes.
///
/// It is here rather than inline in the substep so that the angle a step turns
/// through can be read directly, by the substep and by the test that checks
/// how it scales, instead of being recovered from a rotated vector.
pub(crate) fn half_axis_turn(drift: PoleDriftConfig, step_duration: f32) -> f32 {
    0.5 * drift.axis_drift_rate * step_duration.sqrt()
}

/// The state one step advances, together with the inputs every step reads.
///
/// Ownership, the material, and what each cell samples from it move together
/// once per step and nothing outside evolution owns a step, so holding them
/// in one place keeps the per-step solves in `transport.rs` and
/// `lifecycle.rs` methods on one state rather than functions that also return
/// several vectors.
pub(crate) struct EvolvingWorld<'a> {
    pub(crate) mesh: &'a SphereMesh,
    pub(crate) config: PlateEvolutionConfig,
    /// The config the motion a run starts from was fitted under, which the
    /// respeed re-reads every step. Evolution holds it rather than re-deriving
    /// a speed rule of its own, so one world cannot be run under two.
    pub(crate) kinematics_config: PlateKinematicsConfig,
    /// What the drift has done to each plate's speed, as a multiple of the
    /// speed [`crate::plate_speed`] gives it. The walk is of this factor
    /// rather than of the speed itself, because the speed is derived afresh
    /// every step from the world the plate is in: a walk of the speed would
    /// be overwritten by the next respeed. A factor starts at one, and the
    /// band it is clamped to is one either side, which is the band around the
    /// rule's own answer it replaces.
    pub(crate) drift_factors: Vec<f32>,
    /// The one distance a gap radius is measured against.
    pub(crate) cell_width: f32,
    /// Read between steps to reclassify, and taken when the run ends. The
    /// initial motion belongs to the caller; a run drifts its own copy.
    pub(crate) kinematics: PlateKinematics,
    /// Read between steps to reclassify, and taken when the run ends. Cell
    /// ownership is what the particles in each cell resolved to.
    pub(crate) partition: PlatePartition,
    /// The material itself. Everything a run conserves is conserved here.
    pub(crate) particles: Vec<Particle>,
    /// What each cell sampled from the particle that won it. Both are the
    /// run's output and neither is state a step reads back into the
    /// particles: a transport writes them, and the next step's deformation
    /// and lifecycle read the crust they imply.
    pub(crate) cell_birth: Vec<Option<f32>>,
    pub(crate) cell_deformation: Vec<f32>,
    /// Model time each adjacent continental pair has spent in collision,
    /// keyed by the pair in ascending id order. A pair that stops colliding
    /// drops out and starts over.
    pub(crate) collisions: BTreeMap<(usize, usize), f32>,
}

impl<'a> EvolvingWorld<'a> {
    /// The world before step zero: the given ownership and birth prior, one
    /// particle per cell at the cell's own centre, and no deformation.
    /// `inputs` must already have been validated against `mesh`.
    pub(crate) fn new(
        mesh: &'a SphereMesh,
        inputs: PlateEvolutionInputs<'a>,
        config: PlateEvolutionConfig,
    ) -> Self {
        Self {
            mesh,
            config,
            kinematics_config: inputs.kinematics_config,
            drift_factors: vec![1.0; inputs.partition.plate_count],
            kinematics: inputs.kinematics.clone(),
            cell_width: mean_cell_width(mesh.radius, mesh.cell_count()),
            partition: inputs.partition.clone(),
            particles: initial_particles(
                mesh,
                &inputs.partition.cell_plates,
                &inputs.birth_prior.cell_birth,
            ),
            cell_birth: inputs.birth_prior.cell_birth.clone(),
            cell_deformation: vec![0.0; mesh.cell_count()],
            collisions: BTreeMap::new(),
        }
    }

    /// What crust every cell carries as the run stands: the one per-cell
    /// answer, which every substep that asks about crust reads.
    pub(crate) fn cell_crust(&self) -> CellCrust<'_> {
        CellCrust {
            cell_birth: &self.cell_birth,
        }
    }

    /// Raises this step's share of the boundary profiles onto the material
    /// lying in the cells the current boundaries run through, and returns how
    /// many cells sourced one.
    ///
    /// A step spends `step_duration` of the full deformation time, so a
    /// boundary that stays saturated for that whole time reaches the full
    /// profile offset and one that passes through leaves a fraction of it.
    /// Every particle in a cell takes the cell's increment, stacked ones
    /// included: material under a collision is being deformed too.
    pub(crate) fn deform(&mut self, boundaries: &BoundaryClassification) -> usize {
        let config = self.config.deformation;
        let (increment, source_cell_count) = boundary_deformation_increment(
            self.mesh,
            &self.partition,
            // Spelled out rather than through `cell_crust`, so the borrow is
            // of the birth column alone and the particles stay mutable.
            CellCrust {
                cell_birth: &self.cell_birth,
            },
            boundaries,
            &config,
            self.config.step_duration / config.full_deformation_time,
        );
        for particle in &mut self.particles {
            particle.deformation =
                config.accumulate(particle.deformation, increment[particle.cell]);
        }
        source_cell_count
    }

    /// Steps every plate's rotation vector once: a turn of the axis through a
    /// fixed angle toward a fresh hashed perpendicular direction, and a
    /// hashed change of the plate's drift factor bounded to a band around one.
    ///
    /// The factor is what the walk carries; the respeed that ends the step
    /// multiplies the slab rule's answer by it. The rotation vector's own
    /// length is left where the turn put it, because the respeed sets it.
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
        let half_tangent = half_axis_turn(config, self.config.step_duration);
        let speed_span = config.speed_drift_rate * self.config.step_duration.sqrt();
        let stream = RandomStream::new(self.config.seed, PLATE_POLE_DRIFT);
        for (plate, rotation) in self.kinematics.angular_velocities.iter_mut().enumerate() {
            let speed = rotation.length();
            let item = plate as u64;
            let sample = step as u64 * DRIFT_DRAWS_PER_STEP;
            self.drift_factors[plate] = (self.drift_factors[plate]
                * (1.0 + stream.signed_f32(item, sample + 3) * speed_span))
                .clamp(
                    1.0 - config.speed_drift_limit,
                    1.0 + config.speed_drift_limit,
                );
            if speed == 0.0 {
                // A plate fitted at rest has no axis to turn. Its factor still
                // walks, so that a rift giving one of its halves a direction
                // hands that half a plate whose drift is where the run's is.
                continue;
            }
            let axis = *rotation * speed.recip();
            let hashed = Vec3::new(
                stream.signed_f32(item, sample),
                stream.signed_f32(item, sample + 1),
                stream.signed_f32(item, sample + 2),
            );
            // Perpendicular to the axis and of the rotation vector's own
            // length, which is what makes the turn preserve that length.
            let perpendicular = (hashed - axis * hashed.dot(axis)).normalized() * speed;
            *rotation = rotation.rotated_toward(perpendicular, half_tangent);
        }
    }

    /// Sets every plate's speed from the world the step has left it in: the
    /// slab rule over the plate's current continental share and its current
    /// subducting fraction, times the drift factor. Direction is untouched.
    ///
    /// `boundaries` are the boundaries the step began with, read over the
    /// ownership transport has since moved, so the fraction lags the ownership
    /// by one step exactly as every other substep's reads do. The
    /// classification that follows therefore reads speeds consistent with the
    /// boundaries it will be stored beside.
    ///
    /// A plate whose rotation vector is zero has no direction to keep and is
    /// left at rest.
    pub(crate) fn respeed(&mut self, boundaries: &BoundaryClassification) {
        let continental = self
            .cell_crust()
            .plate_continental_fraction(self.mesh, &self.partition);
        let subducting =
            subducting_fractions(self.mesh, &self.partition, self.cell_crust(), boundaries);
        for (plate, rotation) in self.kinematics.angular_velocities.iter_mut().enumerate() {
            let length = rotation.length();
            if length == 0.0 {
                continue;
            }
            let speed = plate_speed(
                self.kinematics.base_speeds[plate],
                continental[plate],
                subducting[plate],
                self.kinematics_config,
            ) * self.drift_factors[plate];
            *rotation = *rotation * (speed / length);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CrustClass;
    use crate::test_support::{drift_config, two_plate_fixture};

    /// The angle a drift turns an axis through, read off the substep's own
    /// half-angle tangent. The `atan` is scaffolding: the substep itself never
    /// calls libm.
    fn per_step_angle(config: PlateEvolutionConfig) -> f32 {
        2.0 * half_axis_turn(config.pole_drift, config.step_duration).atan()
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

    /// The band is around one, which is the band around the speed the slab
    /// rule gives a plate that it stands for.
    #[test]
    fn a_long_walk_leaves_every_drift_factor_inside_its_band() {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental, CrustClass::Oceanic]);
        let config = drift_config(1);
        let limit = config.pole_drift.speed_drift_limit;
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), config);
        assert!(world.drift_factors.iter().all(|&factor| factor == 1.0));

        let mut reached = 0.0_f32;
        for step in 0..200 {
            world.drift(step);
            for (plate, &factor) in world.drift_factors.iter().enumerate() {
                assert!(
                    (1.0 - limit..=1.0 + limit).contains(&factor),
                    "plate {plate} left its band at step {step}: {factor}"
                );
                reached = reached.max((factor - 1.0).abs());
            }
        }
        // The walk has to reach the band for the bound above to mean anything.
        assert!(reached > limit / 2.0, "the walk barely moved: {reached}");
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
