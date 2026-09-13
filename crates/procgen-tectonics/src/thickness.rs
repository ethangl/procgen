//! Crustal thickness: how a column gains parcels.
//!
//! A parcel of continent carries the original parcels it holds. Two rules
//! change that number and nothing else does. A continent arriving under
//! another continent at a trench merges into it, so a collision makes one
//! doubled column instead of a stack nothing reads. Compaction merges the
//! continent of a plate that has run out of cells into the column above it.
//!
//! Both exist to stop material disappearing. What a run conserves is the sum
//! of thickness over continental parcels, which is an integer and exact:
//! accretion merges two columns into one and compaction merges an orphan into
//! its cover, and neither creates or destroys any.
//!
//! Nothing reads the number as elevation. `cell_thickness` is a record of
//! where continents have collided, which the viewer draws as an overlay.

use crate::{
    BoundaryClassification,
    step::EvolvingWorld,
    transport::{Particle, cell_thickness},
};

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
        NO_LIFECYCLE, NO_POLE_DRIFT, empty_boundaries, one_arrival, two_plate_fixture,
    };
    use crate::{BoundaryClass, CrustClass, PlateEvolutionConfig, TRANSPORT_REACH_HOPS};

    /// A world that moves nothing, so that a test sees the rule it is about
    /// and nothing else.
    fn still_config() -> PlateEvolutionConfig {
        PlateEvolutionConfig {
            step_duration: 0.0,
            pole_drift: NO_POLE_DRIFT,
            lifecycle: NO_LIFECYCLE,
            ..PlateEvolutionConfig::default()
        }
    }

    /// A world where nothing converges has nothing to merge, and a transport
    /// over it leaves every column holding exactly what it held.
    #[test]
    fn a_still_world_with_no_boundaries_moves_no_thickness() {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental; 2]);
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), still_config());
        let cell = fixture.mesh.edges[0].cells[0];
        world.particles[world.cell_winner[cell]].thickness = 5;
        let total = world.continental_thickness();

        let counts = world.transport(&empty_boundaries(&fixture.mesh), 0.0);

        assert_eq!(counts.accreted_particle_count, 0);
        assert_eq!(world.continental_thickness(), total);
        assert_eq!(world.particles[world.cell_winner[cell]].thickness, 5);
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
