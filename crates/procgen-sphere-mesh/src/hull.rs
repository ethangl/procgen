use crate::{
    TopologyError,
    initial::{initial_tetrahedron, validate_points},
};
use procgen_core::Vec3;
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, VecDeque},
};

const VISIBILITY_EPSILON: f32 = 1.0e-7;

#[derive(Clone, Debug)]
pub struct SphericalDelaunay {
    points: Vec<Vec3>,
    /// Outward-facing triangles, counter-clockwise when viewed from outside.
    triangles: Vec<[usize; 3]>,
    /// Opposite half-edge for each flattened triangle edge.
    opposite_half_edges: Vec<usize>,
}

impl SphericalDelaunay {
    pub fn build(points: Vec<Vec3>) -> Result<Self, TopologyError> {
        validate_points(&points)?;
        QuickHull::build(points)
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    pub fn points(&self) -> &[Vec3] {
        &self.points
    }

    pub fn triangles(&self) -> &[[usize; 3]] {
        &self.triangles
    }

    pub fn opposite_half_edges(&self) -> &[usize] {
        &self.opposite_half_edges
    }

    pub fn edge_triangle(edge: usize) -> usize {
        edge / 3
    }

    fn next_half_edge(edge: usize) -> usize {
        if edge % 3 == 2 { edge - 2 } else { edge + 1 }
    }

    pub fn edge_origin(&self, edge: usize) -> usize {
        self.triangles[Self::edge_triangle(edge)][edge % 3]
    }

    pub fn edge_destination(&self, edge: usize) -> usize {
        self.triangles[Self::edge_triangle(edge)][Self::next_half_edge(edge) % 3]
    }

    pub fn unique_edges(&self) -> impl Iterator<Item = usize> + '_ {
        self.opposite_half_edges
            .iter()
            .enumerate()
            .filter_map(|(edge, &opposite)| (edge < opposite).then_some(edge))
    }

    pub fn triangle_circumcenter(&self, triangle: usize) -> Vec3 {
        let [p0, p1, p2] = self.triangles[triangle];
        let a = self.points[p0];
        let b = self.points[p1];
        let c = self.points[p2];
        let normal = (b - a).cross(c - a);
        normal.normalized()
    }

    pub(crate) fn edges_around_point(&self, start: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(Some(start), move |&incoming| {
            let next = self.opposite_half_edges[Self::next_half_edge(incoming)];
            (next != start).then_some(next)
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FaceEdge {
    face: usize,
    edge: usize,
}

impl FaceEdge {
    const fn new(face: usize, edge: usize) -> Self {
        Self { face, edge }
    }
}

#[derive(Clone, Copy, Debug)]
struct HorizonEdge {
    from: usize,
    to: usize,
    outside: FaceEdge,
}

#[derive(Debug)]
struct Face {
    vertices: [usize; 3],
    neighbors: [FaceEdge; 3],
    normal: Vec3,
    plane_distance: f32,
    alive: bool,
    conflicts: Vec<usize>,
    farthest_distance: f32,
    farthest_point: usize,
    visit_stamp: u64,
}

impl Face {
    fn distance_to(&self, point: Vec3) -> f32 {
        self.normal.dot(point) - self.plane_distance
    }
}

#[derive(Clone, Copy, Debug)]
struct HeapEntry {
    distance: f32,
    face: usize,
}

impl PartialEq for HeapEntry {
    fn eq(&self, other: &Self) -> bool {
        self.face == other.face && self.distance.total_cmp(&other.distance) == Ordering::Equal
    }
}

impl Eq for HeapEntry {}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.distance
            .total_cmp(&other.distance)
            .then_with(|| self.face.cmp(&other.face))
    }
}

struct QuickHull {
    points: Vec<Vec3>,
    faces: Vec<Face>,
    interior_point: Vec3,
    visit_stamp: u64,
}

impl QuickHull {
    fn build(points: Vec<Vec3>) -> Result<SphericalDelaunay, TopologyError> {
        let [i0, i1, i2, i3] = initial_tetrahedron(&points)?;
        let interior_point = (points[i0] + points[i1] + points[i2] + points[i3]) * 0.25;
        let expected_faces = points.len() * 2 - 4;
        let mut hull = Self {
            points,
            faces: Vec::with_capacity(expected_faces),
            interior_point,
            visit_stamp: 0,
        };

        let seed_faces = [[i0, i2, i1], [i0, i1, i3], [i0, i3, i2], [i1, i2, i3]];
        let seed_neighbors = tetrahedron_neighbors(&seed_faces);
        for (vertices, neighbors) in seed_faces.into_iter().zip(seed_neighbors) {
            hull.add_face(vertices, neighbors)?;
        }

        for point_index in 0..hull.points.len() {
            if ![i0, i1, i2, i3].contains(&point_index) {
                hull.assign_to_best_face(point_index, 0..hull.faces.len());
            }
        }

        let mut heap = BinaryHeap::new();
        hull.push_conflicting_faces(&mut heap, 0..hull.faces.len());

        while let Some(entry) = heap.pop() {
            let face = &hull.faces[entry.face];
            if !face.alive {
                continue;
            }

            let apex = face.farthest_point;
            let (new_faces, orphaned) = hull.expand(entry.face, apex)?;

            for point_index in orphaned {
                hull.assign_to_best_face(point_index, new_faces.clone());
            }
            hull.push_conflicting_faces(&mut heap, new_faces);
        }

        Ok(hull.compact())
    }

    fn expand(
        &mut self,
        initial_face: usize,
        apex: usize,
    ) -> Result<(std::ops::Range<usize>, Vec<usize>), TopologyError> {
        let (visible, horizon) = self.visible_faces_and_horizon(initial_face, self.points[apex])?;
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
                [
                    edge.outside,
                    FaceEdge::new(next, 2),
                    FaceEdge::new(previous, 1),
                ],
            )?;
            self.faces[edge.outside.face].neighbors[edge.outside.edge] = FaceEdge::new(face, 0);
        }

        Ok((new_start..self.faces.len(), orphaned))
    }

    fn add_face(
        &mut self,
        vertices: [usize; 3],
        neighbors: [FaceEdge; 3],
    ) -> Result<(), TopologyError> {
        let [v0, v1, v2] = vertices;
        let a = self.points[v0];
        let b = self.points[v1];
        let c = self.points[v2];
        let normal = (b - a).cross(c - a);
        if normal.length_squared() < 1.0e-16 {
            return Err(TopologyError::DegeneratePoints);
        }

        let centroid = (a + b + c) * (1.0 / 3.0);
        debug_assert!(
            normal.dot(centroid - self.interior_point) > 0.0,
            "QuickHull produced an inward-facing triangle"
        );
        let normal = normal.normalized();

        self.faces.push(Face {
            vertices,
            neighbors,
            normal,
            plane_distance: normal.dot(self.points[vertices[0]]),
            alive: true,
            conflicts: Vec::new(),
            farthest_distance: f32::NEG_INFINITY,
            farthest_point: usize::MAX,
            visit_stamp: 0,
        });
        Ok(())
    }

    fn push_conflicting_faces(
        &self,
        heap: &mut BinaryHeap<HeapEntry>,
        faces: impl Iterator<Item = usize>,
    ) {
        // A face is queued once, after its conflict set is complete.
        for face_index in faces {
            let face = &self.faces[face_index];
            if !face.conflicts.is_empty() {
                heap.push(HeapEntry {
                    distance: face.farthest_distance,
                    face: face_index,
                });
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
        let point = self.points[point_index];
        let mut best = None;
        for face_index in face_indices {
            let distance = self.faces[face_index].distance_to(point);
            if distance > VISIBILITY_EPSILON
                && best.is_none_or(|(_, best_distance)| distance > best_distance)
            {
                best = Some((face_index, distance));
            }
        }

        if let Some((face_index, distance)) = best {
            let face = &mut self.faces[face_index];
            face.conflicts.push(point_index);
            if distance > face.farthest_distance {
                face.farthest_distance = distance;
                face.farthest_point = point_index;
            }
        }
    }

    fn visible_faces_and_horizon(
        &mut self,
        initial: usize,
        apex: Vec3,
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
                debug_assert!(self.faces[neighbor.face].alive);
                if self.faces[neighbor.face].visit_stamp == visit_stamp {
                    continue;
                }
                if self.faces[neighbor.face].distance_to(apex) > VISIBILITY_EPSILON {
                    self.faces[neighbor.face].visit_stamp = visit_stamp;
                    queue.push_back(neighbor.face);
                } else {
                    horizon.push(HorizonEdge {
                        from: vertices[edge],
                        to: vertices[(edge + 1) % 3],
                        outside: neighbor,
                    });
                }
            }
        }
        Ok((visible, order_horizon_cycle(horizon)?))
    }

    fn compact(self) -> SphericalDelaunay {
        let mut face_remap = vec![usize::MAX; self.faces.len()];
        let mut triangles = Vec::with_capacity(self.points.len() * 2 - 4);
        for (face_index, face) in self.faces.iter().enumerate() {
            if face.alive {
                face_remap[face_index] = triangles.len();
                triangles.push(face.vertices);
            }
        }

        let mut opposite_half_edges = Vec::with_capacity(triangles.len() * 3);
        for face in self.faces.iter().filter(|face| face.alive) {
            for neighbor in face.neighbors {
                let triangle = face_remap[neighbor.face];
                debug_assert_ne!(triangle, usize::MAX);
                opposite_half_edges.push(triangle * 3 + neighbor.edge);
            }
        }

        SphericalDelaunay {
            points: self.points,
            triangles,
            opposite_half_edges,
        }
    }
}

fn tetrahedron_neighbors(faces: &[[usize; 3]; 4]) -> [[FaceEdge; 3]; 4] {
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
                        .map(|neighbor_edge| FaceEdge::new(neighbor_face, neighbor_edge))
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
                assert_eq!(
                    neighbors[neighbor.face][neighbor.edge],
                    FaceEdge::new(face, edge)
                );
                assert_eq!(
                    faces[face][edge],
                    faces[neighbor.face][(neighbor.edge + 1) % 3]
                );
                assert_eq!(
                    faces[face][(edge + 1) % 3],
                    faces[neighbor.face][neighbor.edge]
                );
            }
        }
    }
}
