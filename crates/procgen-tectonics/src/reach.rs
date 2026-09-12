//! How far transport looks, and the longest step that keeps it honest.
//!
//! One fact — the number of rings an empty cell searches — bounds three
//! separate things, so it is stated once here with the two values derived from
//! it: the widest gap radius a search can honour, and the longest step a run
//! may take. Evolution rejects a config that breaks either, and the viewer
//! reads the same two numbers for its sliders, so neither can drift from what
//! `transport.rs` actually does.

use crate::PlateEvolutionConfig;

/// How many hops out from a cell transport looks: the one fact every reach in
/// this module is stated against.
///
/// It bounds three things that have to agree. An empty cell searches this many
/// rings for material, so a gap radius beyond it would be a silent false floor
/// rather than a wider search. A trench takes a losing particle only if a
/// convergent edge lies within it, so a particle that landed further inside
/// another plate than this would stack instead of subduct. And a step that
/// carries a plate further than this outruns both: material jumps trenches
/// without subducting, gaps open that no search can fill, and deformation is
/// raised at boundary positions the plates left partway through the step.
/// [`maximum_step_duration`] is that last one as a time.
pub const TRANSPORT_REACH_HOPS: usize = 2;

/// Largest [`MaterialTransportConfig::gap_radius`] the search can honour, in
/// cell widths: one per hop of [`TRANSPORT_REACH_HOPS`]. A radius beyond it is
/// not a wider search but silent false floor, because the cell would accept
/// material it never looks at. Evolution rejects a larger radius and the
/// viewer's slider stops here.
pub const MAX_GAP_RADIUS: f32 = TRANSPORT_REACH_HOPS as f32;

/// The longest step that keeps a run a coarser version of the same world:
/// what the fastest plate a run can hold takes to cross
/// [`TRANSPORT_REACH_HOPS`] cells.
///
/// It lives here because it is the reach above restated as a time, and the two
/// have to move together: a step longer than this carries material past
/// everything this module looks at.
///
/// The fastest plate is the configured ceiling, not the fastest a fit
/// produced: every respeed clamps to that ceiling, so a plate that gains a
/// trench reaches it. Pole drift may raise a speed by `speed_drift_limit` on
/// top of it, and that is the fastest any plate can be when a step moves
/// material. A world in which nothing can move has no bound at all, and this
/// returns infinity.
///
/// The `rift_opening_speed` term below is dead reserve. A rift adds the
/// opening to the halves' rotation vectors, but the respeed that ends the same
/// step sets their lengths back inside the ceiling, and a run respeeds the
/// motion it was handed before its first step too, so no transport ever reads
/// a speed carrying it. It is kept only because dropping it would loosen the
/// bound, which changes what step durations are legal and how far the viewer's
/// slider reaches; that is its own change and not this one.
///
/// Evolution and the viewer pass the same number, the kinematics config's own
/// maximum: a user editing a step duration has not fitted the plates yet, and
/// a run no longer moves at the speed its plates were fitted at.
pub fn maximum_step_duration(
    maximum_angular_speed: f32,
    radius: f32,
    cell_width: f32,
    config: &PlateEvolutionConfig,
) -> f32 {
    let reach = TRANSPORT_REACH_HOPS as f32 * cell_width;
    let fastest = maximum_angular_speed * (1.0 + config.pole_drift.speed_drift_limit)
        + config.lifecycle.rift_opening_speed;
    reach / (fastest * radius)
}

/// How an empty cell decides whether the plates opened a gap there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialTransportConfig {
    /// How far, in mesh cell widths, an empty cell reaches for material
    /// before it decides nothing arrived and makes ocean floor instead.
    /// Bounded above by [`MAX_GAP_RADIUS`], which is how far the search
    /// itself reaches.
    ///
    /// Cell areas vary, so a rigid rotation alone leaves a third of the cells
    /// empty and a quarter doubled at any moment; those empty cells have
    /// material just outside them and must not make floor. Measured on the
    /// default mesh after rigid rotations of 1, 7, and 50 cell widths, no
    /// empty cell's nearest particle was further than 1.5 cell widths, where
    /// a radius of 1.0 would have made 2, 672, and 949 cells of false floor.
    /// The real gaps a run opens are much wider: after five steps of the
    /// viewer's plate motion, 103 cells had no particle within two hops at
    /// all.
    pub gap_radius: f32,
}

impl Default for MaterialTransportConfig {
    fn default() -> Self {
        Self { gap_radius: 1.5 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        NO_LIFECYCLE, NO_POLE_DRIFT, evolution_fixture, reference_evolution_config,
    };
    use crate::{
        PlateEvolutionError, PlateKinematicsConfig, evolve_plate_ownership, mean_cell_width,
    };

    #[test]
    fn a_step_that_outruns_the_transport_reach_is_rejected() {
        let fixture = evolution_fixture();
        let config = reference_evolution_config();
        let radius = fixture.mesh.radius;
        let cell_width = mean_cell_width(radius, fixture.mesh.cell_count());
        // The configured ceiling, not the fastest plate the fit produced: a
        // plate that gains a trench is respeeded up to that ceiling, so it is
        // the speed the bound has to hold.
        let fastest = PlateKinematicsConfig::new(7).maximum_angular_speed;
        let bound = |config: &PlateEvolutionConfig| {
            maximum_step_duration(fastest, radius, cell_width, config)
        };
        let run = |step_duration| {
            evolve_plate_ownership(
                &fixture.mesh,
                fixture.inputs(),
                PlateEvolutionConfig {
                    step_duration,
                    ..config
                },
            )
        };

        assert!(
            config.step_duration < bound(&config),
            "the reference run must sit inside its own bound"
        );
        assert!(run(bound(&config)).is_ok());
        assert_eq!(
            run(bound(&config) * 1.001),
            Err(PlateEvolutionError::StepOutrunsReach)
        );
        assert_eq!(
            PlateEvolutionError::StepOutrunsReach.to_string(),
            "step duration must not carry a plate further than the 2 cells transport looks"
        );

        // Both terms reserve part of the reach, so switching them off hands it
        // back and a step between the two bounds becomes legal. Only the drift
        // term is spent: see this function's doc for why the rift term is not.
        let still = PlateEvolutionConfig {
            pole_drift: NO_POLE_DRIFT,
            lifecycle: NO_LIFECYCLE,
            ..config
        };
        assert!(bound(&still) > bound(&config));
        let between = 0.5 * (bound(&config) + bound(&still));
        assert_eq!(run(between), Err(PlateEvolutionError::StepOutrunsReach));
        assert!(
            evolve_plate_ownership(
                &fixture.mesh,
                fixture.inputs(),
                PlateEvolutionConfig {
                    step_duration: between,
                    ..still
                },
            )
            .is_ok()
        );
    }
}
