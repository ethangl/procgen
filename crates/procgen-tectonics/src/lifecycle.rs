//! The two events that change the plate set during a run, and the compaction
//! that tidies up after them.
//!
//! Everything else evolution does moves ownership between a fixed set of
//! plates. Here the set itself changes: a large continental plate rifts along
//! a fresh crack arc, and two continental plates that have collided long
//! enough suture into one. Both happen at the end of a step, after the poles
//! drift and before the respeed and the next boundary classification, so both
//! read the new plate set.
//!
//! Rifting is the longer of the two and lives in [`crate::rifting`]; this
//! module owns the config both read, the order the two run in, suturing, and
//! compaction.
//!
//! **Suturing.** Each step counts, for every adjacent pair of plates, the
//! shared edges that are convergent with continental crust on both sides. A
//! pair whose shared front is at least `suture_minimum_shared_length` grows
//! its collision time by the step; a pair below it starts over, so a pair that
//! stopped colliding begins its next collision from nothing. At `suture_time` the
//! plate with more area absorbs the other, taking the area-weighted mean of
//! the two rotation vectors, of their base speeds, and of their drift
//! factors, and the absorbed id is left owning nothing.
//!
//! **Compaction.** The run ends by removing every plate id that owns no cell
//! — the ones suturing emptied and the ones transport did — and remapping
//! ownership and kinematics to `0..live_count` in id order, so every plate
//! identity a consumer sees owns cells and no id means the same plate either
//! side of a run.
//!
//! Determinism: the edge tallies and area sums are exact, and the only floats
//! that decide an integer here are area comparisons. Nothing on the path calls
//! libm, so the plate ids and the boundary classes the new rotation vectors
//! decide stay bit-identical across machines.

use crate::{
    BoundaryClass, BoundaryClassification, CrustClass, PlateEvolutionError, step::EvolvingWorld,
    transport::MaterialScope,
};
use procgen_sphere_mesh::{default_hop_length, hops};
use std::collections::BTreeMap;

/// How often a large continental plate breaks up, how large it has to be, and
/// how fast the halves part.
///
/// A `rift_rate` of zero and a `suture_time` of infinity each disable their
/// event, which is what [`crate::test_support::NO_LIFECYCLE`] is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateLifecycleConfig {
    /// Expected rifts per unit of model time per eligible plate. It is a rate
    /// rather than a per-step chance so that changing the step duration
    /// changes how many steps a break-up takes rather than how many a run
    /// produces.
    pub rift_rate: f32,
    /// Fraction of the sphere a continental plate must exceed to rift at all.
    /// Small plates have no room for an arc to separate two halves worth
    /// having.
    pub rift_minimum_area_fraction: f32,
    /// Heading change in radians per radian travelled for the rift arc, the
    /// same units as the partition's crack curvature. Zero rifts along a
    /// great circle.
    pub rift_curvature: f32,
    /// Angular speed each half gains away from the rift, on top of the
    /// parent's motion. It is what makes the new boundary classify divergent,
    /// so it has to part the halves faster than
    /// [`crate::MaterialTransportConfig::gap_radius`] to open anything.
    pub rift_opening_speed: f32,
    /// Model time a continental pair must stay in collision before it merges.
    /// Infinity never merges anything.
    pub suture_time: f32,
    /// Length of shared convergent continental front a pair needs before its
    /// collision time grows at all, as a model length on the unit sphere. A
    /// boundary's edges are its length in hops, to within the mesh's
    /// irregularity, so the stage converts this to an edge count once against
    /// the mesh and the test itself stays an integer.
    pub suture_minimum_shared_length: f32,
}

impl Default for PlateLifecycleConfig {
    fn default() -> Self {
        Self {
            // `3.0 * 0.014` is a chance of one in twenty-four a step per
            // eligible plate: with the four plates the minimum area below
            // makes eligible, a fifteen-step run at the viewer's settings
            // breaks one up and a thirty-step run two. It is swept against
            // that minimum rather than alone, because the two together decide
            // the count: at 4.5 the same four plates give three rifts over
            // those fifteen steps and at 2.25 they give none, the draws being
            // hashed and so lumpy rather than smooth. A half that a rift leaves is usually too
            // small to be eligible again, which is what stops the count
            // running away.
            rift_rate: 3.0,
            // Eligibility is a plate's continental area, which since per-cell
            // crust is much less than its extent: the largest any plate holds
            // at the viewer's defaults is 0.0127 of the sphere, and the fourth
            // largest 0.0122, so this makes four plates eligible at step zero
            // where 0.014 and above make none.
            rift_minimum_area_fraction: 0.012,
            // The partition's own default, so a rift arc bends like the arcs
            // that drew the plate it splits.
            rift_curvature: 8.0,
            // A third of the default maximum angular speed: enough for the
            // halves to part a cell width in about three default steps, and
            // little enough that they stay part of the flow field's pattern.
            rift_opening_speed: 0.33,
            // Eight default steps. Together with the edge count below, the
            // viewer's settings suture once over a nine- or fifteen-step run
            // and three times over thirty.
            suture_time: 8.0 * crate::field::DEFAULT_STEP_DURATION,
            // About a fifth of a default-mesh plate's perimeter, which is a
            // collision front rather than two plates meeting at a corner. It
            // is the knob that matters: at eight default hops a fifteen-step
            // run at the viewer's settings sutures six times, at sixteen three
            // times, and at twenty once.
            suture_minimum_shared_length: 20.0 * default_hop_length(),
        }
    }
}

pub(crate) fn validate_config(config: PlateLifecycleConfig) -> Result<(), PlateEvolutionError> {
    if !config.rift_rate.is_finite() || config.rift_rate < 0.0 {
        return Err(PlateEvolutionError::InvalidRiftRate);
    }
    if !(0.0..=1.0).contains(&config.rift_minimum_area_fraction) {
        return Err(PlateEvolutionError::InvalidRiftAreaFraction);
    }
    if !config.rift_curvature.is_finite() || config.rift_curvature < 0.0 {
        return Err(PlateEvolutionError::InvalidRiftCurvature);
    }
    if !config.rift_opening_speed.is_finite() || config.rift_opening_speed < 0.0 {
        return Err(PlateEvolutionError::InvalidRiftOpeningSpeed);
    }
    // Infinity is the value that disables suturing, so only a negative time
    // or a NaN is rejected.
    if config.suture_time.is_nan() || config.suture_time < 0.0 {
        return Err(PlateEvolutionError::InvalidSutureTime);
    }
    Ok(())
}

/// What one step's lifecycle did to the plate set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LifecycleEvents {
    pub(crate) rift_count: usize,
    pub(crate) failed_rift_count: usize,
    pub(crate) suture_count: usize,
}

impl EvolvingWorld<'_> {
    /// Rifts the large continental plates whose draw passes and merges the
    /// continental pairs whose collision has lasted long enough.
    ///
    /// `boundaries` are the boundaries the step began with, which is what
    /// the transport read to decide its trenches too.
    pub(crate) fn lifecycle(
        &mut self,
        boundaries: &BoundaryClassification,
        step: i32,
    ) -> LifecycleEvents {
        let mut events = self.rift(step);
        events.suture_count = self.suture(boundaries);
        events
    }

    /// Advances every adjacent continental pair's collision time and merges
    /// the pairs that have reached `suture_time`.
    fn suture(&mut self, boundaries: &BoundaryClassification) -> usize {
        let config = self.config.lifecycle;
        // No collision time can ever reach infinity, so the bookkeeping has
        // nothing to decide.
        if !config.suture_time.is_finite() {
            return 0;
        }

        // The one conversion: the configured front is a model length, and the
        // tally below counts edges.
        let minimum_shared_edges =
            hops(self.mesh.cell_count(), config.suture_minimum_shared_length);
        let cell_crust = self.cell_crust();
        let mut shared: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        for (edge_index, edge) in self.mesh.edges.iter().enumerate() {
            if boundaries.edge_classes[edge_index] != BoundaryClass::Convergent {
                continue;
            }
            let plates = edge.cells.map(|cell| self.partition.cell_plates[cell]);
            if plates[0] == plates[1] {
                continue;
            }
            // A suture is continent meeting continent, which is what the two
            // cells' own crust says.
            if edge
                .cells
                .iter()
                .any(|&cell| cell_crust.class(cell) != CrustClass::Continental)
            {
                continue;
            }
            let pair = (plates[0].min(plates[1]), plates[0].max(plates[1]));
            *shared.entry(pair).or_default() += 1;
        }

        // Rebuilt rather than updated in place, so a pair that stopped
        // colliding this step starts its next collision from nothing.
        self.collisions = shared
            .iter()
            .filter(|&(_, &count)| count >= minimum_shared_edges)
            .map(|(&pair, _)| {
                let carried = self.collisions.get(&pair).copied().unwrap_or(0.0);
                (pair, carried + self.config.step_duration)
            })
            .collect();

        let due: Vec<(usize, usize)> = self
            .collisions
            .iter()
            .filter(|&(_, &time)| time >= config.suture_time)
            .map(|(&pair, _)| pair)
            .collect();
        let mut suture_count = 0;
        for pair in due {
            // An earlier merge in this step may have absorbed one of the two,
            // which drops every entry that named it.
            if !self.collisions.contains_key(&pair) {
                continue;
            }
            self.merge_plates(pair);
            suture_count += 1;
        }
        suture_count
    }

    /// Absorbs the smaller of a colliding pair into the larger: every cell of
    /// the absorbed plate takes the absorber's id, the absorber's rotation
    /// vector, base speed, and drift factor each become the area-weighted mean
    /// of the pair's, and the absorbed id is left owning nothing until
    /// [`Self::compact`] removes it.
    fn merge_plates(&mut self, pair: (usize, usize)) {
        // Both plates own the cells of a shared boundary edge, so neither
        // area is zero and the weights are well defined.
        let areas = self.partition.plate_areas(self.mesh);
        let (absorber, absorbed) = if areas[pair.1] > areas[pair.0] {
            (pair.1, pair.0)
        } else {
            pair
        };

        let total = areas[absorber] + areas[absorbed];
        let weights = [absorber, absorbed].map(|plate| (areas[plate] / total) as f32);
        let rotations = [absorber, absorbed].map(|plate| self.kinematics.angular_velocities[plate]);
        self.kinematics.angular_velocities[absorber] =
            rotations[0] * weights[0] + rotations[1] * weights[1];
        // The merged plate is one plate, so it has one base speed and one
        // drift factor. Left at the absorber's own, the smaller plate would
        // bring nothing to the motion the next respeed derives.
        let mean = |values: &[f32]| values[absorber] * weights[0] + values[absorbed] * weights[1];
        self.kinematics.base_speeds[absorber] = mean(&self.kinematics.base_speeds);
        self.drift_factors[absorber] = mean(&self.drift_factors);
        for plate in self.partition.cell_plates.iter_mut() {
            if *plate == absorbed {
                *plate = absorber;
            }
        }
        self.relabel_particles(absorbed, absorber, MaterialScope::WholePlate);
        self.collisions
            .retain(|&(low, high), _| low != absorbed && high != absorbed);
    }

    /// Removes every plate id that owns no cell, remapping ownership, the
    /// rotation vectors, and the plate classes to `0..live_count` in id order.
    ///
    /// Suturing empties the id it absorbs, and transport can take the last
    /// cell of a plate on its own; both leave a hole this closes, so plate ids
    /// are not stable across a run.
    pub(crate) fn compact(&mut self) {
        let mut owned = vec![false; self.partition.plate_count];
        for &plate in &self.partition.cell_plates {
            owned[plate] = true;
        }
        let live: Vec<usize> = (0..self.partition.plate_count)
            .filter(|&plate| owned[plate])
            .collect();
        // A run that emptied no plate has no hole to close, and every id
        // already maps to itself.
        if live.len() == self.partition.plate_count {
            return;
        }

        let mut compacted = vec![usize::MAX; self.partition.plate_count];
        for (id, &plate) in live.iter().enumerate() {
            compacted[plate] = id;
        }
        for plate in self.partition.cell_plates.iter_mut() {
            *plate = compacted[*plate];
        }
        self.remap_particle_plates(&compacted);
        self.kinematics.angular_velocities = live
            .iter()
            .map(|&plate| self.kinematics.angular_velocities[plate])
            .collect();
        self.kinematics.base_speeds = live
            .iter()
            .map(|&plate| self.kinematics.base_speeds[plate])
            .collect();
        self.drift_factors = live
            .iter()
            .map(|&plate| self.drift_factors[plate])
            .collect();
        self.partition.plate_count = live.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CrustClass;
    use crate::test_support::{
        NO_LIFECYCLE, empty_boundaries, evolution_fixture, forced_suture_fixture,
        reference_evolution_config, two_plate_fixture,
    };
    use crate::{PlateEvolutionConfig, evolve_plate_ownership};

    #[test]
    fn a_forced_suture_merges_at_the_step_its_collision_time_predicts() {
        let (fixture, config, steps) = forced_suture_fixture();
        let waiting = fixture.evolve(PlateEvolutionConfig {
            step_count: steps - 1,
            ..config
        });
        assert_eq!(waiting.diagnostics.suture_count, 0);
        assert_eq!(waiting.partition.plate_count, 2);

        let merged = fixture.evolve(PlateEvolutionConfig {
            step_count: steps,
            ..config
        });
        assert_eq!(merged.diagnostics.suture_count, 1);
        // Compaction leaves one plate owning every cell, with one motion to
        // its name.
        assert_eq!(merged.partition.plate_count, 1);
        assert!(merged.partition.cell_plates.iter().all(|&plate| plate == 0));
        assert_eq!(merged.kinematics.angular_velocities.len(), 1);
        merged.validate(&fixture.mesh).unwrap();

        // The larger plate absorbed the smaller, and the direction it goes in
        // is the area-weighted mean of the pair's. Only the direction: the
        // respeed that ends the same step sets the length from the merged
        // plate's own crust and trenches.
        let areas = fixture.partition.plate_areas(&fixture.mesh);
        let total = areas[0] + areas[1];
        let expected = (waiting.kinematics.angular_velocities[0] * (areas[0] / total) as f32
            + waiting.kinematics.angular_velocities[1] * (areas[1] / total) as f32)
            .normalized();
        assert!(
            (merged.kinematics.angular_velocities[0].normalized() - expected).length() < 1.0e-6,
            "{:?} against {expected:?}",
            merged.kinematics.angular_velocities[0]
        );
    }

    /// The base speed and the drift a merged plate keeps, which its every
    /// later respeed reads. Driven a substep at a time, because a whole run
    /// gives both plates the same base and could not tell a mean from either.
    #[test]
    fn a_suture_takes_the_area_weighted_mean_of_base_speed_and_drift() {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental; 2]);
        let (_, config, steps) = forced_suture_fixture();
        let colliding = fixture.boundaries.clone();
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), config);
        world.kinematics.base_speeds = vec![0.4, 0.9];
        world.drift_factors = vec![0.8, 1.2];

        for step in 0..steps {
            world.lifecycle(&colliding, step as i32);
        }

        // The one-cell plate is the smaller, so the plate around it absorbs.
        let areas = fixture.partition.plate_areas(&fixture.mesh);
        let weights = [
            areas[0] / (areas[0] + areas[1]),
            areas[1] / (areas[0] + areas[1]),
        ];
        let mean = |values: [f32; 2]| values[0] * weights[0] as f32 + values[1] * weights[1] as f32;
        assert_eq!(world.kinematics.base_speeds[0], mean([0.4, 0.9]));
        assert_eq!(world.drift_factors[0], mean([0.8, 1.2]));
    }

    /// Both halves of a rift are the parent's crust on the parent's base speed
    /// and the drift it had walked to, so the opening decides which way each
    /// goes and the respeed decides how fast.
    #[test]
    fn a_pair_that_stops_colliding_starts_its_collision_over() {
        // Driven a substep at a time with boundaries chosen per step, which a
        // whole run cannot do: the rule is about what one quiet step does to a
        // collision already under way.
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental; 2]);
        let (_, config, steps) = forced_suture_fixture();
        let colliding = fixture.boundaries.clone();
        let quiet = empty_boundaries(&fixture.mesh);
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), config);

        let collide = |world: &mut EvolvingWorld<'_>, step: usize| {
            world.lifecycle(&colliding, step as i32).suture_count
        };
        for step in 0..steps - 1 {
            assert_eq!(collide(&mut world, step), 0);
        }
        assert_eq!(world.lifecycle(&quiet, steps as i32).suture_count, 0);
        // The collision starts from nothing, so it takes the whole schedule
        // again rather than the one step it was short of.
        for step in 0..steps - 1 {
            assert_eq!(collide(&mut world, steps + 1 + step), 0);
        }
        assert_eq!(collide(&mut world, 2 * steps + 1), 1);
    }

    #[test]
    fn no_lifecycle_leaves_the_plate_set_the_run_started_with() {
        let fixture = evolution_fixture();
        let evolution = fixture.evolve(PlateEvolutionConfig {
            lifecycle: NO_LIFECYCLE,
            ..reference_evolution_config()
        });

        assert_eq!(
            evolution.kinematics.angular_velocities.len(),
            evolution.partition.plate_count
        );
        assert_eq!(evolution.diagnostics.rift_count, 0);
        assert_eq!(evolution.diagnostics.failed_rift_count, 0);
        assert_eq!(evolution.diagnostics.suture_count, 0);
        // Migration can wipe out a plate on its own, and compaction removes
        // every id that owns nothing however it was emptied.
        assert!(evolution.partition.plate_count <= fixture.partition.plate_count);
    }

    #[test]
    fn compaction_removes_the_ids_that_own_nothing_and_keeps_the_rest_in_order() {
        let fixture = evolution_fixture();
        let mut world = EvolvingWorld::new(
            &fixture.mesh,
            fixture.inputs(),
            reference_evolution_config(),
        );
        // An empty id in the middle of the set, the way a suture leaves one.
        // The reference partition seeds every plate, so this is the only one.
        const ABSORBED: usize = 1;
        for plate in world.partition.cell_plates.iter_mut() {
            if *plate == ABSORBED {
                *plate = 0;
            }
        }
        let plate_count = world.partition.plate_count;
        let motions = world.kinematics.angular_velocities.clone();
        let bases = world.kinematics.base_speeds.clone();
        // Distinct per plate, so a factor that moved to the wrong id shows.
        world.drift_factors = (0..plate_count).map(|plate| plate as f32).collect();

        world.compact();

        assert_eq!(world.partition.plate_count, plate_count - 1);
        world.partition.validate(&fixture.mesh).unwrap();
        let survivors: Vec<usize> = (0..plate_count).filter(|&p| p != ABSORBED).collect();
        let kept =
            |values: &[f32]| -> Vec<f32> { survivors.iter().map(|&plate| values[plate]).collect() };
        assert_eq!(
            world.kinematics.angular_velocities,
            survivors
                .iter()
                .map(|&plate| motions[plate])
                .collect::<Vec<_>>()
        );
        assert_eq!(world.kinematics.base_speeds, kept(&bases));
        assert_eq!(
            world.drift_factors,
            survivors
                .iter()
                .map(|&plate| plate as f32)
                .collect::<Vec<_>>()
        );
        for (cell, &plate) in fixture.partition.cell_plates.iter().enumerate() {
            let expected = match plate {
                ABSORBED => 0,
                plate if plate > ABSORBED => plate - 1,
                plate => plate,
            };
            assert_eq!(world.partition.cell_plates[cell], expected, "cell {cell}");
        }
    }

    #[test]
    fn rejects_invalid_lifecycle_configuration() {
        let fixture = evolution_fixture();
        let default = PlateLifecycleConfig::default();
        for (lifecycle, expected) in [
            (
                PlateLifecycleConfig {
                    rift_rate: -1.0,
                    ..default
                },
                PlateEvolutionError::InvalidRiftRate,
            ),
            (
                PlateLifecycleConfig {
                    rift_rate: f32::INFINITY,
                    ..default
                },
                PlateEvolutionError::InvalidRiftRate,
            ),
            (
                PlateLifecycleConfig {
                    rift_minimum_area_fraction: 1.5,
                    ..default
                },
                PlateEvolutionError::InvalidRiftAreaFraction,
            ),
            (
                PlateLifecycleConfig {
                    rift_minimum_area_fraction: f32::NAN,
                    ..default
                },
                PlateEvolutionError::InvalidRiftAreaFraction,
            ),
            (
                PlateLifecycleConfig {
                    rift_curvature: -0.5,
                    ..default
                },
                PlateEvolutionError::InvalidRiftCurvature,
            ),
            (
                PlateLifecycleConfig {
                    rift_curvature: f32::NAN,
                    ..default
                },
                PlateEvolutionError::InvalidRiftCurvature,
            ),
            (
                PlateLifecycleConfig {
                    rift_opening_speed: -1.0,
                    ..default
                },
                PlateEvolutionError::InvalidRiftOpeningSpeed,
            ),
            (
                PlateLifecycleConfig {
                    rift_opening_speed: f32::NAN,
                    ..default
                },
                PlateEvolutionError::InvalidRiftOpeningSpeed,
            ),
            (
                PlateLifecycleConfig {
                    suture_time: -1.0,
                    ..default
                },
                PlateEvolutionError::InvalidSutureTime,
            ),
            (
                PlateLifecycleConfig {
                    suture_time: f32::NAN,
                    ..default
                },
                PlateEvolutionError::InvalidSutureTime,
            ),
        ] {
            assert_eq!(
                evolve_plate_ownership(
                    &fixture.mesh,
                    fixture.inputs(),
                    PlateEvolutionConfig {
                        lifecycle,
                        ..reference_evolution_config()
                    }
                ),
                Err(expected),
                "{lifecycle:?}"
            );
        }
        // Infinity is how suturing is switched off, not an invalid time.
        assert!(
            evolve_plate_ownership(
                &fixture.mesh,
                fixture.inputs(),
                PlateEvolutionConfig {
                    lifecycle: PlateLifecycleConfig {
                        suture_time: f32::INFINITY,
                        ..default
                    },
                    ..reference_evolution_config()
                }
            )
            .is_ok()
        );
    }
}
