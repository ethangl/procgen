//! Breaking one continental plate into two.
//!
//! It is the half of [`crate::lifecycle`] that creates a plate, and the only
//! place a run makes a new plate identity. Suturing, which destroys one, and
//! the compaction that closes the holes both leave stay there.
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
//! wall lies closest to and `m` is the wall's mean center. Both inherit the
//! parent's base speed and drift factor, so the opening decides which way a
//! half goes and the step's respeed decides how fast. That axis is the
//! one whose velocity at the rift is normal to the rift, so the relative
//! motion across the new boundary is pure opening and it classifies divergent.
//! An arc that closed on itself is the one case with no such axis: its wall's
//! mean center is the sphere's own. That rift fails too.
//!
//!
//! Determinism: the arc walk, connected components, integer hashes, and area
//! sums are all exact, and the only floats that decide an integer here are
//! area comparisons and the plane fit, which use add, multiply, and square
//! root alone. Nothing on the path calls libm.

use crate::{
    CrustClass,
    cracks::{Cracks, adopt_unassigned_cells},
    lifecycle::LifecycleEvents,
    step::EvolvingWorld,
    transport::MaterialScope,
};
use procgen_core::{RandomStream, Vec3, random_streams::PLATE_RIFT};
use procgen_sphere_mesh::{SphereMesh, connected_components};

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

impl EvolvingWorld<'_> {
    pub(crate) fn rift(&mut self, step: i32) -> LifecycleEvents {
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

        // The half's material leaves with it, so the two halves part carrying
        // their own crust and the arc between them opens a real gap.
        self.relabel_particles(plate, new_plate, MaterialScope::OwnCells);

        let parent = self.kinematics.angular_velocities[plate];
        let rotations = [parent + opening, parent - opening];
        self.kinematics.angular_velocities[plate] = rotations[keeps];
        self.kinematics
            .angular_velocities
            .push(rotations[1 - keeps]);
        // A half is the parent's own crust on the parent's own base speed,
        // carrying the drift the parent had walked to. The opening sets its
        // direction and the respeed that ends this step sets its length, so
        // what the two halves keep of the opening is the way they part.
        let base = self.kinematics.base_speeds[plate];
        self.kinematics.base_speeds.push(base);
        self.drift_factors.push(self.drift_factors[plate]);
        true
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
    use crate::test_support::{
        EvolutionFixture, closed_arc_rift_fixture, empty_boundaries, failed_rift_fixture,
        forced_rift_fixture, half_oceanic_rift_fixture,
    };
    use crate::{BoundaryClass, CrustClass, PlateEvolution, PlateEvolutionConfig};

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
        assert!(run.diagnostics.born_particle_count > 0);
        let born: Vec<usize> = (0..fixture.mesh.cell_count())
            .filter(|&cell| run.cell_birth[cell].is_some_and(|birth| birth >= 0.0))
            .collect();
        assert!(!born.is_empty(), "the rift made no oceanic crust");
        for &cell in &born {
            assert!(
                run.cell_birth[cell].is_some_and(|birth| birth < run.elapsed_time),
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
    fn a_rift_gives_both_halves_the_parents_base_speed_and_drift() {
        let (fixture, config) = forced_rift_fixture();
        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), config);
        world.kinematics.base_speeds = vec![0.6, 0.3];
        world.drift_factors = vec![1.1, 0.7];

        let events = world.lifecycle(&empty_boundaries(&fixture.mesh), 0);

        assert_eq!(events.rift_count, 1);
        assert_eq!(world.partition.plate_count, 3);
        // The parent keeps its id and the new half takes the next one.
        assert_eq!(world.kinematics.base_speeds, vec![0.6, 0.3, 0.6]);
        assert_eq!(world.drift_factors, vec![1.1, 0.7, 1.1]);
    }

    #[test]
    fn a_run_that_rifts_is_deterministic() {
        let (fixture, config) = forced_rift_fixture();
        let first = fixture.evolve(config);

        assert_eq!(first, fixture.evolve(config));
        assert!(first.diagnostics.rift_count > 0);
    }
}
