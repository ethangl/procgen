//! The crack pattern that gives the plate partition its primary boundaries.
//!
//! Arcs walk the mesh — great circles, optionally bent by noise — and every
//! cell they pass through becomes a wall owned by that arc. The connected
//! components of the remaining cells are the faces; wall cells then join the
//! face they touch most, and faces that are too small or too thin merge into
//! a neighbor.
//!
//! Face ids decide integers every later stage reports, so the walk is
//! libm-free: add, multiply, divide, and square root on `f32`, the polynomial
//! gradient noise, and integer hashes are all IEEE-exact, and point location
//! compares the same floats on the same inputs, so the wall cells come out
//! bit-identical on macOS and Windows. Merging then compares mesh cell areas,
//! which every stage that weights by area already depends on.

use procgen_core::{RandomStream, Vec3, random_streams::PLATE_CRACK_ARC};
use procgen_noise::{fold_seed_u64_to_u32, gradient_noise_3d};
use procgen_sphere_mesh::{SphereMesh, connected_components};
use std::{cmp::Reverse, f64::consts::PI, ops::ControlFlow};

/// Walk step as a fraction of the mean cell spacing. Well below one so a step
/// cannot pass a cell, which keeps the wall connected and one cell wide.
const STEP_SPACING: f32 = 0.3;
/// Lattice frequency of the field that bends a crack's heading.
const NOISE_FREQUENCY: f32 = 2.5;
/// Each direction of an arc stops after half a circle, so an arc that runs
/// into nothing closes on itself instead of overlapping its own start.
const DIRECTION_ARC: f32 = std::f32::consts::PI;
/// Faces below this fraction of the sphere merge into a neighbor.
const MIN_FACE_AREA_FRACTION: f64 = 0.004;
/// Faces whose perimeter exceeds this multiple of an equal-area circle's
/// circumference merge into a neighbor.
const MAX_FACE_PERIMETER_RATIO: f64 = 2.6;

/// A per-cell face assignment covering every mesh cell.
pub(crate) struct CrackFaces {
    pub(crate) cell_faces: Vec<usize>,
    pub(crate) face_count: usize,
}

/// Cracks the mesh into faces: walls, connected components, wall adoption,
/// and merging. `curvature` is the heading change in radians per radian
/// travelled; zero walks great circles.
pub(crate) fn crack_faces(
    mesh: &SphereMesh,
    seed: u64,
    arc_count: usize,
    curvature: f32,
) -> CrackFaces {
    let mut cracks = Cracks::new(mesh, seed, curvature);
    for arc in 0..arc_count {
        cracks.walk_arc(arc);
    }

    let components =
        connected_components(mesh, |cell| cracks.cell_arcs[cell].is_none(), |_, _| true);
    if components.is_empty() {
        // Every cell is a wall, so the walls themselves are the only face.
        return CrackFaces {
            cell_faces: vec![0; mesh.cell_count()],
            face_count: 1,
        };
    }

    let mut cell_faces = vec![None; mesh.cell_count()];
    for (face, component) in components.iter().enumerate() {
        for &cell in component {
            cell_faces[cell] = Some(face);
        }
    }
    adopt_wall_cells(mesh, &mut cell_faces);
    let cell_faces: Vec<usize> = cell_faces
        .into_iter()
        .map(|face| face.expect("every wall cell reaches a face"))
        .collect();

    merge_faces(mesh, cell_faces, components.len())
}

/// Rotates `vector` toward `perpendicular` by the angle whose tangent
/// half-angle is `half_tangent`. Writing every rotation this way keeps the
/// walk to add, multiply, and divide, which is what makes it exact on both
/// development machines.
fn rotate(vector: Vec3, perpendicular: Vec3, half_tangent: f32) -> Vec3 {
    let square = half_tangent * half_tangent;
    let scale = (1.0 + square).recip();
    vector * ((1.0 - square) * scale) + perpendicular * (2.0 * half_tangent * scale)
}

struct Cracks<'mesh> {
    mesh: &'mesh SphereMesh,
    /// Owning arc per wall cell.
    cell_arcs: Vec<Option<usize>>,
    triangle_hint: usize,
    step: f32,
    curvature: f32,
    seed: u64,
    arcs: RandomStream,
}

impl<'mesh> Cracks<'mesh> {
    fn new(mesh: &'mesh SphereMesh, seed: u64, curvature: f32) -> Self {
        let spacing = (4.0 * std::f32::consts::PI / mesh.cell_count() as f32).sqrt();
        Self {
            mesh,
            cell_arcs: vec![None; mesh.cell_count()],
            triangle_hint: 0,
            step: STEP_SPACING * spacing,
            curvature,
            seed,
            arcs: RandomStream::new(seed, PLATE_CRACK_ARC),
        }
    }

    /// Walks one arc outward from a hashed start cell in both directions. An
    /// arc that starts on an existing wall is skipped.
    fn walk_arc(&mut self, arc: usize) {
        let item = arc as u64;
        let start = (self.arcs.sample_u64(item, 0) % self.mesh.cell_count() as u64) as usize;
        if self.cell_arcs[start].is_some() {
            return;
        }
        self.cell_arcs[start] = Some(arc);

        let center = self.mesh.cell_centers[start].normalized();
        let hashed = Vec3::new(
            self.arcs.signed_f32(item, 1),
            self.arcs.signed_f32(item, 2),
            self.arcs.signed_f32(item, 3),
        );
        // The hashed half-tangent lands in [-1, 1), a rotation of plus or
        // minus ninety degrees. Together with the spread of the hashed
        // direction and the two opposite walk directions, that covers every
        // orientation a crack line can take.
        let tangent = center.cross(hashed).normalized();
        let tangent = rotate(
            tangent,
            center.cross(tangent),
            self.arcs.signed_f32(item, 4),
        )
        .normalized();

        self.walk(arc, start, center, tangent);
        self.walk(arc, start, center, -tangent);
    }

    fn walk(&mut self, arc: usize, start: usize, mut position: Vec3, mut tangent: Vec3) {
        let noise_key = fold_seed_u64_to_u32(self.seed.wrapping_add(arc as u64));
        let half_step = 0.5 * self.step;
        let mut previous = start;
        let mut travelled = 0.0;

        while travelled < DIRECTION_ARC {
            let advanced = rotate(position, tangent, half_step);
            tangent = rotate(tangent, -position, half_step);
            position = advanced.normalized();
            tangent = (tangent - position * position.dot(tangent)).normalized();
            if self.curvature > 0.0 {
                let noise = gradient_noise_3d(noise_key, position * NOISE_FREQUENCY).value;
                let bend = self.curvature * noise * self.step;
                tangent = rotate(tangent, position.cross(tangent), 0.5 * bend).normalized();
            }
            travelled += self.step;

            let cell = self.locate(position);
            if cell == previous {
                continue;
            }
            if self.claim_step(arc, previous, cell).is_break() {
                return;
            }
            previous = cell;
        }
    }

    /// Claims every cell one step entered, bridging the cell a step skipped
    /// over so the wall stays connected. Breaks when the step cannot be
    /// bridged or ran into another arc, either of which ends this direction.
    fn claim_step(&mut self, arc: usize, previous: usize, cell: usize) -> ControlFlow<()> {
        if !self.mesh.are_adjacent(previous, cell) {
            match self
                .mesh
                .cell_corners(previous)
                .iter()
                .map(|corner| corner.neighbor)
                .find(|&between| self.mesh.are_adjacent(between, cell))
            {
                Some(between) => self.claim(arc, between)?,
                None => return ControlFlow::Break(()),
            }
        }
        self.claim(arc, cell)
    }

    fn claim(&mut self, arc: usize, cell: usize) -> ControlFlow<()> {
        match self.cell_arcs[cell] {
            Some(owner) if owner != arc => ControlFlow::Break(()),
            _ => {
                self.cell_arcs[cell] = Some(arc);
                ControlFlow::Continue(())
            }
        }
    }

    /// Returns the Voronoi cell holding `position`: among the located
    /// triangle's three cells, the nearest center.
    fn locate(&mut self, position: Vec3) -> usize {
        let location = self.mesh.locate_delaunay(position, self.triangle_hint);
        self.triangle_hint = location.triangle;
        location
            .cells
            .into_iter()
            .max_by(|&left, &right| {
                position
                    .dot(self.mesh.cell_centers[left])
                    .total_cmp(&position.dot(self.mesh.cell_centers[right]))
            })
            .expect("a Delaunay triangle has three corners")
    }
}

/// Wall cells join the neighboring face they share the most edges with,
/// repeating until the whole mesh is covered. Ties go to the lower face id.
fn adopt_wall_cells(mesh: &SphereMesh, cell_faces: &mut Vec<Option<usize>>) {
    loop {
        let mut next = cell_faces.clone();
        let mut changed = false;
        for cell in 0..mesh.cell_count() {
            if cell_faces[cell].is_some() {
                continue;
            }
            let mut tally: Vec<(usize, usize)> = Vec::new();
            for corner in mesh.cell_corners(cell) {
                let Some(face) = cell_faces[corner.neighbor] else {
                    continue;
                };
                match tally.iter_mut().find(|entry| entry.0 == face) {
                    Some(entry) => entry.1 += 1,
                    None => tally.push((face, 1)),
                }
            }
            if let Some(&(face, _)) = tally
                .iter()
                .max_by_key(|&&(face, shared)| (shared, Reverse(face)))
            {
                next[cell] = Some(face);
                changed = true;
            }
        }
        *cell_faces = next;
        if !changed {
            return;
        }
    }
}

/// How much of a merging face's boundary one neighbor holds.
#[derive(Clone, Copy, Default)]
struct SharedBoundary {
    edges: usize,
    length: f64,
}

/// Merges faces that are too small or too thin into the neighbor they share
/// the most edges with, smallest first, then compacts the surviving ids.
fn merge_faces(mesh: &SphereMesh, mut cell_faces: Vec<usize>, face_count: usize) -> CrackFaces {
    // The thinness ratio is a shape heuristic, so an edge's length is the
    // straight chord between its two Voronoi vertices rather than the arc.
    let edge_lengths: Vec<f64> = mesh
        .edges
        .iter()
        .map(|edge| {
            f64::from(
                mesh.vertices[edge.vertices[0]].distance_squared(mesh.vertices[edge.vertices[1]]),
            )
            .sqrt()
        })
        .collect();
    let minimum_area = MIN_FACE_AREA_FRACTION * mesh.total_area();

    let mut areas = vec![0.0; face_count];
    for (cell, &face) in cell_faces.iter().enumerate() {
        areas[face] += f64::from(mesh.cell_areas[cell]);
    }
    let mut perimeters = vec![0.0; face_count];
    for (edge, length) in mesh.edges.iter().zip(&edge_lengths) {
        let (left, right) = (cell_faces[edge.cells[0]], cell_faces[edge.cells[1]]);
        if left != right {
            perimeters[left] += length;
            perimeters[right] += length;
        }
    }

    loop {
        let offending = |face: usize| {
            areas[face] > 0.0
                && (areas[face] < minimum_area
                    || perimeters[face]
                        > MAX_FACE_PERIMETER_RATIO * 2.0 * (PI * areas[face]).sqrt())
        };
        let Some(small) = (0..face_count)
            .filter(|&face| offending(face))
            .min_by(|&left, &right| areas[left].total_cmp(&areas[right]).then(left.cmp(&right)))
        else {
            break;
        };

        let mut shared = vec![SharedBoundary::default(); face_count];
        for (edge, length) in mesh.edges.iter().zip(&edge_lengths) {
            let (left, right) = (cell_faces[edge.cells[0]], cell_faces[edge.cells[1]]);
            let neighbor = match (left == small, right == small) {
                (true, false) => right,
                (false, true) => left,
                _ => continue,
            };
            shared[neighbor].edges += 1;
            shared[neighbor].length += length;
        }
        // A face with no neighbor is the only face left, and it stays.
        let Some(target) = (0..face_count)
            .filter(|&face| shared[face].edges > 0)
            .max_by_key(|&face| (shared[face].edges, Reverse(face)))
        else {
            break;
        };

        // The two faces become one: areas add, and the boundary they shared
        // stops bounding either of them. Every other face keeps its own
        // perimeter, since its share of the boundary only changes owner.
        areas[target] += areas[small];
        areas[small] = 0.0;
        perimeters[target] += perimeters[small] - 2.0 * shared[target].length;
        perimeters[small] = 0.0;
        for face in cell_faces.iter_mut() {
            if *face == small {
                *face = target;
            }
        }
    }

    let mut compacted = vec![None; face_count];
    let mut next = 0;
    for face in cell_faces.iter_mut() {
        *face = match compacted[*face] {
            Some(id) => id,
            None => {
                compacted[*face] = Some(next);
                next += 1;
                next - 1
            }
        };
    }
    CrackFaces {
        cell_faces,
        face_count: next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mesh;

    #[test]
    fn every_cell_belongs_to_one_connected_face() {
        let cell_count = 1_024;
        let mesh = mesh(cell_count);
        let faces = crack_faces(&mesh, 7, 24, 2.0);

        assert_eq!(faces.cell_faces.len(), cell_count);
        assert!(faces.face_count > 1);
        for face in 0..faces.face_count {
            assert_eq!(
                connected_components(&mesh, |cell| faces.cell_faces[cell] == face, |_, _| true)
                    .len(),
                1,
                "face {face} is empty or disconnected"
            );
        }
    }

    #[test]
    fn merged_faces_clear_the_small_area_threshold() {
        let mesh = mesh(1_024);
        let faces = crack_faces(&mesh, 7, 24, 2.0);
        let mut areas = vec![0.0; faces.face_count];
        for (cell, &face) in faces.cell_faces.iter().enumerate() {
            areas[face] += f64::from(mesh.cell_areas[cell]);
        }

        assert!(faces.face_count > 1);
        for (face, area) in areas.into_iter().enumerate() {
            assert!(
                area >= MIN_FACE_AREA_FRACTION * mesh.total_area(),
                "face {face} survived below the threshold"
            );
        }
    }

    #[test]
    fn straight_arcs_walk_a_great_circle() {
        let cell_count = 4_096;
        let mesh = mesh(cell_count);
        let mut cracks = Cracks::new(&mesh, 7, 0.0);
        cracks.walk_arc(0);
        let wall: Vec<Vec3> = (0..mesh.cell_count())
            .filter(|&cell| cracks.cell_arcs[cell] == Some(0))
            .map(|cell| mesh.cell_centers[cell].normalized())
            .collect();

        let spacing = (4.0 * std::f32::consts::PI / cell_count as f32).sqrt();
        assert!(
            wall.len() as f32 > 0.5 * std::f32::consts::TAU / spacing,
            "an arc that meets nothing closes on itself"
        );

        // The plane through the start and the wall cell nearest a quarter turn
        // away. Two cell spacings of slack: one for how far a wall cell's
        // center sits off the arc, one for the same error in those two.
        let normal = wall[1..]
            .iter()
            .map(|&center| wall[0].cross(center))
            .max_by(|left, right| left.length_squared().total_cmp(&right.length_squared()))
            .expect("the arc has more than one wall cell")
            .normalized();
        for center in wall {
            assert!(center.dot(normal).abs() <= 2.0 * spacing);
        }
    }
}
