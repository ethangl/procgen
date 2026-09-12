//! Crustal thickness: how a column gains parcels, and how it spreads them.
//!
//! A parcel of continent carries the original parcels it holds. Three rules
//! change that number and nothing else does. A continent arriving under
//! another continent at a trench merges into it, so a collision makes one
//! doubled column instead of a stack nothing reads. Compaction merges the
//! continent of a plate that has run out of cells into the column above it.
//! And thickness flows sideways, one parcel at a time, so a column that keeps
//! receiving crust becomes a plateau rather than a spike.
//!
//! The flow is what makes the source worth having. Without it every arriving
//! parcel stays in the cell it landed in: the deepest column of a 240-step
//! run reached 115 parcels while the largest connected run of thickened cells
//! held at about 34, and no value of
//! [`crate::BaseElevationConfig::thickness_uplift`] turned that into a
//! plateau — it turned it into cells at the elevation clamp. Real thick crust
//! flows. Tibet is flat because its lower crust spreads under its own weight,
//! and a plateau's extent is set by that spreading as much as by the
//! underthrust that fed it.
//!
//! What a run conserves is the sum of thickness over continental parcels,
//! which is an integer and exact: accretion merges two columns into one,
//! compaction merges an orphan into its cover, and flow moves a parcel from
//! one column to a neighbour. None of the three creates or destroys any.

use crate::{
    BoundaryClassification,
    step::EvolvingWorld,
    transport::{Particle, cell_thickness},
};

/// Parcels a column may hand out in one pass.
///
/// Two, and the bound is a proof rather than a preference. Let `Q` be the sum
/// of squared thickness over continental parcels. Every cell asks at most one
/// neighbour, so a column receives at most one parcel and gives at most
/// `MAXIMUM_GRANTS`; every transfer runs from a column at `t` to one at
/// `s <= t - GIVING_GAP`. Writing `T` for the transfers a pass makes,
///
/// ```text
/// dQ = sum (s - g + r)^2 - s^2
///    = 2 sum_transfers (s_receiver - s_giver) + sum (r - g)^2
///   <= -4T + (T + 2T)  =  -T
/// ```
///
/// so a pass that moves anything strictly lowers `Q`. `Q` is a non-negative
/// integer, so the passes reach a fixed point, and at that point no cell has
/// a same-plate continental neighbour two or more parcels thicker than
/// itself: the crust is a plateau with a one-parcel rim.
///
/// The cap is what makes the middle term small enough. A column granting `k`
/// requests moves `Q` by at most `k(k - 3)`, which is negative at one and two
/// grants, zero at three, and positive from four: uncapped, a thick column
/// surrounded by thin ones would empty into them and the pass could grow `Q`
/// instead of shrinking it.
const MAXIMUM_GRANTS: usize = 2;

/// How much thicker a neighbour must be before a column pulls from it.
///
/// Two. At one, two adjacent columns differing by one would each pull from
/// the other for ever, swapping the same parcel back and forth: a pass that
/// reads before it writes has no order to break that tie with. Two leaves a
/// one-parcel rim, which is the flattest a lattice of whole parcels can be.
const GIVING_GAP: u32 = 2;

/// One column's request: the cell asking, and the neighbour it asks.
struct Request {
    receiver: usize,
    giver: usize,
}

impl EvolvingWorld<'_> {
    /// Whether the continent that arrived is thrust under the one already
    /// here, which merges the two into one column twice as thick.
    ///
    /// The test is the trench rule's, on continental material instead of
    /// ocean floor: a parcel of another plate's continent, landing inside a
    /// plate that is converging on it. India goes under Asia and the pair is
    /// one crust from then on, so the arrival stops being a parcel nothing
    /// reads and becomes the thickness that floats a plateau.
    ///
    /// The winner of a cell holding continental material is continental,
    /// because [`crate::crust::material_order`] puts continent over floor,
    /// and among two continents the cell's own plate keeps it. So the
    /// incumbent continent stays on top and the boundary stays where the
    /// suture will form, which is what makes the plateau grow behind the
    /// front rather than in it.
    ///
    /// Everything else stacks as before. Two parcels of one plate are that
    /// plate's own crowding rather than a collision, and a parcel that
    /// crossed a transform or a ridge by lattice jitter is not colliding with
    /// anything.
    pub(crate) fn accretes(
        &self,
        cell: usize,
        ring: &[usize],
        boundaries: &BoundaryClassification,
        owners: &[usize],
        loser: usize,
        winner: usize,
    ) -> bool {
        let loser = self.particles[loser];
        if !loser.is_continental() || loser.plate == self.particles[winner].plate {
            return false;
        }
        debug_assert!(
            self.particles[winner].is_continental(),
            "continental material outranks ocean floor, so it cannot lose a cell to it"
        );
        self.converges_on(cell, ring, boundaries, owners, loser.plate)
    }

    /// Hands each accreting winner the thickness of the parcels it took.
    ///
    /// The loser is already marked removed, so this is the whole of the
    /// merge: one column of crust where there were two, holding both parcels
    /// and standing where the incumbent stood. The winner keeps its own
    /// deformation, because the loser's relief was paint on material that is
    /// now underneath.
    pub(crate) fn merge_accreted(&mut self, accreted: &[(usize, u32)]) {
        for &(winner, thickness) in accreted {
            self.particles[winner].thickness += thickness;
        }
    }

    /// Spreads thickness one parcel at a time from the thickest columns to
    /// their thinner neighbours, and returns how many parcels moved.
    ///
    /// Every cell whose winner is continental asks its thickest same-plate
    /// continental neighbour for a parcel, if that neighbour stands at least
    /// [`GIVING_GAP`] parcels above it; equally thick neighbours lose the tie
    /// to the lower cell id. A column granting more requests than
    /// [`MAXIMUM_GRANTS`] serves the lowest cell ids and refuses the rest.
    ///
    /// Read before the pass and written after it, so the update is
    /// simultaneous like every other substep and a parcel cannot cross two
    /// cells in one step.
    ///
    /// Continental only, same plate only, and only the columns that cells
    /// actually read. A plateau cannot spread into ocean floor, cannot cross
    /// a plate boundary before a suture has made the two plates one — after
    /// which it spreads across the old boundary, which is what a suture is —
    /// and cannot reach a covered parcel that no cell reads.
    pub(crate) fn flow_thickness(&mut self, winners: &[Option<usize>]) -> usize {
        // The column each cell reads and what it holds, which is the whole of
        // what the pass decides from. A cell with no winner yet is one no
        // parcel reached; it has nothing to give and nothing to stand on.
        let standing: Vec<Option<(usize, u32)>> = winners
            .iter()
            .map(|winner| {
                let winner = (*winner)?;
                let particle = self.particles[winner];
                particle
                    .is_continental()
                    .then_some((winner, particle.thickness))
            })
            .collect();

        let mut requests = Vec::new();
        for cell in 0..self.mesh.cell_count() {
            let Some((winner, thickness)) = standing[cell] else {
                continue;
            };
            let plate = self.particles[winner].plate;
            let mut best: Option<(usize, u32)> = None;
            for corner in self.mesh.cell_corners(cell) {
                let neighbor = corner.neighbor;
                let Some((other, other_thickness)) = standing[neighbor] else {
                    continue;
                };
                if self.particles[other].plate != plate {
                    continue;
                }
                // Ascending cell order, so the first of equally thick
                // neighbours is the lowest id and keeps the tie.
                if best.is_none_or(|(_, best_thickness)| other_thickness > best_thickness) {
                    best = Some((neighbor, other_thickness));
                }
            }
            if let Some((giver, giver_thickness)) = best
                && giver_thickness >= thickness + GIVING_GAP
            {
                requests.push(Request {
                    receiver: cell,
                    giver,
                });
            }
        }
        if requests.is_empty() {
            return 0;
        }

        // Requests are built in ascending receiver order, so counting grants
        // per giver as they are read serves the lowest cell ids first.
        let mut granted = vec![0usize; self.mesh.cell_count()];
        let mut transfers = 0;
        for request in &requests {
            if granted[request.giver] == MAXIMUM_GRANTS {
                continue;
            }
            granted[request.giver] += 1;
            transfers += 1;
            let (giver, _) = standing[request.giver].expect("a giver is a column a cell reads");
            let (receiver, _) = standing[request.receiver].expect("a receiver is one too");
            debug_assert!(
                self.particles[giver].thickness > 1,
                "a column {GIVING_GAP} parcels above another holds at least {GIVING_GAP} plus one"
            );
            self.particles[giver].thickness -= 1;
            self.particles[receiver].thickness += 1;
        }
        transfers
    }

    /// Merges every continental parcel `dying` marks into the column its cell
    /// reads, and returns how many merged.
    ///
    /// A parcel of a plate that owns no cell is material standing under
    /// somebody else's column, so it joins that column exactly as a collision
    /// would rather than being dropped with its plate. The column a cell
    /// reads can itself belong to a dying plate — the parcel a cell samples
    /// need not stand in it, and the plate that owns a cell need not be the
    /// plate of every parcel in it — so those columns are relabelled to the
    /// plate that owns their cell first. That conserves the same thickness by
    /// a different route and leaves every cell reading a parcel that lives.
    pub(crate) fn accrete_orphans(&mut self, dying: &impl Fn(&Particle) -> bool) -> usize {
        let orphans: Vec<usize> = (0..self.particles.len())
            .filter(|&index| {
                let particle = self.particles[index];
                particle.is_continental() && dying(&particle)
            })
            .collect();
        if orphans.is_empty() {
            return 0;
        }

        for cell in 0..self.mesh.cell_count() {
            let winner = self.cell_winner[cell];
            if dying(&self.particles[winner]) {
                self.particles[winner].plate = self.partition.cell_plates[cell];
            }
        }

        let mut accreted = 0;
        for orphan in orphans {
            // Relabelled just above, because this parcel is the column its
            // own cell reads. It stays as itself rather than merging.
            if !dying(&self.particles[orphan]) {
                continue;
            }
            let target = self.cell_winner[self.particles[orphan].cell];
            debug_assert!(
                !dying(&self.particles[target]),
                "every column a cell reads was relabelled to the plate that owns that cell"
            );
            let merged = self.particles[orphan].thickness;
            self.particles[target].thickness += merged;
            accreted += 1;
        }
        // The columns that grew are read by the cells pointing at them, and
        // the projection that would normally write those cells has already
        // run, so the cells are brought up to date here.
        for cell in 0..self.mesh.cell_count() {
            self.cell_thickness[cell] = cell_thickness(self.particles[self.cell_winner[cell]]);
        }
        accreted
    }

    /// Every original parcel of continent the run still holds, wherever it
    /// lies and whatever it has merged into.
    ///
    /// This is what a run conserves. The particle count is not, since a
    /// collision merges two parcels into one column; the sum of what those
    /// columns hold is the same integer before and after, so it is exact and
    /// pinnable rather than a tolerance.
    pub(crate) fn continental_thickness(&self) -> u32 {
        self.particles
            .iter()
            .filter(|particle| particle.is_continental())
            .map(|particle| particle.thickness)
            .sum()
    }

    /// The sum of squared thickness over continental parcels, which the flow
    /// pass lowers every time it moves anything. See [`MAXIMUM_GRANTS`].
    ///
    /// A `u64` because it is a sum of squares over tens of thousands of
    /// parcels, and the point of it is that it is exact.
    ///
    /// Nothing in a run reads it: it is the quantity the convergence argument
    /// is about, so the tests that hold the pass to that argument are its
    /// only callers.
    #[cfg(test)]
    pub(crate) fn squared_continental_thickness(&self) -> u64 {
        self.particles
            .iter()
            .filter(|particle| particle.is_continental())
            .map(|particle| u64::from(particle.thickness) * u64::from(particle.thickness))
            .sum()
    }

    /// The deepest continental column the run holds, in original parcels.
    pub(crate) fn maximum_thickness(&self) -> u32 {
        self.particles
            .iter()
            .filter(|particle| particle.is_continental())
            .map(|particle| particle.thickness)
            .max()
            .unwrap_or(0)
    }

    /// Cells whose crust is more than one parcel thick, which is the extent
    /// of the collision plateaus a run built.
    pub(crate) fn thickened_cell_count(&self) -> usize {
        self.cell_thickness
            .iter()
            .filter(|&&thickness| thickness > 1)
            .count()
    }

    /// Continental particles no cell reads, because another parcel of
    /// continent shares the cell with them.
    ///
    /// Continental material outranks every parcel of ocean floor, so a cell
    /// holding any of it reads continental and the cells holding some are
    /// exactly the continental cells the material itself accounts for. What is
    /// left over is covered, and nothing spreads it back out: it is the gap
    /// between the material a run conserves and the raster it can show.
    pub(crate) fn covered_continental_particle_count(&self) -> usize {
        let mut occupied = vec![false; self.mesh.cell_count()];
        let mut continental = 0;
        for particle in &self.particles {
            if particle.is_continental() {
                continental += 1;
                occupied[particle.cell] = true;
            }
        }
        continental - occupied.iter().filter(|held| **held).count()
    }

    /// Continental particles lying in a cell their own plate does not own,
    /// which is the part of
    /// [`Self::covered_continental_particle_count`] that a collision stacked
    /// rather than that one plate's own material crowded together.
    pub(crate) fn foreign_continental_particle_count(&self) -> usize {
        self.particles
            .iter()
            .filter(|particle| {
                particle.is_continental()
                    && self.partition.cell_plates[particle.cell] != particle.plate
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        NO_LIFECYCLE, NO_POLE_DRIFT, empty_boundaries, forced_suture_fixture, one_arrival,
        two_plate_fixture,
    };
    use crate::{BoundaryClass, CrustClass, PlateEvolutionConfig, TRANSPORT_REACH_HOPS};

    /// A world that moves nothing, so that a test about the flow pass sees
    /// the flow pass and nothing else.
    fn still_config() -> PlateEvolutionConfig {
        PlateEvolutionConfig {
            step_duration: 0.0,
            pole_drift: NO_POLE_DRIFT,
            lifecycle: NO_LIFECYCLE,
            ..PlateEvolutionConfig::default()
        }
    }

    /// Runs the flow pass over every cell's own parcel, which is what a still
    /// world's winners are, and returns how many parcels moved.
    fn flow_once(world: &mut EvolvingWorld<'_>) -> usize {
        let winners: Vec<Option<usize>> = (0..world.mesh.cell_count())
            .map(|cell| Some(world.cell_winner[cell]))
            .collect();
        let moved = world.flow_thickness(&winners);
        for cell in 0..world.mesh.cell_count() {
            world.cell_thickness[cell] = cell_thickness(world.particles[world.cell_winner[cell]]);
        }
        moved
    }

    /// Every same-plate continental neighbour pair the world holds, as the
    /// pair of thicknesses the cells read.
    fn neighbor_pairs(world: &EvolvingWorld<'_>) -> Vec<(u32, u32)> {
        let mut pairs = Vec::new();
        for cell in 0..world.mesh.cell_count() {
            let here = world.particles[world.cell_winner[cell]];
            if !here.is_continental() {
                continue;
            }
            for corner in world.mesh.cell_corners(cell) {
                let there = world.particles[world.cell_winner[corner.neighbor]];
                if there.is_continental() && there.plate == here.plate {
                    pairs.push((here.thickness, there.thickness));
                }
            }
        }
        pairs
    }

    /// Flow moves a parcel from a column to a thinner neighbour and never
    /// creates or destroys one, and every pass that moves anything lowers the
    /// sum of squares the convergence argument is about.
    #[test]
    fn flow_conserves_the_sum_and_lowers_the_square() {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental; 2]);
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), still_config());
        // One deep column, the way a long collision leaves one.
        let source = fixture.mesh.edges[0].cells[0];
        world.particles[world.cell_winner[source]].thickness = 20;

        let total = world.continental_thickness();
        let mut square = world.squared_continental_thickness();
        for pass in 1..=40 {
            let moved = flow_once(&mut world);
            assert_eq!(
                world.continental_thickness(),
                total,
                "pass {pass} changed how much continental material exists"
            );
            let next = world.squared_continental_thickness();
            if moved > 0 {
                assert!(
                    next < square,
                    "pass {pass} moved {moved} parcels without lowering the square"
                );
            } else {
                assert_eq!(next, square, "a pass that moves nothing changes nothing");
            }
            square = next;
        }
    }

    /// The fixed point the square argument promises: a plateau with a
    /// one-parcel rim, reached and then held.
    #[test]
    fn a_column_spreads_into_a_plateau_and_stops() {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental; 2]);
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), still_config());
        let source = fixture.mesh.edges[0].cells[0];
        let source_plate = world.particles[world.cell_winner[source]].plate;
        world.particles[world.cell_winner[source]].thickness = 20;
        let total = world.continental_thickness();

        let settled = (1..=200)
            .find(|_| flow_once(&mut world) == 0)
            .expect("the square is a non-negative integer that every pass lowers");
        assert!(settled > 1, "the column must take more than one pass");
        assert_eq!(flow_once(&mut world), 0, "a fixed point must hold");
        assert_eq!(world.continental_thickness(), total);

        for (here, there) in neighbor_pairs(&world) {
            assert!(
                here.abs_diff(there) <= 1,
                "a settled plateau differs by at most one parcel: {here} against {there}"
            );
        }
        // The parcels went to the source's own plate and stayed connected to
        // it: the cells above one parcel form one region holding the source.
        let thick: Vec<usize> = (0..world.mesh.cell_count())
            .filter(|&cell| world.cell_thickness[cell] > 1)
            .collect();
        assert!(
            thick.len() > 1,
            "twenty parcels in one cell must reach more than that cell"
        );
        assert!(thick.contains(&source));
        for &cell in &thick {
            assert_eq!(
                world.particles[world.cell_winner[cell]].plate, source_plate,
                "cell {cell} is not the source's plate"
            );
        }
    }

    /// Ocean floor is not crust a plateau can spread into, and neither is a
    /// column of another plate until a suture has made the two one.
    #[test]
    fn flow_stays_inside_continental_crust_of_one_plate() {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental, CrustClass::Oceanic]);
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), still_config());
        // The one-cell oceanic plate, and the continental cell beside it.
        let ocean = fixture.mesh.edges[0].cells[1];
        let shore = fixture.mesh.edges[0].cells[0];
        world.particles[world.cell_winner[shore]].thickness = 20;
        let ocean_parcel = world.cell_winner[ocean];
        let before = world.particles[ocean_parcel].thickness;

        for _ in 0..40 {
            flow_once(&mut world);
        }

        assert_eq!(
            world.particles[ocean_parcel].thickness, before,
            "ocean floor took a parcel of continent"
        );
        assert_eq!(world.cell_thickness[ocean], 0);
    }

    /// Two continental plates side by side: nothing crosses the boundary
    /// until the suture merges them, and then it does.
    #[test]
    fn a_suture_lets_a_plateau_spread_across_the_old_boundary() {
        let (fixture, config, _) = forced_suture_fixture();
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), config);
        let [thick_cell, across] = fixture.mesh.edges[0].cells;
        let across_parcel = world.cell_winner[across];
        world.particles[world.cell_winner[thick_cell]].thickness = 20;
        assert_ne!(
            world.particles[world.cell_winner[thick_cell]].plate,
            world.particles[across_parcel].plate,
            "the two cells must start on different plates"
        );

        for _ in 0..10 {
            flow_once(&mut world);
        }
        assert_eq!(
            world.particles[across_parcel].thickness, 1,
            "a plateau crossed a plate boundary before the suture"
        );

        // The suture, forced by hand rather than by running the lifecycle, so
        // that this test is about the flow and not about when a pair merges.
        let absorbed = world.particles[across_parcel].plate;
        let absorber = world.particles[world.cell_winner[thick_cell]].plate;
        for particle in &mut world.particles {
            if particle.plate == absorbed {
                particle.plate = absorber;
            }
        }
        for plate in world.partition.cell_plates.iter_mut() {
            if *plate == absorbed {
                *plate = absorber;
            }
        }
        flow_once(&mut world);

        assert_eq!(
            world.particles[across_parcel].thickness, 2,
            "one plate is one crust, and a plateau spreads across the suture"
        );
    }

    /// The two rules that make the pass deterministic and bounded: a tie goes
    /// to the lower cell id, and a column serves at most two requests.
    #[test]
    fn ties_go_to_the_lower_id_and_a_column_grants_at_most_two() {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental; 2]);
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), still_config());

        // A column deep enough that every neighbour asks it, well away from
        // the one-cell plate so that every neighbour is its own plate's.
        let small = fixture.mesh.edges[0].cells[1];
        let giver = (0..fixture.mesh.cell_count())
            .find(|&cell| {
                cell != small
                    && fixture
                        .mesh
                        .cell_corners(cell)
                        .iter()
                        .all(|corner| corner.neighbor != small)
            })
            .expect("the large plate reaches further than one cell from the small one");
        world.particles[world.cell_winner[giver]].thickness = 20;
        let neighbors: Vec<usize> = fixture
            .mesh
            .cell_corners(giver)
            .iter()
            .map(|corner| corner.neighbor)
            .collect();

        assert!(
            neighbors.len() > MAXIMUM_GRANTS,
            "the test needs more requesters than the cap"
        );

        let moved = flow_once(&mut world);

        assert_eq!(moved, MAXIMUM_GRANTS, "a column may grant no more than two");
        assert_eq!(
            world.particles[world.cell_winner[giver]].thickness,
            20 - MAXIMUM_GRANTS as u32
        );
        let mut served: Vec<usize> = neighbors
            .iter()
            .copied()
            .filter(|&cell| world.cell_thickness[cell] > 1)
            .collect();
        served.sort_unstable();
        let mut expected = neighbors.clone();
        expected.sort_unstable();
        expected.truncate(MAXIMUM_GRANTS);
        assert_eq!(served, expected, "the lowest cell ids are the ones served");
    }

    /// A parcel no cell reads is under somebody else's column, so the flow
    /// neither takes from it nor gives to it.
    #[test]
    fn a_covered_parcel_neither_gives_nor_receives() {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental; 2]);
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), still_config());
        let cell = fixture.mesh.edges[0].cells[0];
        world.particles[world.cell_winner[cell]].thickness = 20;

        // A second continental parcel in the same cell, which no cell reads
        // because the cell's own parcel won it.
        let covered = world.particles.len();
        let mut buried = world.particles[world.cell_winner[cell]];
        buried.thickness = 7;
        world.particles.push(buried);

        for _ in 0..40 {
            flow_once(&mut world);
        }

        assert_eq!(
            world.particles[covered].thickness, 7,
            "a covered parcel took part in the flow"
        );
    }

    /// Nothing here should touch the boundary classification, but the flow
    /// runs inside a transport that reads one, so the still world's empty
    /// boundaries are what the other tests stand on.
    #[test]
    fn a_still_world_with_no_boundaries_moves_nothing_but_thickness() {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental; 2]);
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), still_config());
        let cell = fixture.mesh.edges[0].cells[0];
        world.particles[world.cell_winner[cell]].thickness = 5;
        let total = world.continental_thickness();

        let counts = world.transport(&empty_boundaries(&fixture.mesh), 0.0);

        assert_eq!(counts.accreted_particle_count, 0);
        assert!(counts.thickness_transfer_count > 0);
        assert_eq!(world.continental_thickness(), total);
    }

    /// The counterpart of the trench rule on continental material: the
    /// arrival goes under and the two become one column.
    #[test]
    fn a_continent_that_arrives_under_a_continent_thickens_it() {
        for hops in 1..=TRANSPORT_REACH_HOPS {
            let arrival = one_arrival(BoundaryClass::Convergent, hops, CrustClass::Continental);

            assert_eq!(arrival.counts.accreted_particle_count, 1, "{hops} hops in");
            assert_eq!(arrival.counts.subducted_particle_count, 0, "{hops} hops in");
            assert_eq!(
                arrival.landing_thickness, 2,
                "cell {} stands on both parcels",
                arrival.landing
            );
            assert_eq!(
                arrival.counts.collided_cell_count, 0,
                "a merge is not a stack for the collision count to record"
            );
        }
    }

    /// A continent that crossed a transform or a ridge is not colliding with
    /// anything, so it stacks as it always did rather than merging.
    #[test]
    fn a_continent_that_crossed_a_transform_does_not_thicken_anything() {
        for class in [BoundaryClass::Transform, BoundaryClass::Divergent] {
            for hops in 1..=TRANSPORT_REACH_HOPS {
                let arrival = one_arrival(class, hops, CrustClass::Continental);

                assert_eq!(
                    arrival.counts.accreted_particle_count, 0,
                    "{class:?}, {hops} hops"
                );
                assert_eq!(
                    arrival.landing_thickness, 1,
                    "{class:?}, {hops} hops: cell {} stands on one parcel",
                    arrival.landing
                );
                assert_eq!(
                    arrival.counts.collided_cell_count, 1,
                    "{class:?}, {hops} hops: the arrival must stack instead"
                );
            }
        }
    }

    /// A cell that reads a parcel lying in another cell reads the whole of
    /// it, thickness included: the three columns are projections of one
    /// parcel rather than three separate rules.
    #[test]
    fn an_empty_cell_carries_the_thickness_of_the_parcel_it_samples() {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental; 2]);
        let [emptied, _] = fixture.mesh.edges[0].cells;
        let incumbent = fixture.partition.cell_plates[emptied];
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), still_config());
        world.particles.remove(emptied);
        // Columns three parcels deep across the plate the emptied cell
        // belongs to, which is the material an empty cell prefers to sample.
        // Three rather than two so that the assertion below can be neither
        // the one parcel a lone column holds nor the zero a floor cell reads.
        for particle in &mut world.particles {
            if particle.plate == incumbent {
                particle.thickness = 3;
            }
        }

        let counts = world.transport(&fixture.boundaries, 1.0);

        assert_eq!(counts.sampled_cell_count, 1);
        assert_eq!(counts.born_particle_count, 0);
        assert_eq!(
            world.cell_thickness[emptied], 3,
            "the emptied cell reads the whole of the parcel it sampled"
        );
    }
}
