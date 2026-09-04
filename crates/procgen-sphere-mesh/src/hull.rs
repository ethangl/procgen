use crate::TopologyError;
use procgen_core::Vec3;
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, VecDeque},
};

const VISIBILITY_EPSILON: f32 = 1.0e-7;
const UNIT_SPHERE_TOLERANCE: f32 = 1.0e-4;

#[derive(Clone, Debug)]
pub struct SphericalDelaunay {
    pub points: Vec<Vec3>,
    /// Outward-facing triangles, counter-clockwise when viewed from outside.
    pub triangles: Vec<[usize; 3]>,
    /// Opposite half-edge for each flattened triangle edge.
    pub opposite_half_edges: Vec<usize>,
}

impl SphericalDelaunay {
    pub fn build(points: Vec<Vec3>) -> Result<Self, TopologyError> {
        validate_points(&points)?;
        QuickHull::build(points)
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    fn half_edge_triangle(edge: usize) -> usize {
        edge / 3
    }

    fn next_half_edge(edge: usize) -> usize {
        if edge % 3 == 2 { edge - 2 } else { edge + 1 }
    }

    pub(crate) fn edge_triangle(&self, edge: usize) -> usize {
        Self::half_edge_triangle(edge)
    }

    pub fn edge_origin(&self, edge: usize) -> usize {
        self.triangles[self.edge_triangle(edge)][edge % 3]
    }

    pub fn edge_destination(&self, edge: usize) -> usize {
        self.triangles[self.edge_triangle(edge)][Self::next_half_edge(edge) % 3]
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
        let centroid = (a + b + c) * (1.0 / 3.0);
        debug_assert!(normal.dot(centroid) > 0.0);
        normal.normalized()
    }

    pub(crate) fn edges_around_point(&self, start: usize) -> Vec<usize> {
        let mut edges = Vec::new();
        let mut incoming = start;

        loop {
            edges.push(incoming);
            let outgoing = Self::next_half_edge(incoming);
            incoming = self.opposite_half_edges[outgoing];
            if incoming == start {
                break;
            }
            debug_assert!(edges.len() <= self.opposite_half_edges.len());
        }

        edges
    }
}

fn validate_points(points: &[Vec3]) -> Result<(), TopologyError> {
    if points.len() < 4 {
        return Err(TopologyError::TooFewPoints);
    }

    for (index, point) in points.iter().enumerate() {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(TopologyError::NonFinitePoint { index });
        }
        let length = point.length();
        if (length - 1.0).abs() > UNIT_SPHERE_TOLERANCE {
            return Err(TopologyError::PointNotOnUnitSphere { index, length });
        }
    }

    Ok(())
}

#[derive(Debug)]
struct Face {
    vertices: [usize; 3],
    normal: Vec3,
    plane_distance: f32,
    alive: bool,
    conflicts: Vec<usize>,
    farthest_distance: f32,
    farthest_point: usize,
    visit_stamp: u32,
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
    directed_edges: HashMap<(usize, usize), usize>,
    interior_point: Vec3,
    visit_stamp: u32,
}

impl QuickHull {
    fn build(points: Vec<Vec3>) -> Result<SphericalDelaunay, TopologyError> {
        let [i0, i1, i2, i3] = initial_tetrahedron(&points)?;
        let interior_point = (points[i0] + points[i1] + points[i2] + points[i3]) * 0.25;
        let expected_faces = points.len() * 2 - 4;
        let mut hull = Self {
            points,
            faces: Vec::with_capacity(expected_faces),
            directed_edges: HashMap::with_capacity(expected_faces * 3),
            interior_point,
            visit_stamp: 0,
        };

        hull.add_face([i0, i2, i1])?;
        hull.add_face([i0, i1, i3])?;
        hull.add_face([i0, i3, i2])?;
        hull.add_face([i1, i2, i3])?;

        let mut in_hull = vec![false; hull.points.len()];
        for index in [i0, i1, i2, i3] {
            in_hull[index] = true;
        }

        for (point_index, is_in_hull) in in_hull.iter().copied().enumerate() {
            if !is_in_hull {
                hull.assign_to_best_face(point_index, 0..hull.faces.len());
            }
        }

        let mut heap = BinaryHeap::new();
        for (face_index, face) in hull.faces.iter().enumerate() {
            if !face.conflicts.is_empty() {
                heap.push(HeapEntry {
                    distance: face.farthest_distance,
                    face: face_index,
                });
            }
        }

        while let Some(entry) = heap.pop() {
            let face = &hull.faces[entry.face];
            if !face.alive || face.conflicts.is_empty() || face.farthest_distance != entry.distance
            {
                continue;
            }

            let apex = face.farthest_point;
            let (visible, visit_stamp) = hull.visible_faces(entry.face, hull.points[apex]);
            let horizon = hull.horizon(&visible, visit_stamp)?;
            let mut orphaned = Vec::new();
            for &face_index in &visible {
                orphaned.extend(
                    hull.faces[face_index]
                        .conflicts
                        .iter()
                        .copied()
                        .filter(|&point| point != apex),
                );
            }

            for &face_index in &visible {
                hull.remove_face(face_index);
            }

            let new_start = hull.faces.len();
            for (from, to) in horizon {
                hull.add_face([from, to, apex])?;
            }
            let new_end = hull.faces.len();
            in_hull[apex] = true;

            for point_index in orphaned {
                hull.assign_to_best_face(point_index, new_start..new_end);
            }
            for face_index in new_start..new_end {
                let face = &hull.faces[face_index];
                if !face.conflicts.is_empty() {
                    heap.push(HeapEntry {
                        distance: face.farthest_distance,
                        face: face_index,
                    });
                }
            }
        }

        hull.compact()
    }

    fn add_face(&mut self, vertices: [usize; 3]) -> Result<(), TopologyError> {
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

        let face_index = self.faces.len();
        self.faces.push(Face {
            vertices,
            normal,
            plane_distance: normal.dot(self.points[vertices[0]]),
            alive: true,
            conflicts: Vec::new(),
            farthest_distance: f32::NEG_INFINITY,
            farthest_point: usize::MAX,
            visit_stamp: 0,
        });
        for edge in 0..3 {
            let from = vertices[edge];
            let to = vertices[(edge + 1) % 3];
            self.directed_edges
                .insert((from, to), face_index * 3 + edge);
        }
        Ok(())
    }

    fn remove_face(&mut self, face_index: usize) {
        let face = &mut self.faces[face_index];
        for edge in 0..3 {
            self.directed_edges
                .remove(&(face.vertices[edge], face.vertices[(edge + 1) % 3]));
        }
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

    fn visible_faces(&mut self, initial: usize, apex: Vec3) -> (Vec<usize>, u32) {
        self.visit_stamp = self.visit_stamp.wrapping_add(1);
        if self.visit_stamp == 0 {
            for face in &mut self.faces {
                face.visit_stamp = 0;
            }
            self.visit_stamp = 1;
        }
        let visit_stamp = self.visit_stamp;
        let mut visible = Vec::new();
        let mut queue = VecDeque::from([initial]);
        self.faces[initial].visit_stamp = visit_stamp;

        while let Some(face_index) = queue.pop_front() {
            visible.push(face_index);
            let vertices = self.faces[face_index].vertices;
            for edge in 0..3 {
                let from = vertices[edge];
                let to = vertices[(edge + 1) % 3];
                if let Some(opposite) = self.directed_edges.get(&(to, from)) {
                    let neighbor = opposite / 3;
                    if self.faces[neighbor].visit_stamp != visit_stamp
                        && self.faces[neighbor].alive
                        && self.faces[neighbor].distance_to(apex) > VISIBILITY_EPSILON
                    {
                        self.faces[neighbor].visit_stamp = visit_stamp;
                        queue.push_back(neighbor);
                    }
                }
            }
        }
        (visible, visit_stamp)
    }

    fn horizon(
        &self,
        visible_faces: &[usize],
        visit_stamp: u32,
    ) -> Result<Vec<(usize, usize)>, TopologyError> {
        let mut edges = Vec::new();
        for &face_index in visible_faces {
            let face = &self.faces[face_index];
            for edge in 0..3 {
                let from = face.vertices[edge];
                let to = face.vertices[(edge + 1) % 3];
                let opposite = self
                    .directed_edges
                    .get(&(to, from))
                    .ok_or(TopologyError::OpenHull)?;
                if self.faces[opposite / 3].visit_stamp != visit_stamp {
                    edges.push((from, to));
                }
            }
        }
        validate_horizon_cycle(&edges)?;
        Ok(edges)
    }

    fn compact(self) -> Result<SphericalDelaunay, TopologyError> {
        let triangles: Vec<_> = self
            .faces
            .into_iter()
            .filter(|face| face.alive)
            .map(|face| face.vertices)
            .collect();
        let mut edge_lookup = HashMap::with_capacity(triangles.len() * 3);
        for (triangle, vertices) in triangles.iter().enumerate() {
            for edge in 0..3 {
                edge_lookup.insert(
                    (vertices[edge], vertices[(edge + 1) % 3]),
                    triangle * 3 + edge,
                );
            }
        }

        let mut opposite_half_edges = Vec::with_capacity(triangles.len() * 3);
        for vertices in &triangles {
            for edge in 0..3 {
                opposite_half_edges.push(
                    *edge_lookup
                        .get(&(vertices[(edge + 1) % 3], vertices[edge]))
                        .ok_or(TopologyError::OpenHull)?,
                );
            }
        }

        Ok(SphericalDelaunay {
            points: self.points,
            triangles,
            opposite_half_edges,
        })
    }
}

fn initial_tetrahedron(points: &[Vec3]) -> Result<[usize; 4], TopologyError> {
    let i0 = (0..points.len())
        .max_by(|&a, &b| points[a].x.total_cmp(&points[b].x))
        .ok_or(TopologyError::DegeneratePoints)?;
    let i1 = (0..points.len())
        .filter(|&index| index != i0)
        .max_by(|&a, &b| {
            points[a]
                .distance_squared(points[i0])
                .total_cmp(&points[b].distance_squared(points[i0]))
        })
        .ok_or(TopologyError::DegeneratePoints)?;

    let baseline = points[i1] - points[i0];
    let i2 = (0..points.len())
        .filter(|&index| index != i0 && index != i1)
        .max_by(|&a, &b| {
            baseline
                .cross(points[a] - points[i0])
                .length_squared()
                .total_cmp(&baseline.cross(points[b] - points[i0]).length_squared())
        })
        .ok_or(TopologyError::DegeneratePoints)?;

    let normal = baseline.cross(points[i2] - points[i0]);
    let (i3, signed_volume) = (0..points.len())
        .filter(|&index| index != i0 && index != i1 && index != i2)
        .map(|index| (index, normal.dot(points[index] - points[i0])))
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .ok_or(TopologyError::DegeneratePoints)?;

    if signed_volume.abs() < 1.0e-10 {
        return Err(TopologyError::DegeneratePoints);
    }
    if signed_volume > 0.0 {
        Ok([i0, i1, i2, i3])
    } else {
        Ok([i0, i2, i1, i3])
    }
}

fn validate_horizon_cycle(edges: &[(usize, usize)]) -> Result<(), TopologyError> {
    let Some(&first) = edges.first() else {
        return Err(TopologyError::BrokenHorizon);
    };
    let mut by_start = HashMap::with_capacity(edges.len());
    for &edge in edges {
        if by_start.insert(edge.0, edge.1).is_some() {
            return Err(TopologyError::BrokenHorizon);
        }
    }

    let mut current = first.0;
    while let Some(next) = by_start.remove(&current) {
        current = next;
    }
    if !by_start.is_empty() || current != first.0 {
        return Err(TopologyError::BrokenHorizon);
    }
    Ok(())
}
