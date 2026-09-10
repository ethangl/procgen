use crate::{
    TopologyError,
    initial::{initial_tetrahedron, validate_points},
};
use procgen_core::Vec3;
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Debug)]
pub struct SphericalDelaunay {
    /// Cell centers, each the f32 rounding of the direction beside it.
    points: Vec<Vec3>,
    /// Unit-length cell directions in f64, the positions every geometric
    /// decision here is made on. See `direction`.
    directions: Vec<[f64; 3]>,
    /// Outward-facing triangles, counter-clockwise when viewed from outside.
    triangles: Vec<[usize; 3]>,
    /// Opposite half-edge for each flattened triangle edge.
    opposite_half_edges: Vec<usize>,
}

impl SphericalDelaunay {
    pub fn build(points: Vec<Vec3>) -> Result<Self, TopologyError> {
        validate_points(&points)?;
        QuickHull::build(&points)
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    /// The cell centers, which are the input points normalized: the
    /// triangulation is the one of the input *directions*, and a center off
    /// the sphere would not be a point of the cell it names. Input within
    /// `UNIT_SPHERE_TOLERANCE` of unit length moves by at most that much.
    pub fn points(&self) -> &[Vec3] {
        &self.points
    }

    pub fn triangles(&self) -> &[[usize; 3]] {
        &self.triangles
    }

    pub fn half_edge_count(&self) -> usize {
        self.opposite_half_edges.len()
    }

    pub fn opposite(&self, edge: usize) -> usize {
        self.opposite_half_edges[edge]
    }

    pub fn edge_origin(&self, edge: usize) -> usize {
        self.triangles[half_edge_face(edge)][half_edge_corner(edge)]
    }

    pub fn edge_destination(&self, edge: usize) -> usize {
        let next = next_half_edge(edge);
        self.triangles[half_edge_face(next)][half_edge_corner(next)]
    }

    pub fn unique_edges(&self) -> impl Iterator<Item = usize> + '_ {
        self.opposite_half_edges
            .iter()
            .enumerate()
            .filter_map(|(edge, &opposite)| (edge < opposite).then_some(edge))
    }

    /// The direction equidistant from the triangle's three cells, which is
    /// the Voronoi vertex the triangle is dual to.
    ///
    /// In f64, because a triangle spanning two cells only a few
    /// ten-thousandths apart has one very short edge, and its plane normal
    /// swings by a few percent of a cell width for every f32 step its
    /// corners move. That is far enough to push such a cell's Voronoi vertex
    /// past the ring's own edge and invert a corner of it.
    pub fn triangle_circumcenter(&self, triangle: usize) -> Vec3 {
        let [a, b, c] = self.triangles[triangle].map(|point| self.directions[point]);
        let normal = cross(offset(b, a), offset(c, a));
        let scale = length_squared(normal).sqrt().recip();
        Vec3::new(
            (normal[0] * scale) as f32,
            (normal[1] * scale) as f32,
            (normal[2] * scale) as f32,
        )
    }

    pub const fn edge_triangle(&self, edge: usize) -> usize {
        half_edge_face(edge)
    }

    pub fn triangle_neighbors(&self, triangle: usize) -> [usize; 3] {
        std::array::from_fn(|corner| self.edge_triangle(self.opposite(flat_edge(triangle, corner))))
    }

    pub(crate) fn edges_around_point(&self, start: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(Some(start), move |&incoming| {
            let next = self.opposite_half_edges[next_half_edge(incoming)];
            (next != start).then_some(next)
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct HorizonEdge {
    from: usize,
    to: usize,
    outside: usize,
}

#[derive(Debug)]
struct Face {
    vertices: [usize; 3],
    neighbors: [usize; 3],
    /// Outward plane normal, unnormalized. Held per face, with the first
    /// vertex for the plane's origin, so that a conflict query costs one
    /// difference and one dot.
    normal: [f64; 3],
    alive: bool,
    conflicts: Vec<usize>,
    // Selecting the farthest conflict as apex materially reduces hull expansion work.
    farthest_volume: f64,
    farthest_point: usize,
    visit_stamp: u64,
}

struct QuickHull {
    directions: Vec<[f64; 3]>,
    faces: Vec<Face>,
    interior_point: [f64; 3],
    visit_stamp: u64,
}

impl QuickHull {
    fn build(points: &[Vec3]) -> Result<SphericalDelaunay, TopologyError> {
        let [i0, i1, i2, i3] = initial_tetrahedron(points)?;
        let directions: Vec<[f64; 3]> = points.iter().copied().map(direction).collect();
        let interior_point = std::array::from_fn(|axis| {
            [i0, i1, i2, i3]
                .iter()
                .map(|&corner| directions[corner][axis])
                .sum::<f64>()
                * 0.25
        });
        let expected_faces = directions.len() * 2 - 4;
        let mut hull = Self {
            directions,
            faces: Vec::with_capacity(expected_faces),
            interior_point,
            visit_stamp: 0,
        };

        let seed_faces = [[i0, i2, i1], [i0, i1, i3], [i0, i3, i2], [i1, i2, i3]];
        let seed_neighbors = tetrahedron_neighbors(&seed_faces);
        for (vertices, neighbors) in seed_faces.into_iter().zip(seed_neighbors) {
            hull.add_face(vertices, neighbors)?;
        }

        for point_index in 0..hull.directions.len() {
            if ![i0, i1, i2, i3].contains(&point_index) {
                hull.assign_to_best_face(point_index, 0..hull.faces.len());
            }
        }

        let mut pending = VecDeque::new();
        hull.push_conflicting_faces(&mut pending, 0..hull.faces.len());

        while let Some(face_index) = pending.pop_front() {
            let face = &hull.faces[face_index];
            if !face.alive {
                continue;
            }

            let apex = face.farthest_point;
            let (new_faces, orphaned) = hull.expand(face_index, apex)?;

            for point_index in orphaned {
                hull.assign_to_best_face(point_index, new_faces.clone());
            }
            hull.push_conflicting_faces(&mut pending, new_faces);
        }

        Ok(hull.compact())
    }

    fn expand(
        &mut self,
        initial_face: usize,
        apex: usize,
    ) -> Result<(std::ops::Range<usize>, Vec<usize>), TopologyError> {
        let (visible, horizon) =
            self.visible_faces_and_horizon(initial_face, self.directions[apex])?;
        let orphaned = visible
            .iter()
            .flat_map(|&face| self.faces[face].conflicts.iter().copied())
            .filter(|&point| point != apex)
            .collect();

        for face in visible {
            self.remove_face(face);
        }

        let new_start = self.faces.len();
        let new_face_count = horizon.len();
        for (index, edge) in horizon.into_iter().enumerate() {
            let face = new_start + index;
            let next = new_start + (index + 1) % new_face_count;
            let previous = new_start + (index + new_face_count - 1) % new_face_count;
            self.add_face(
                [edge.from, edge.to, apex],
                [edge.outside, flat_edge(next, 2), flat_edge(previous, 1)],
            )?;
            self.faces[half_edge_face(edge.outside)].neighbors[half_edge_corner(edge.outside)] =
                flat_edge(face, 0);
        }

        Ok((new_start..self.faces.len(), orphaned))
    }

    fn add_face(
        &mut self,
        vertices: [usize; 3],
        neighbors: [usize; 3],
    ) -> Result<(), TopologyError> {
        let [a, b, c] = vertices.map(|vertex| self.directions[vertex]);
        let normal = cross(offset(b, a), offset(c, a));
        if length_squared(normal) < 1.0e-16 {
            return Err(TopologyError::DegeneratePoints);
        }

        debug_assert!(
            dot(normal, offset(self.interior_point, a)) < 0.0,
            "QuickHull produced an inward-facing triangle"
        );

        self.faces.push(Face {
            vertices,
            neighbors,
            normal,
            alive: true,
            conflicts: Vec::new(),
            farthest_volume: f64::NEG_INFINITY,
            farthest_point: usize::MAX,
            visit_stamp: 0,
        });
        Ok(())
    }

    /// Twice the signed volume of the tetrahedron on the face and the point,
    /// positive when the point lies outside the face's plane. It is the
    /// point's distance from that plane scaled by twice the face's area, so
    /// values compare as distances do among the conflicts of one face.
    ///
    /// Written as a normal dotted with a *difference* of positions, in f64.
    /// The plain plane distance `normal.dot(point) - plane_distance`
    /// subtracts two quantities of magnitude one to resolve a difference of
    /// 1e-9, so in f32 it carries absolute error near 1e-7 — larger than the
    /// true distance for a few hundred of the near-cocircular point
    /// quadruples a 65,536-point Fibonacci lattice contains. Each such
    /// misread leaves the hull locally concave, which is to say
    /// non-Delaunay, and inverts a lobe of the two Voronoi rings that share
    /// the edge.
    fn outward_volume(&self, face: usize, point: [f64; 3]) -> f64 {
        let face = &self.faces[face];
        dot(
            face.normal,
            offset(point, self.directions[face.vertices[0]]),
        )
    }

    fn push_conflicting_faces(
        &self,
        pending: &mut VecDeque<usize>,
        faces: impl Iterator<Item = usize>,
    ) {
        // A face is queued once, after its conflict set is complete.
        for face_index in faces {
            let face = &self.faces[face_index];
            if !face.conflicts.is_empty() {
                pending.push_back(face_index);
            }
        }
    }

    fn remove_face(&mut self, face_index: usize) {
        let face = &mut self.faces[face_index];
        face.alive = false;
        face.conflicts.clear();
    }

    fn assign_to_best_face(
        &mut self,
        point_index: usize,
        face_indices: impl Iterator<Item = usize>,
    ) {
        let point = self.directions[point_index];
        let mut best = None;
        for face_index in face_indices {
            let volume = self.outward_volume(face_index, point);
            if volume > 0.0 && best.is_none_or(|(_, best_volume)| volume > best_volume) {
                best = Some((face_index, volume));
            }
        }

        if let Some((face_index, volume)) = best {
            let face = &mut self.faces[face_index];
            face.conflicts.push(point_index);
            if volume > face.farthest_volume {
                face.farthest_volume = volume;
                face.farthest_point = point_index;
            }
        }
    }

    fn visible_faces_and_horizon(
        &mut self,
        initial: usize,
        apex: [f64; 3],
    ) -> Result<(Vec<usize>, Vec<HorizonEdge>), TopologyError> {
        self.visit_stamp += 1;
        let visit_stamp = self.visit_stamp;
        let mut visible = Vec::new();
        let mut horizon = Vec::new();
        let mut queue = VecDeque::from([initial]);
        self.faces[initial].visit_stamp = visit_stamp;

        while let Some(face_index) = queue.pop_front() {
            visible.push(face_index);
            let vertices = self.faces[face_index].vertices;
            let neighbors = self.faces[face_index].neighbors;
            for edge in 0..3 {
                let neighbor = neighbors[edge];
                let neighbor_face = half_edge_face(neighbor);
                debug_assert!(self.faces[neighbor_face].alive);
                if self.faces[neighbor_face].visit_stamp == visit_stamp {
                    continue;
                }
                if self.outward_volume(neighbor_face, apex) > 0.0 {
                    self.faces[neighbor_face].visit_stamp = visit_stamp;
                    queue.push_back(neighbor_face);
                } else {
                    horizon.push(HorizonEdge {
                        from: vertices[edge],
                        to: vertices[(edge + 1) % 3],
                        outside: neighbor,
                    });
                }
            }
        }
        order_horizon_cycle(horizon).map(|horizon| (visible, horizon))
    }

    fn compact(self) -> SphericalDelaunay {
        let mut face_remap = vec![usize::MAX; self.faces.len()];
        let mut triangles = Vec::with_capacity(self.directions.len() * 2 - 4);
        for (face_index, face) in self.faces.iter().enumerate() {
            if face.alive {
                face_remap[face_index] = triangles.len();
                triangles.push(face.vertices);
            }
        }

        let mut opposite_half_edges = Vec::with_capacity(triangles.len() * 3);
        for face in self.faces.iter().filter(|face| face.alive) {
            for neighbor in face.neighbors {
                let triangle = face_remap[half_edge_face(neighbor)];
                debug_assert_ne!(triangle, usize::MAX);
                opposite_half_edges.push(flat_edge(triangle, half_edge_corner(neighbor)));
            }
        }

        SphericalDelaunay {
            points: self
                .directions
                .iter()
                .map(|&[x, y, z]| Vec3::new(x as f32, y as f32, z as f32))
                .collect(),
            directions: self.directions,
            triangles,
            opposite_half_edges,
        }
    }
}

/// A point's unit-length direction, in f64. Every geometric decision the
/// hull makes runs on these rather than on the f32 points themselves.
///
/// A convex hull separates two points by a plane, and reads any difference in
/// their distance from the origin as a weight on them: the plane tilts off
/// the angular bisector by that difference divided by the separation. An f32
/// unit vector sits about 6e-8 off the sphere, which for two cells closer
/// together than roughly 3.5e-4 — a separation a jittered 65,536-point
/// lattice does hold — is enough tilt to put one of them on the far side of
/// its own cell wall. A cell that excludes its own center has an inverted
/// corner, and no boundary integral over that ring is right.
///
/// Rounding back to f32 for the cell center afterwards is harmless: it moves
/// the center, not the planes that decided the topology.
fn direction(point: Vec3) -> [f64; 3] {
    let point = [f64::from(point.x), f64::from(point.y), f64::from(point.z)];
    let scale = length_squared(point).sqrt().recip();
    point.map(|axis| axis * scale)
}

/// The difference of two directions. Exact whenever the two agree in sign and
/// magnitude, which two directions a cell width apart do.
fn offset(to: [f64; 3], from: [f64; 3]) -> [f64; 3] {
    [to[0] - from[0], to[1] - from[1], to[2] - from[2]]
}

fn cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn length_squared(vector: [f64; 3]) -> f64 {
    dot(vector, vector)
}

pub(crate) const fn flat_edge(face: usize, edge: usize) -> usize {
    face * 3 + edge
}

pub(crate) const fn half_edge_face(edge: usize) -> usize {
    edge / 3
}

pub(crate) const fn half_edge_corner(edge: usize) -> usize {
    edge % 3
}

pub(crate) const fn next_half_edge(edge: usize) -> usize {
    if half_edge_corner(edge) == 2 {
        edge - 2
    } else {
        edge + 1
    }
}

fn tetrahedron_neighbors(faces: &[[usize; 3]; 4]) -> [[usize; 3]; 4] {
    std::array::from_fn(|face| {
        std::array::from_fn(|edge| {
            let from = faces[face][edge];
            let to = faces[face][(edge + 1) % 3];
            faces
                .iter()
                .enumerate()
                .find_map(|(neighbor_face, vertices)| {
                    (0..3)
                        .find(|&neighbor_edge| {
                            vertices[neighbor_edge] == to
                                && vertices[(neighbor_edge + 1) % 3] == from
                        })
                        .map(|neighbor_edge| flat_edge(neighbor_face, neighbor_edge))
                })
                .expect("tetrahedron edges must have an opposite")
        })
    })
}

fn order_horizon_cycle(edges: Vec<HorizonEdge>) -> Result<Vec<HorizonEdge>, TopologyError> {
    let Some(&first) = edges.first() else {
        return Err(TopologyError::BrokenHorizon);
    };
    let mut by_start = HashMap::with_capacity(edges.len());
    for edge in edges {
        if by_start.insert(edge.from, edge).is_some() {
            return Err(TopologyError::BrokenHorizon);
        }
    }

    let mut ordered = Vec::with_capacity(by_start.len());
    let mut current = first.from;
    while let Some(edge) = by_start.remove(&current) {
        current = edge.to;
        ordered.push(edge);
    }
    if !by_start.is_empty() || current != first.from {
        return Err(TopologyError::BrokenHorizon);
    }
    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tetrahedron_neighbors_are_reciprocal() {
        let faces = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
        let neighbors = tetrahedron_neighbors(&faces);

        for face in 0..4 {
            for edge in 0..3 {
                let neighbor = neighbors[face][edge];
                let neighbor_face = half_edge_face(neighbor);
                let neighbor_edge = half_edge_corner(neighbor);
                assert_eq!(
                    neighbors[neighbor_face][neighbor_edge],
                    flat_edge(face, edge)
                );
                assert_eq!(
                    faces[face][edge],
                    faces[neighbor_face][(neighbor_edge + 1) % 3]
                );
                assert_eq!(
                    faces[face][(edge + 1) % 3],
                    faces[neighbor_face][neighbor_edge]
                );
            }
        }
    }
}
