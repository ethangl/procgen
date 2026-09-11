//! The two events that change the plate set during a run.
//!
//! Everything else evolution does moves ownership between a fixed set of
//! plates. Here the set itself changes: a large continental plate rifts along
//! a fresh crack arc, and two continental plates that have collided long
//! enough suture into one. Both happen at the end of a step, after the poles
//! drift and before the next boundary classification, so the next step's
//! boundaries are the ones the new plate set implies.
//!
//! **Rifting.** Each step every plate whose continental area exceeds
//! `rift_minimum_area_fraction` of the sphere rolls a hashed draw against
//! `rift_rate * step_duration`. A plate that rifts walks one arc of
//! [`crate::cracks`] from a hashed cell of its own, in both directions, and
//! the walk stops as soon as it leaves the plate — so the arc runs from
//! boundary to boundary and the cells it crossed are the rift's wall. The
//! non-wall cells split into connected components, and the two largest of
//! those the arc runs alongside are the halves; everything else, the wall
//! included, joins the half it shares the most edges with, the same adoption
//! rule the crack pattern uses, so both halves stay connected and a sliver
//! goes to the half it actually touches. Fewer than two components beside the
//! arc means the rift separated nothing, and the plate is left alone. The
//! larger half keeps the plate's id and the smaller becomes a new plate. Both
//! halves are continental and both keep the birth and deformation their cells
//! carried: a rift is a line in the crust, and the divergent boundary it
//! becomes makes oceanic crust through the existing rebirth rule over the
//! steps that follow.
//!
//! Both halves start from the parent's rotation vector plus and minus an
//! opening term along `n x m`, where `n` is the normal of the great circle the
//! wall lies closest to and `m` is the wall's mean center. That axis is the
//! one whose velocity at the rift is normal to the rift, so the relative
//! motion across the new boundary is pure opening and it classifies divergent.
//! An arc that closed on itself is the one case with no such axis: its wall's
//! mean center is the sphere's own. That rift fails too.
//!
//! **Suturing.** Each step counts, for every adjacent pair of plates, the
//! shared edges that are convergent with continental crust on both sides. A pair at or above `suture_minimum_shared_edges` grows its collision
//! time by the step; a pair below it starts over, the same convention the
//! closing debt uses for an edge that stopped converging. At `suture_time` the
//! plate with more area absorbs the other, taking the area-weighted mean of
//! the two rotation vectors, and the absorbed id is left owning nothing.
//!
//! **Compaction.** The run ends by removing every plate id that owns no cell
//! — the ones suturing emptied and the ones migration did — and remapping
//! ownership and kinematics to `0..live_count` in id order, so every plate
//! identity a consumer sees owns cells and no id means the same plate either
//! side of a run.
//!
//! Determinism: the arc walk, connected components, integer hashes, edge
//! tallies, and area sums are all exact, and the only floats that decide an
//! integer here are area comparisons and the plane fit, which use add,
//! multiply, and square root alone. Nothing on the path calls libm, so the
//! plate ids and the boundary classes the new rotation vectors decide stay
//! bit-identical across machines.

use crate::{
    BoundaryClass, BoundaryClassification, CrustClass, PlateEvolutionError,
    cracks::{Cracks, adopt_unassigned_cells},
    step::EvolvingWorld,
};
use procgen_core::{RandomStream, Vec3, random_streams::PLATE_RIFT};
use procgen_sphere_mesh::{SphereMesh, connected_components};
use std::collections::BTreeMap;

/// Fraction of the sphere's radius the wall's mean center must clear for the
/// halves to have a direction to part in. An arc spanning `2θ` of a great
/// circle has its mean center at about `R sin(θ) / θ`, which reaches a tenth
/// of the radius only past about nine tenths of a full circle: the bound fires
/// on an arc that has closed on itself, where the mean center is the sphere's
/// own and the direction is undefined, and on nothing else.
const MINIMUM_WALL_CENTER_FRACTION: f32 = 0.1;

/// Draws one plate takes from the rift stream in one step: the roll, the start
/// cell among the plate's own cells, and the seed the arc walk bends with. The
/// item coordinate names the plate, so the step takes its own block.
const RIFT_DRAWS_PER_STEP: u64 = 3;

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
    /// so it has to clear the migration minimum to open anything.
    pub rift_opening_speed: f32,
    /// Model time a continental pair must stay in collision before it merges.
    /// Infinity never merges anything.
    pub suture_time: f32,
    /// Shared convergent continental edges a pair needs before its collision
    /// time grows at all. It is a count rather than a length so that the test
    /// stays an integer on any mesh.
    pub suture_minimum_shared_edges: usize,
}

impl Default for PlateLifecycleConfig {
    fn default() -> Self {
        Self {
            // `3.0 * 0.014` is a chance of one in twenty-four a step per
            // eligible plate: with the four plates the minimum area below
            // makes eligible, the viewer's defaults break one up over fifteen
            // steps and two over thirty. It is swept against that minimum
            // rather than alone, because the two together decide the count:
            // at 4.5 the same four plates give three rifts over fifteen steps
            // and at 2.25 they give none, the draws being hashed and so lumpy
            // rather than smooth. A half that a rift leaves is usually too
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
            // A third of the default maximum angular speed: enough for the new
            // boundary to clear the default minimum convergence on its own,
            // little enough that the halves stay part of the flow field's
            // pattern.
            rift_opening_speed: 0.33,
            // Eight default steps. Together with the edge count below, the
            // viewer's defaults suture once over a nine- or fifteen-step run
            // and three times over thirty.
            suture_time: 8.0 * crate::field::DEFAULT_STEP_DURATION,
            // About a fifth of a default-mesh plate's perimeter, which is a
            // collision front rather than two plates meeting at a corner. It
            // is the knob that matters: at eight shared edges the viewer's
            // defaults suture six times over fifteen steps, at sixteen three
            // times, and at twenty once.
            suture_minimum_shared_edges: 20,
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
    /// `boundaries` are the boundaries the step began with, which is what the
    /// closing debt accumulates against too.
    pub(crate) fn lifecycle(
        &mut self,
        boundaries: &BoundaryClassification,
        step: i32,
    ) -> LifecycleEvents {
        let mut events = self.rift(step);
        events.suture_count = self.suture(boundaries);
        events
    }

    fn rift(&mut self, step: i32) -> LifecycleEvents {
        let config = self.config.lifecycle;
        let probability = config.rift_rate * self.config.step_duration;
        let mut events = LifecycleEvents::default();
        if probability <= 0.0 {
            return events;
        }

        let minimum_area = f64::from(config.rift_minimum_area_fraction) * self.mesh.total_area();
        // A plate can carry both crusts, so what has to clear the minimum is
        // the continental area a rift arc could separate, not the plate's
        // whole extent. The arc itself still walks the whole plate.
        let mut continental_areas = vec![0.0; self.partition.plate_count];
        let cell_crust = self.cell_crust();
        for (cell, &plate) in self.partition.cell_plates.iter().enumerate() {
            if cell_crust.class(cell) == CrustClass::Continental {
                continental_areas[plate] += f64::from(self.mesh.cell_areas[cell]);
            }
        }
        // Taken before any split, so a half a rift just made cannot rift
        // again in the same step.
        let eligible: Vec<usize> = (0..self.partition.plate_count)
            .filter(|&plate| continental_areas[plate] > minimum_area)
            .collect();

        let stream = RandomStream::new(self.config.seed, PLATE_RIFT);
        let sample = step as u64 * RIFT_DRAWS_PER_STEP;
        for plate in eligible {
            let item = plate as u64;
            if stream.unit_f32(item, sample) >= probability {
                continue;
            }
            let split = self.split_plate(
                plate,
                stream.sample_u64(item, sample + 1),
                stream.sample_u64(item, sample + 2),
            );
            events.rift_count += usize::from(split);
            events.failed_rift_count += usize::from(!split);
        }
        events
    }

    /// Walks one rift arc across `plate` and splits what it separated in two,
    /// returning whether it did.
    ///
    /// A rift fails when the arc left fewer than two pieces alongside itself,
    /// or when its wall spanned no plane and so gave the halves no direction
    /// to part in. The plate is then left exactly as it was.
    fn split_plate(&mut self, plate: usize, start_key: u64, walk_seed: u64) -> bool {
        let mesh = self.mesh;
        let mut halves = vec![None; mesh.cell_count()];
        // The wall twice over: in walk order, whose first entry is the cell
        // the arc started from and anchors the plane the halves part across,
        // and as a membership test the adjacency scans below need.
        let mut on_wall = vec![false; mesh.cell_count()];
        let wall;
        {
            let owner = &self.partition.cell_plates;
            let owned: Vec<usize> = (0..mesh.cell_count())
                .filter(|&cell| owner[cell] == plate)
                .collect();
            let start = owned[(start_key % owned.len() as u64) as usize];
            let mut cracks = Cracks::new(mesh, walk_seed, self.config.lifecycle.rift_curvature);
            wall = cracks.walk_arc(0, start, |cell| owner[cell] == plate);
            for &cell in &wall {
                on_wall[cell] = true;
            }

            let components = connected_components(
                mesh,
                |cell| owner[cell] == plate && !on_wall[cell],
                |_, _| true,
            );
            // Only a component the arc runs alongside can be a half. A plate
            // that evolution has left in disconnected pieces has components
            // the arc never reached, and making one of those a half would
            // leave the rift with nothing to open along.
            let mut sides: Vec<usize> = (0..components.len())
                .filter(|&component| {
                    components[component].iter().any(|&cell| {
                        mesh.cell_corners(cell)
                            .iter()
                            .any(|corner| on_wall[corner.neighbor])
                    })
                })
                .collect();
            if sides.len() < 2 {
                return false;
            }
            // Largest by area first, ties to the component holding the lower
            // cell id, which is the order `connected_components` returns.
            let areas: Vec<f64> = components.iter().map(|cells| area(mesh, cells)).collect();
            sides.sort_by(|&left, &right| {
                areas[right].total_cmp(&areas[left]).then(left.cmp(&right))
            });
            for (half, &component) in sides[..2].iter().enumerate() {
                for &cell in &components[component] {
                    halves[cell] = Some(half);
                }
            }
            // Wall cells and the components the arc left over both join the
            // half they touch most. A cell neither half ever reaches shares no
            // edge with either, so the tie leaves it with half zero.
            adopt_unassigned_cells(mesh, &mut halves, |cell| owner[cell] == plate);
            for cell in owned {
                halves[cell].get_or_insert(0);
            }
        }

        let mut cells: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
        for (cell, half) in halves.iter().enumerate() {
            if let Some(half) = half {
                cells[*half].push(cell);
            }
        }
        let Some(opening) = opening_rotation(
            mesh,
            &wall,
            &on_wall,
            &cells[0],
            self.config.lifecycle.rift_opening_speed,
        ) else {
            return false;
        };

        // The larger half keeps the plate's id, so the ids already reported
        // stay on the larger mass.
        let keeps = usize::from(area(mesh, &cells[1]) > area(mesh, &cells[0]));
        let new_plate = self.partition.plate_count;
        for &cell in &cells[1 - keeps] {
            self.partition.cell_plates[cell] = new_plate;
        }
        self.partition.plate_count += 1;

        let parent = self.kinematics.angular_velocities[plate];
        let rotations = [parent + opening, parent - opening];
        self.kinematics.angular_velocities[plate] = rotations[keeps];
        self.kinematics
            .angular_velocities
            .push(rotations[1 - keeps]);
        // Each half is a plate that has just come into being, so the drift
        // band that follows is around the speed it parted with rather than
        // around the parent's.
        self.starting_speeds[plate] = rotations[keeps].length();
        self.starting_speeds.push(rotations[1 - keeps].length());
        true
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
            .filter(|&(_, &count)| count >= config.suture_minimum_shared_edges)
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
    /// becomes the area-weighted mean of the two, and the absorbed id is left
    /// owning nothing until [`Self::compact`] removes it.
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
        let weight = |plate: usize| (areas[plate] / total) as f32;
        let merged = self.kinematics.angular_velocities[absorber] * weight(absorber)
            + self.kinematics.angular_velocities[absorbed] * weight(absorbed);
        self.kinematics.angular_velocities[absorber] = merged;
        // The merged plate starts again from the motion the merge gave it.
        // Left at the absorber's original speed, the drift band would snap
        // the mean straight back and undo the merge.
        self.starting_speeds[absorber] = merged.length();
        for plate in self.partition.cell_plates.iter_mut() {
            if *plate == absorbed {
                *plate = absorber;
            }
        }
        self.collisions
            .retain(|&(low, high), _| low != absorbed && high != absorbed);
    }

    /// Removes every plate id that owns no cell, remapping ownership, the
    /// rotation vectors, and the plate classes to `0..live_count` in id order.
    ///
    /// Suturing empties the id it absorbs, and migration can take the last
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
        self.kinematics.angular_velocities = live
            .iter()
            .map(|&plate| self.kinematics.angular_velocities[plate])
            .collect();
        self.starting_speeds = live
            .iter()
            .map(|&plate| self.starting_speeds[plate])
            .collect();
        self.partition.plate_count = live.len();
    }
}

fn area(mesh: &SphereMesh, cells: &[usize]) -> f64 {
    cells
        .iter()
        .map(|&cell| f64::from(mesh.cell_areas[cell]))
        .sum()
}

/// The rotation the first half gains away from the rift; the second half gains
/// its negation. `None` means the wall gives no direction to part in: it spans
/// no plane, which only a wall of one distinct cell can do, or it has closed
/// on itself, which leaves its mean center at the sphere's own.
///
/// `n x m`, for the wall's plane normal `n` and its mean center `m`, is the
/// rotation whose velocity at the rift is normal to the rift:
/// `(n x m) x m = m (n . m) - n (m . m)`, and the wall's cells lie in their own
/// plane, so `n . m` is nearly zero and the velocity at `m` is `-n |m|^2` —
/// along the plane's normal and along nothing else. Adding that rotation to
/// one half and subtracting it from the other therefore makes the relative
/// velocity across the wall pure opening, which is what classifies the new
/// boundary divergent.
///
/// `n` comes off a cross product, so its sign is arbitrary. It is fixed here
/// against the side of the plane the first half's own rift front sits on, and
/// that half takes the rotation carrying it further that way.
///
/// The plane is the wall's first cell crossed with the wall cell most nearly a
/// quarter turn from it, which is the plane the crack test fits to a straight
/// arc. Add, multiply, and square root only, so no libm call reaches the
/// rotation vectors this sets and the boundary classes they decide stay exact.
fn opening_rotation(
    mesh: &SphereMesh,
    wall: &[usize],
    on_wall: &[bool],
    first_half: &[usize],
    speed: f32,
) -> Option<Vec3> {
    let first = mesh.cell_centers[wall[0]];
    let normal = wall[1..]
        .iter()
        .map(|&cell| first.cross(mesh.cell_centers[cell]))
        .max_by(|left, right| left.length_squared().total_cmp(&right.length_squared()))
        .filter(|normal| normal.length_squared() > 0.0)?
        .normalized();

    let center = |cells: &[usize]| {
        cells
            .iter()
            .fold(Vec3::ZERO, |total, &cell| total + mesh.cell_centers[cell])
            * (cells.len() as f32).recip()
    };
    let middle = center(wall);
    if middle.length() <= MINIMUM_WALL_CENTER_FRACTION * mesh.radius {
        return None;
    }
    // A half is a component the arc runs alongside, so its front is never
    // empty.
    let front: Vec<usize> = first_half
        .iter()
        .copied()
        .filter(|&cell| {
            mesh.cell_corners(cell)
                .iter()
                .any(|corner| on_wall[corner.neighbor])
        })
        .collect();
    let outward = if normal.dot(center(&front)) <= 0.0 {
        speed
    } else {
        -speed
    };
    Some(normal.cross(middle).normalized() * outward)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::EvolutionFixture;
    use crate::test_support::{
        NO_LIFECYCLE, closed_arc_rift_fixture, empty_boundaries, evolution_fixture,
        failed_rift_fixture, forced_rift_fixture, forced_suture_fixture, half_oceanic_rift_fixture,
        reference_evolution_config, two_plate_fixture,
    };
    use crate::{PlateEvolution, PlateEvolutionConfig, evolve_plate_ownership};
    use procgen_sphere_mesh::connected_components;

    /// The plate the forced-rift fixture splits and the id its larger half
    /// keeps, and the id its smaller half takes.
    const HALVES: [usize; 2] = [0, 2];

    /// Edges of the boundary a rift opened: the ones with one half either side.
    fn rift_edges(fixture: &EvolutionFixture, evolution: &PlateEvolution) -> Vec<usize> {
        (0..fixture.mesh.edge_count())
            .filter(|&edge| {
                let plates = fixture.mesh.edges[edge]
                    .cells
                    .map(|cell| evolution.partition.cell_plates[cell]);
                plates[0] != plates[1] && plates.iter().all(|plate| HALVES.contains(plate))
            })
            .collect()
    }

    #[test]
    fn a_forced_rift_splits_one_plate_into_two_connected_continental_halves() {
        let (fixture, config) = forced_rift_fixture();
        let evolution = fixture.evolve(PlateEvolutionConfig {
            step_count: 1,
            ..config
        });

        assert_eq!(evolution.diagnostics.rift_count, 1);
        assert_eq!(evolution.diagnostics.failed_rift_count, 0);
        assert_eq!(
            evolution.partition.plate_count,
            fixture.partition.plate_count + 1
        );
        evolution.validate(&fixture.mesh).unwrap();

        // Each half is one connected piece and together they are exactly the
        // cells the parent owned.
        let mut covered = Vec::new();
        for half in HALVES {
            let pieces = connected_components(
                &fixture.mesh,
                |cell| evolution.partition.cell_plates[cell] == half,
                |_, _| true,
            );
            assert_eq!(pieces.len(), 1, "half {half} is empty or disconnected");
            covered.extend(pieces.into_iter().flatten());
        }
        covered.sort_unstable();
        let parent: Vec<usize> = (0..fixture.mesh.cell_count())
            .filter(|&cell| fixture.partition.cell_plates[cell] == HALVES[0])
            .collect();
        assert_eq!(covered, parent);
    }

    #[test]
    fn a_forced_rift_opens_a_ridge_that_makes_oceanic_crust() {
        let (fixture, config) = forced_rift_fixture();
        let opened = fixture.evolve(PlateEvolutionConfig {
            step_count: 1,
            ..config
        });
        let edges = rift_edges(&fixture, &opened);
        assert!(!edges.is_empty(), "the rift left no boundary");

        // The halves part across the wall's own plane, so the boundary opens
        // on the whole and reads as a ridge. It is not divergent edge by edge:
        // where the wall jogs, an edge's normal points along the arc rather
        // than across it, and a few edges shear or close instead.
        let closing: f32 = edges
            .iter()
            .map(|&edge| opened.boundaries.convergence(edge))
            .sum();
        assert!(closing < 0.0, "the rift boundary closed on the whole");
        let divergent = edges
            .iter()
            .filter(|&&edge| opened.boundaries.edge_classes[edge] == BoundaryClass::Divergent)
            .count();
        assert!(
            divergent * 2 > edges.len(),
            "only {divergent} of {} rift edges opened",
            edges.len()
        );

        // The ridge the rift opened is what makes the ocean, several steps on.
        // Only crust born during the run counts: the prior already dated the
        // oceanic plate the cap sits in.
        let run = fixture.evolve(config);
        let born: Vec<usize> = (0..fixture.mesh.cell_count())
            .filter(|&cell| run.cell_birth[cell].is_some_and(|birth| birth >= 0))
            .collect();
        assert!(!born.is_empty(), "the rift made no oceanic crust");
        for &cell in &born {
            assert!(
                run.cell_birth[cell].is_some_and(|birth| birth < config.step_count as i32),
                "cell {cell} was born during the run"
            );
            assert_eq!(run.cell_crust().class(cell), CrustClass::Oceanic);
        }
        // The only ridge in this world is the one the rift opened, so the new
        // crust came off it.
        assert!(
            born.iter().any(|&cell| edges
                .iter()
                .any(|&edge| { fixture.mesh.edges[edge].cells.contains(&cell) })),
            "no new crust sits on the rift itself"
        );
    }

    /// Rift eligibility is a plate's continental area, not its extent: half
    /// the cap's crust is ocean, so the plate still holds three fifths of the
    /// sphere and no longer holds the minimum in continent.
    #[test]
    fn a_plate_whose_continent_is_too_small_is_never_drawn_for() {
        let (fixture, config) = half_oceanic_rift_fixture();
        let minimum_area =
            f64::from(config.lifecycle.rift_minimum_area_fraction) * fixture.mesh.total_area();
        let areas = fixture.partition.plate_areas(&fixture.mesh);
        assert!(
            areas.iter().any(|&area| area > minimum_area),
            "a plate must still clear the minimum on total area"
        );
        assert!(
            fixture
                .crust
                .plate_continental_fraction(&fixture.mesh, &fixture.partition)
                .iter()
                .zip(&areas)
                .all(|(share, &area)| share * area <= minimum_area),
            "no plate may clear it on continental area"
        );

        let evolution = fixture.evolve(config);

        assert_eq!(evolution.diagnostics.rift_count, 0);
        assert_eq!(evolution.diagnostics.failed_rift_count, 0);
        assert_eq!(
            evolution.partition.plate_count,
            fixture.partition.plate_count
        );
        // The forced-rift fixture is this world with the whole cap
        // continental, and it splits on its first step.
        let (continental, continental_config) = forced_rift_fixture();
        assert_eq!(
            continental
                .evolve(continental_config)
                .diagnostics
                .rift_count,
            1
        );
    }

    #[test]
    fn a_rift_that_separates_nothing_leaves_the_plate_alone() {
        let (fixture, config) = failed_rift_fixture();
        let evolution = fixture.evolve(config);

        assert_eq!(evolution.diagnostics.rift_count, 0);
        assert_eq!(evolution.diagnostics.failed_rift_count, 1);
        assert_eq!(evolution.partition, fixture.partition);
        assert_eq!(evolution.kinematics, fixture.kinematics);
    }

    #[test]
    fn a_rift_arc_that_closes_on_itself_leaves_the_plate_alone() {
        // The arc meets no other plate, so both directions run their half
        // circle and the wall wraps the sphere. Its mean center is then the
        // sphere's own and the halves have no direction to part in, whatever
        // the arc did separate.
        let (fixture, config) = closed_arc_rift_fixture();
        let evolution = fixture.evolve(config);

        assert_eq!(evolution.diagnostics.rift_count, 0);
        assert_eq!(evolution.diagnostics.failed_rift_count, 1);
        assert_eq!(evolution.partition, fixture.partition);
        assert_eq!(evolution.kinematics, fixture.kinematics);
        assert!(
            evolution
                .kinematics
                .angular_velocities
                .iter()
                .all(|rotation| rotation.length().is_finite())
        );
    }

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

        // The larger plate absorbed the smaller, and its motion is the
        // area-weighted mean of the pair's.
        let areas = fixture.partition.plate_areas(&fixture.mesh);
        let total = areas[0] + areas[1];
        let expected = waiting.kinematics.angular_velocities[0] * (areas[0] / total) as f32
            + waiting.kinematics.angular_velocities[1] * (areas[1] / total) as f32;
        assert!(
            (merged.kinematics.angular_velocities[0] - expected).length() < 1.0e-6,
            "{:?} against {expected:?}",
            merged.kinematics.angular_velocities[0]
        );
    }

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
        let speeds = world.starting_speeds.clone();

        world.compact();

        assert_eq!(world.partition.plate_count, plate_count - 1);
        world.partition.validate(&fixture.mesh).unwrap();
        let survivors: Vec<usize> = (0..plate_count).filter(|&p| p != ABSORBED).collect();
        assert_eq!(
            world.kinematics.angular_velocities,
            survivors
                .iter()
                .map(|&plate| motions[plate])
                .collect::<Vec<_>>()
        );
        assert_eq!(
            world.starting_speeds,
            survivors
                .iter()
                .map(|&plate| speeds[plate])
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

    #[test]
    fn a_run_that_rifts_is_deterministic() {
        let (fixture, config) = forced_rift_fixture();
        let first = fixture.evolve(config);

        assert_eq!(first, fixture.evolve(config));
        assert!(first.diagnostics.rift_count > 0);
    }
}
