//! Conservative material transport: the particles a plate's crust lives in,
//! and the one substep that moves them.
//!
//! Ownership used to move cell to cell — a convergent edge paid a closing
//! debt and flipped the cell behind it, and every cell pulled what its
//! upstream neighbour carried once it had travelled a cell width. Neither
//! move conserves anything. A per-step permutation of cells along the flow
//! does not exist on this mesh: at the viewer's defaults a third of the cells
//! are pulled by two downstream cells and a third by none, so a third of the
//! material is duplicated and a third dropped every time a plate moves one
//! cell, and a single plate under a pure rotation with no boundaries at all
//! lost five percent of a continental cap over forty steps.
//!
//! So the material is points rather than cells. Each particle carries the two
//! fields a cell used to carry, rotates rigidly with its plate, and the cells
//! sample whatever lands in them. Conservation is then structural: the only
//! place a particle is created is a cell no particle reached, and the only
//! place one is destroyed is a trench. Continental material is never created
//! and never destroyed at all.
//!
//! What the cells still decide is ownership, because a cell holding several
//! particles has to answer with one of them. That resolution is where
//! subduction, collision, and the ridge live.

use crate::{BoundaryClass, BoundaryClassification, step::EvolvingWorld};
use procgen_core::Vec3;
use procgen_sphere_mesh::SphereMesh;
use std::cmp::Ordering;

/// Largest [`MaterialTransportConfig::gap_radius`] the search can honour. An
/// empty cell looks through its one- and two-hop rings, which reach about two
/// cell widths, so a radius beyond that is not a wider search but silent
/// false floor: the cell would accept material it never looks at. Evolution
/// rejects a larger radius and the viewer's slider stops here.
pub const MAX_GAP_RADIUS: f32 = 2.0;

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

/// One parcel of crust.
///
/// A particle is the thing that moves. Its plate says which rotation carries
/// it, and the two fields it holds are exactly the two a cell used to carry,
/// so a cell's answer is one particle's answer rather than an average.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Particle {
    /// Unit direction on the sphere. `locate_delaunay` takes a unit
    /// direction, so positions live on the unit sphere whatever the mesh's
    /// radius, and the one distance they are measured against is scaled to
    /// match.
    pub(crate) position: Vec3,
    pub(crate) plate: usize,
    /// Step at which this material was created; `None` is original
    /// continental crust, exactly as in the field a cell carries.
    pub(crate) birth: Option<i32>,
    pub(crate) deformation: f32,
    /// The Voronoi cell holding `position`, and the Delaunay triangle the
    /// last location walk ended in. Both follow from `position`; locating is
    /// a walk, and everything after the move asks for the cell, so the walk's
    /// answer is kept rather than recomputed.
    pub(crate) cell: usize,
    pub(crate) triangle: usize,
}

impl Particle {
    /// Crust class follows birth here exactly as it does on a cell: original
    /// crust is continental and anything a run made is ocean floor.
    pub(crate) fn is_continental(self) -> bool {
        self.birth.is_none()
    }
}

/// What one transport did. Every count is an event count, so a run sums them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TransportCounts {
    pub(crate) owner_change_count: usize,
    pub(crate) subducted_particle_count: usize,
    pub(crate) born_particle_count: usize,
    pub(crate) collided_cell_count: usize,
    pub(crate) maximum_collision_stack: usize,
    pub(crate) sampled_cell_count: usize,
}

/// Which particles lie in each cell, as one block per cell over one shared
/// index array. Built once per transport and read by every cell resolution.
struct Occupancy {
    starts: Vec<usize>,
    particles: Vec<usize>,
}

impl Occupancy {
    fn new(cell_count: usize, particles: &[Particle]) -> Self {
        let mut starts = vec![0; cell_count + 1];
        for particle in particles {
            starts[particle.cell + 1] += 1;
        }
        for cell in 0..cell_count {
            starts[cell + 1] += starts[cell];
        }
        let mut cursor = starts.clone();
        let mut indices = vec![0; particles.len()];
        for (index, particle) in particles.iter().enumerate() {
            indices[cursor[particle.cell]] = index;
            cursor[particle.cell] += 1;
        }
        Self {
            starts,
            particles: indices,
        }
    }

    /// The particles in `cell`, in ascending particle index.
    fn at(&self, cell: usize) -> &[usize] {
        &self.particles[self.starts[cell]..self.starts[cell + 1]]
    }
}

/// How much of a plate's material a lifecycle event takes with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MaterialScope {
    /// Every particle the plate holds, wherever it lies. A suture absorbs the
    /// whole of the plate it merges.
    WholePlate,
    /// Only what lies in a cell the receiving plate now owns. A rift gives the
    /// new half the material standing on it; a particle of a third plate
    /// stacked there is not the parent's to give away and keeps its own plate.
    OwnCells,
}

/// One transport's working state over the cells, which its four passes hand
/// between them.
///
/// `occupancy` is a snapshot taken before any pass runs, so a particle a pass
/// creates is invisible to every other cell and the update stays simultaneous.
/// `removed` is indexed by particle rather than by cell, so a pass that
/// pushes a particle pushes a `false` beside it.
struct Resolution {
    previous_owners: Vec<usize>,
    occupancy: Occupancy,
    winners: Vec<Option<usize>>,
    removed: Vec<bool>,
    counts: TransportCounts,
}

/// The mesh's cell centre as a unit direction, which is what a particle
/// position is.
fn cell_direction(mesh: &SphereMesh, cell: usize) -> Vec3 {
    mesh.cell_centers[cell] * mesh.radius.recip()
}

/// A Delaunay triangle incident to `cell`, which is where a particle placed
/// at the cell's own centre already sits.
fn incident_triangle(mesh: &SphereMesh, cell: usize) -> usize {
    mesh.cell_corners(cell)[0].vertex
}

/// The particle every cell starts with: the cell's own material, at its own
/// centre.
pub(crate) fn initial_particles(
    mesh: &SphereMesh,
    cell_plates: &[usize],
    cell_birth: &[Option<i32>],
) -> Vec<Particle> {
    (0..mesh.cell_count())
        .map(|cell| Particle {
            position: cell_direction(mesh, cell),
            plate: cell_plates[cell],
            birth: cell_birth[cell],
            deformation: 0.0,
            cell,
            triangle: incident_triangle(mesh, cell),
        })
        .collect()
}

impl EvolvingWorld<'_> {
    /// Moves every particle with its plate and resolves what each cell holds
    /// afterwards.
    ///
    /// `boundaries` are the boundaries the step began with, classified from
    /// the ownership this transport reads and replaces.
    pub(crate) fn transport(
        &mut self,
        boundaries: &BoundaryClassification,
        step: i32,
    ) -> TransportCounts {
        self.move_particles();
        self.resolve_cells(boundaries, step)
    }

    /// Rotates every particle rigidly about its plate's rotation vector and
    /// locates where it landed.
    ///
    /// The rotation splits the position into the part along the axis, which a
    /// rotation about that axis leaves alone, and the part across it, which
    /// turns through the step's angle toward the velocity direction. Both the
    /// turn and the length it preserves come out of add, multiply, and
    /// divide: the half-angle tangent stands in for its own angle, as the
    /// pole drift's does, and is short by four parts in ten thousand at the
    /// default step. Every particle of one plate goes through the same linear
    /// map, so the plate's material is rigid by construction rather than by
    /// tolerance.
    fn move_particles(&mut self) {
        let mesh = self.mesh;
        let duration = self.config.step_duration;
        let rotations = &self.kinematics.angular_velocities;
        for particle in &mut self.particles {
            let rotation = rotations[particle.plate];
            let speed = rotation.length();
            if speed == 0.0 || duration == 0.0 {
                continue;
            }
            let axis = rotation * speed.recip();
            let along = axis * particle.position.dot(axis);
            let across = particle.position - along;
            // `axis x position` is perpendicular to `across` and of its
            // length, which is what makes the turn preserve that length.
            let velocity = axis.cross(particle.position);
            let turned = across.rotated_toward(velocity, 0.5 * speed * duration);
            particle.position = (along + turned).normalized();

            let location = mesh.locate_delaunay(particle.position, particle.triangle);
            particle.triangle = location.triangle;
            particle.cell = location
                .cells
                .into_iter()
                .max_by(|&left, &right| {
                    particle
                        .position
                        .dot(mesh.cell_centers[left])
                        .total_cmp(&particle.position.dot(mesh.cell_centers[right]))
                })
                .expect("a Delaunay triangle has three corners");
        }
    }

    /// Gives every cell the particle that won it, subducts the losers a
    /// trench takes, and makes ocean floor where nothing arrived.
    ///
    /// Four passes over the cells, in this order, because each needs the whole
    /// of the one before it. Every cell reads the ownership the step began
    /// with, and the new ownership is written only once every cell has been
    /// decided, so the update stays simultaneous.
    fn resolve_cells(&mut self, boundaries: &BoundaryClassification, step: i32) -> TransportCounts {
        let mut resolution = Resolution {
            previous_owners: self.partition.cell_plates.clone(),
            occupancy: Occupancy::new(self.mesh.cell_count(), &self.particles),
            winners: vec![None; self.mesh.cell_count()],
            removed: vec![false; self.particles.len()],
            counts: TransportCounts::default(),
        };
        self.pick_winners(boundaries, &mut resolution);
        self.fill_empty_cells(step, &mut resolution);
        self.project_to_cells(&mut resolution);
        self.drop_removed(&resolution.removed);
        resolution.counts
    }

    /// Gives every occupied cell the particle that won it and marks the losers
    /// a trench takes.
    ///
    /// A surviving loser of the winner's own plate is lattice noise that
    /// spreads back out next step. One of another plate is a collision, and
    /// the column of them is what a later slice will read as crustal
    /// thickness, so that is what the counts record.
    fn pick_winners(&self, boundaries: &BoundaryClassification, resolution: &mut Resolution) {
        for cell in 0..self.mesh.cell_count() {
            let incumbent = resolution.previous_owners[cell];
            let occupants = resolution.occupancy.at(cell);
            let Some((&first, rest)) = occupants.split_first() else {
                continue;
            };
            let winner = rest.iter().fold(first, |best, &candidate| {
                match self.occupant_order(cell, incumbent, candidate, best) {
                    Ordering::Greater => candidate,
                    _ => best,
                }
            });

            let mut foreign = 0;
            for &loser in occupants {
                if loser == winner {
                    continue;
                }
                if self.subducts(cell, boundaries, &resolution.previous_owners, loser, winner) {
                    resolution.removed[loser] = true;
                    resolution.counts.subducted_particle_count += 1;
                } else if self.particles[loser].plate != self.particles[winner].plate {
                    foreign += 1;
                }
            }
            if foreign > 0 {
                resolution.counts.collided_cell_count += 1;
                resolution.counts.maximum_collision_stack =
                    resolution.counts.maximum_collision_stack.max(foreign + 1);
            }
            resolution.winners[cell] = Some(winner);
        }
    }

    /// Gives every cell no particle reached the nearest one within the gap
    /// radius, and makes ocean floor in the cells that have none.
    ///
    /// A particle made here is pushed at once, so its index is a real one. It
    /// is still invisible to every other cell, because the search reads the
    /// occupancy snapshot rather than the particle list: two adjacent gap
    /// cells both make floor rather than one sampling the other's.
    fn fill_empty_cells(&mut self, step: i32, resolution: &mut Resolution) {
        let mut ring = Vec::new();
        for cell in 0..self.mesh.cell_count() {
            if resolution.winners[cell].is_some() {
                continue;
            }
            let incumbent = resolution.previous_owners[cell];
            let sampled = self.sample_nearby(
                cell,
                incumbent,
                &resolution.removed,
                &resolution.occupancy,
                &mut ring,
            );
            let winner = match sampled {
                Some(particle) => {
                    resolution.counts.sampled_cell_count += 1;
                    particle
                }
                None => {
                    self.particles.push(Particle {
                        position: cell_direction(self.mesh, cell),
                        plate: incumbent,
                        birth: Some(step),
                        deformation: 0.0,
                        cell,
                        triangle: incident_triangle(self.mesh, cell),
                    });
                    resolution.removed.push(false);
                    resolution.counts.born_particle_count += 1;
                    self.particles.len() - 1
                }
            };
            resolution.winners[cell] = Some(winner);
        }
    }

    /// Writes the three columns a run returns from the particle each cell
    /// took: its owner, and the two fields it reads off the material.
    fn project_to_cells(&mut self, resolution: &mut Resolution) {
        for cell in 0..self.mesh.cell_count() {
            let winner = resolution.winners[cell].expect("every cell holds or samples a particle");
            let particle = self.particles[winner];
            resolution.counts.owner_change_count +=
                usize::from(particle.plate != resolution.previous_owners[cell]);
            self.partition.cell_plates[cell] = particle.plate;
            self.cell_birth[cell] = particle.birth;
            self.cell_deformation[cell] = particle.deformation;
        }
    }

    /// Drops the particles the trenches took. It happens last because every
    /// pass before it indexes the particle list as it stood.
    fn drop_removed(&mut self, removed: &[bool]) {
        self.particles = self
            .particles
            .drain(..)
            .enumerate()
            .filter(|(index, _)| !removed[*index])
            .map(|(_, particle)| particle)
            .collect();
    }

    /// Orders two particles in one cell, greatest first.
    ///
    /// Continental material covers ocean floor whichever plates the two
    /// belong to. Among continental material the cell's own plate keeps it,
    /// so a boundary cell does not flicker between the two plates that share
    /// it. Among ocean floor the younger subducts the older, which is the
    /// polarity of an island arc. What is left is two parcels of one plate,
    /// where the nearer to the cell's centre is the better answer for it, and
    /// the lower index breaks an exact tie.
    fn occupant_order(&self, cell: usize, incumbent: usize, left: usize, right: usize) -> Ordering {
        let class = |index: usize| {
            let particle = self.particles[index];
            (
                particle.is_continental(),
                particle.is_continental() && particle.plate == incumbent,
                particle.birth.unwrap_or(i32::MIN),
            )
        };
        let nearness = |index: usize| {
            self.particles[index]
                .position
                .dot(self.mesh.cell_centers[cell])
        };
        class(left)
            .cmp(&class(right))
            .then_with(|| nearness(left).total_cmp(&nearness(right)))
            .then(right.cmp(&left))
    }

    /// Whether the trench at this cell takes a losing particle.
    ///
    /// Ocean floor of another plate, in a cell that plate is converging on:
    /// that is the slab going under, and the material is gone. Everything
    /// else stacks. Two parcels of one plate are lattice noise that spreads
    /// back out next step, continental material is never destroyed, and a
    /// parcel that crossed a transform or a ridge is not being subducted.
    fn subducts(
        &self,
        cell: usize,
        boundaries: &BoundaryClassification,
        owners: &[usize],
        loser: usize,
        winner: usize,
    ) -> bool {
        let loser = self.particles[loser];
        if loser.is_continental() || loser.plate == self.particles[winner].plate {
            return false;
        }
        self.mesh.cell_corners(cell).iter().any(|corner| {
            boundaries.edge_classes[corner.edge] == BoundaryClass::Convergent
                && owners[corner.neighbor] == loser.plate
        })
    }

    /// The particle an empty cell samples: the nearest one within the gap
    /// radius, preferring the cell's own plate.
    ///
    /// This is a raster fill rather than a material event. The cell reads
    /// what the particle carries and the particle stays where it is, so
    /// nothing is created, moved, or counted twice. Only the one- and two-hop
    /// rings are searched, which is as far as the gap radius can reach.
    fn sample_nearby(
        &self,
        cell: usize,
        incumbent: usize,
        removed: &[bool],
        occupancy: &Occupancy,
        ring: &mut Vec<usize>,
    ) -> Option<usize> {
        let center = cell_direction(self.mesh, cell);
        // Both are unit directions, so `|a - b|^2 = 2 - 2 a.b` and the one
        // dot product decides the radius and the ordering together.
        let limit = self.config.transport.gap_radius * self.cell_width * self.mesh.radius.recip();
        let nearest = 1.0 - 0.5 * limit * limit;

        ring.clear();
        ring.extend(
            self.mesh
                .cell_corners(cell)
                .iter()
                .map(|corner| corner.neighbor),
        );
        for index in 0..ring.len() {
            for corner in self.mesh.cell_corners(ring[index]) {
                if corner.neighbor != cell && !ring.contains(&corner.neighbor) {
                    ring.push(corner.neighbor);
                }
            }
        }

        let mut best: Option<(bool, f32, usize)> = None;
        for &neighbor in ring.iter() {
            for &index in occupancy.at(neighbor) {
                if removed[index] {
                    continue;
                }
                let particle = self.particles[index];
                let nearness = particle.position.dot(center);
                if nearness < nearest {
                    continue;
                }
                let candidate = (particle.plate == incumbent, nearness, index);
                if best.is_none_or(|current| sample_precedes(candidate, current)) {
                    best = Some(candidate);
                }
            }
        }
        best.map(|(_, _, index)| index)
    }

    /// Hands the material of plate `from` to plate `to`, which is what makes
    /// a lifecycle event move crust rather than only ownership.
    ///
    /// The lifecycle relabels cells; this is the same relabelling on the
    /// material, and it lives here so that the ownership module never reaches
    /// into the particles.
    pub(crate) fn relabel_particles(&mut self, from: usize, to: usize, scope: MaterialScope) {
        for particle in &mut self.particles {
            if particle.plate != from {
                continue;
            }
            if scope == MaterialScope::OwnCells && self.partition.cell_plates[particle.cell] != to {
                continue;
            }
            particle.plate = to;
        }
    }

    /// Remaps every particle's plate through `compacted`, and drops the
    /// material of a plate the map has no id for.
    ///
    /// Only compaction can drop material this way, and only because it runs
    /// once after the last step: a plate that owns no cell holds particles
    /// stacked under other plates' cells, and nothing moves them again.
    pub(crate) fn remap_particle_plates(&mut self, compacted: &[usize]) {
        self.particles
            .retain(|particle| compacted[particle.plate] != usize::MAX);
        for particle in &mut self.particles {
            particle.plate = compacted[particle.plate];
        }
    }

    pub(crate) fn continental_particle_count(&self) -> usize {
        self.particles
            .iter()
            .filter(|particle| particle.is_continental())
            .count()
    }
}

/// Orders two candidates an empty cell could sample: its own plate first,
/// then the nearer, then the lower index.
fn sample_precedes(candidate: (bool, f32, usize), current: (bool, f32, usize)) -> bool {
    candidate
        .0
        .cmp(&current.0)
        .then_with(|| candidate.1.total_cmp(&current.1))
        .then(current.2.cmp(&candidate.2))
        == Ordering::Greater
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        NO_LIFECYCLE, NO_POLE_DRIFT, fingerprint, forced_rift_fixture, mesh as test_mesh,
        two_plate_fixture,
    };
    use crate::{
        CrustBirthPriorConfig, CrustClass, CrustClassification, CrustClassificationDiagnostics,
        PlateEvolutionConfig, PlateEvolutionInputs, PlateKinematics, PlatePartition,
        classify_boundaries, derive_crust_birth_prior, evolve_plate_ownership,
    };

    /// A run over the two-plate fixture that moves no material at all, so a
    /// test can place a particle by hand and see what one resolution does to
    /// it and nothing else.
    fn still_config() -> PlateEvolutionConfig {
        PlateEvolutionConfig {
            step_duration: 0.0,
            pole_drift: NO_POLE_DRIFT,
            lifecycle: NO_LIFECYCLE,
            ..PlateEvolutionConfig::default()
        }
    }

    /// One step over the two-plate fixture with an extra oceanic particle of
    /// the small plate placed in the large plate's cell across their shared
    /// edge, which is what arriving at a trench looks like, and that edge
    /// classified as `class`.
    fn one_arrival(class: BoundaryClass) -> (usize, TransportCounts) {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental, CrustClass::Oceanic]);
        let [overriding, arriving] = fixture.mesh.edges[0].cells;
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), still_config());
        world.particles.push(Particle {
            position: cell_direction(&fixture.mesh, overriding),
            plate: fixture.partition.cell_plates[arriving],
            birth: Some(0),
            deformation: 0.0,
            cell: overriding,
            triangle: incident_triangle(&fixture.mesh, overriding),
        });

        let mut boundaries = fixture.boundaries.clone();
        boundaries.edge_classes[0] = class;
        let counts = world.transport(&boundaries, 1);
        (overriding, counts)
    }

    #[test]
    fn a_trench_takes_the_floor_that_arrives_under_it() {
        let (overriding, counts) = one_arrival(BoundaryClass::Convergent);

        assert_eq!(counts.subducted_particle_count, 1);
        assert_eq!(
            counts.collided_cell_count, 0,
            "cell {overriding} kept only the material that won it"
        );
    }

    #[test]
    fn material_that_crossed_a_transform_is_not_being_subducted() {
        for class in [BoundaryClass::Transform, BoundaryClass::Divergent] {
            let (overriding, counts) = one_arrival(class);

            assert_eq!(counts.subducted_particle_count, 0, "{class:?}");
            assert_eq!(
                counts.collided_cell_count, 1,
                "cell {overriding} must stack the {class:?} arrival rather than lose it"
            );
            assert_eq!(counts.maximum_collision_stack, 2, "{class:?}");
        }
    }

    /// Empties `cell` and leaves the small plate's only particle just outside
    /// it, nearer to its centre than anything the cell's own plate has, so
    /// that what the cell samples is decided by whose material it is.
    #[test]
    fn an_empty_cell_samples_its_own_plates_material_over_nearer_material() {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental, CrustClass::Oceanic]);
        let [emptied, neighbor] = fixture.mesh.edges[0].cells;
        let incumbent = fixture.partition.cell_plates[emptied];
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), still_config());
        world.particles.remove(emptied);
        let nearest = world
            .particles
            .iter_mut()
            .find(|particle| particle.cell == neighbor)
            .expect("the small plate owns one cell");
        // Just the neighbour's side of the boundary between the two: inside
        // the neighbour's cell, and nearer to the emptied cell's centre than
        // any of that cell's own neighbours are.
        nearest.position = (cell_direction(&fixture.mesh, emptied) * 0.45
            + cell_direction(&fixture.mesh, neighbor) * 0.55)
            .normalized();

        let counts = world.transport(&fixture.boundaries, 1);

        assert_eq!(counts.sampled_cell_count, 1);
        assert_eq!(counts.born_particle_count, 0);
        assert_eq!(
            world.partition.cell_plates[emptied], incumbent,
            "the emptied cell took the nearer plate's material instead of its own"
        );
    }

    #[test]
    fn a_cell_with_nothing_within_the_gap_radius_makes_floor_for_its_own_plate() {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental, CrustClass::Oceanic]);
        let emptied = fixture.mesh.edges[0].cells[0];
        let incumbent = fixture.partition.cell_plates[emptied];
        let config = PlateEvolutionConfig {
            // Nothing is ever within no distance at all, so every empty cell
            // is a gap and the rule under test is the only one that can fire.
            transport: MaterialTransportConfig { gap_radius: 0.0 },
            ..still_config()
        };
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), config);
        world.particles.remove(emptied);

        let counts = world.transport(&fixture.boundaries, 7);

        assert_eq!(counts.born_particle_count, 1);
        assert_eq!(counts.sampled_cell_count, 0);
        assert_eq!(world.partition.cell_plates[emptied], incumbent);
        assert_eq!(world.cell_birth[emptied], Some(7));
        assert_eq!(world.cell_deformation[emptied], 0.0);
    }

    #[test]
    fn a_suture_takes_the_whole_of_the_plate_it_absorbs() {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental, CrustClass::Oceanic]);
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), still_config());
        let before = world
            .particles
            .iter()
            .filter(|particle| particle.plate == 1)
            .count();
        assert!(before > 0);

        world.relabel_particles(1, 0, MaterialScope::WholePlate);

        assert!(
            world.particles.iter().all(|particle| particle.plate == 0),
            "an absorbed plate keeps no material of its own"
        );
    }

    #[test]
    fn a_rift_hands_the_new_halfs_material_to_the_new_plate() {
        let (fixture, config) = forced_rift_fixture();
        // Before any transport, so every cell holds its own material and what
        // the rift did to the material is the only thing on show.
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), config);
        assert_eq!(world.lifecycle(&fixture.boundaries, 0).rift_count, 1);

        let new_plate = world.partition.plate_count - 1;
        let moved = world
            .particles
            .iter()
            .filter(|particle| world.partition.cell_plates[particle.cell] == new_plate)
            .inspect(|particle| {
                assert_eq!(
                    particle.plate, new_plate,
                    "the new half left its own material behind"
                );
            })
            .count();
        assert!(moved > 0, "the rift gave the new plate no cells");
    }

    /// One plate over the whole mesh with a continental cap on it, turning
    /// about a fixed axis: the case where conservation has to be exact,
    /// because there is no boundary anywhere for material to be made or
    /// destroyed at.
    #[test]
    fn a_pure_rotation_makes_and_destroys_nothing() {
        let mesh = test_mesh(65_536);
        let partition = PlatePartition {
            cell_plates: vec![0; mesh.cell_count()],
            plate_count: 1,
        };
        let crust = CrustClassification {
            cell_classes: mesh
                .cell_centers
                .iter()
                .map(|center| match center.z > 0.5 {
                    true => CrustClass::Continental,
                    false => CrustClass::Oceanic,
                })
                .collect(),
            diagnostics: CrustClassificationDiagnostics::default(),
        };
        let kinematics = PlateKinematics {
            angular_velocities: vec![Vec3::Z],
        };
        let boundaries = classify_boundaries(&mesh, &partition, &kinematics).unwrap();
        let birth_prior = derive_crust_birth_prior(
            &mesh,
            &partition,
            &crust,
            &boundaries,
            CrustBirthPriorConfig::default(),
        )
        .unwrap();
        let evolution = evolve_plate_ownership(
            &mesh,
            PlateEvolutionInputs {
                partition: &partition,
                kinematics: &kinematics,
                boundaries: &boundaries,
                birth_prior: &birth_prior,
            },
            PlateEvolutionConfig {
                step_count: 40,
                // One cell width per step at the unit speed above, so the cap
                // crosses forty cells over the run.
                step_duration: crate::field::mean_cell_width(&mesh),
                pole_drift: NO_POLE_DRIFT,
                lifecycle: NO_LIFECYCLE,
                ..PlateEvolutionConfig::default()
            },
        )
        .unwrap();

        assert_eq!(evolution.diagnostics.born_particle_count, 0);
        assert_eq!(evolution.diagnostics.subducted_particle_count, 0);
        assert_eq!(
            evolution.diagnostics.final_continental_particle_count,
            evolution.diagnostics.starting_continental_particle_count
        );
        // The cap is a raster of that material and the count wanders with the
        // cell areas the cap happens to cover, so the mask is pinned and the
        // count is not.
        assert_eq!(
            fingerprint(
                evolution
                    .cell_birth
                    .iter()
                    .map(|birth| u64::from(birth.is_none()))
            ),
            12_365_028_840_329_121_326
        );
    }
}
