use crate::{SphericalDelaunay, TopologyError};
use procgen_core::Vec3;
use rayon::prelude::*;
use std::collections::VecDeque;

// Workload-specific tuning belongs with the algorithm, not in `procgen-core`.
const PARALLEL_THRESHOLD: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoronoiEdge {
    pub vertices: [usize; 2],
    pub cells: [usize; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellCorner {
    pub vertex: usize,
    pub neighbor: usize,
    pub edge: usize,
}

#[derive(Clone, Debug)]
pub struct SphereMesh {
    pub radius: f32,
    pub cell_centers: Vec<Vec3>,
    /// CSR offsets into the flat per-cell corner array.
    pub cell_offsets: Vec<usize>,
    pub corners: Vec<CellCorner>,
    pub cell_areas: Vec<f32>,
    pub vertices: Vec<Vec3>,
    pub vertex_cells: Vec<[usize; 3]>,
    pub vertex_neighbors: Vec<[usize; 3]>,
    pub edges: Vec<VoronoiEdge>,
}

impl SphereMesh {
    pub fn from_delaunay(delaunay: &SphericalDelaunay, radius: f32) -> Result<Self, TopologyError> {
        if !radius.is_finite() || radius <= 0.0 {
            return Err(TopologyError::InvalidRadius);
        }

        let points = delaunay.points();
        let triangles = delaunay.triangles();
        let cell_count = points.len();
        let triangle_count = delaunay.triangle_count();
        let half_edge_count = delaunay.half_edge_count();
        let cell_centers: Vec<Vec3> = points.iter().map(|&point| point * radius).collect();
        let unit_vertices: Vec<_> = (0..triangle_count)
            .map(|triangle| delaunay.triangle_circumcenter(triangle))
            .collect();
        let vertex_cells = triangles.to_vec();
        let vertex_neighbors = (0..triangle_count)
            .map(|triangle| delaunay.triangle_neighbors(triangle))
            .collect();

        let mut edges = Vec::with_capacity(half_edge_count / 2);
        let mut half_edge_to_edge = vec![usize::MAX; half_edge_count];
        for edge in delaunay.unique_edges() {
            let opposite = delaunay.opposite(edge);
            let edge_index = edges.len();
            edges.push(VoronoiEdge {
                vertices: [
                    delaunay.edge_triangle(edge),
                    delaunay.edge_triangle(opposite),
                ],
                cells: [delaunay.edge_origin(edge), delaunay.edge_destination(edge)],
            });
            half_edge_to_edge[edge] = edge_index;
            half_edge_to_edge[opposite] = edge_index;
        }

        let mut point_to_edge = vec![usize::MAX; cell_count];
        for edge in 0..half_edge_count {
            point_to_edge[delaunay.edge_destination(edge)] = edge;
        }
        debug_assert!(point_to_edge.iter().all(|&edge| edge != usize::MAX));

        let mut cell_offsets = Vec::with_capacity(cell_count + 1);
        let mut corners = Vec::with_capacity(half_edge_count);
        cell_offsets.push(0);
        for (cell, &start_edge) in point_to_edge.iter().enumerate() {
            for edge in delaunay.edges_around_point(start_edge) {
                debug_assert_eq!(delaunay.edge_destination(edge), cell);
                corners.push(CellCorner {
                    vertex: delaunay.edge_triangle(edge),
                    neighbor: delaunay.edge_origin(edge),
                    edge: half_edge_to_edge[edge],
                });
            }
            cell_offsets.push(corners.len());
        }

        let cell_areas = cell_offsets
            .par_windows(2)
            .with_min_len(PARALLEL_THRESHOLD)
            .enumerate()
            .map(|(cell, offsets)| {
                spherical_polygon_area(
                    points[cell],
                    &corners[offsets[0]..offsets[1]],
                    &unit_vertices,
                ) * radius
                    * radius
            })
            .collect();
        let vertices = unit_vertices
            .into_iter()
            .map(|vertex| vertex * radius)
            .collect();

        Ok(Self {
            radius,
            cell_centers,
            cell_offsets,
            corners,
            cell_areas,
            vertices,
            vertex_cells,
            vertex_neighbors,
            edges,
        })
    }

    pub fn cell_count(&self) -> usize {
        self.cell_centers.len()
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn cell_corners(&self, cell: usize) -> &[CellCorner] {
        &self.corners[self.cell_offsets[cell]..self.cell_offsets[cell + 1]]
    }

    /// Interpolates a surface position within one triangle of a cell's fan.
    /// Weights correspond to the cell center, the selected corner, and the
    /// next corner in ring order; they must be finite, nonnegative, and sum to one.
    pub fn interpolate_cell_triangle(
        &self,
        cell: usize,
        corner_index: usize,
        weights: [f32; 3],
    ) -> Vec3 {
        let corners = self.cell_corners(cell);
        assert!(corner_index < corners.len(), "cell corner must exist");
        assert!(
            weights
                .iter()
                .all(|weight| weight.is_finite() && *weight >= 0.0),
            "cell interpolation weights must be finite and nonnegative"
        );
        assert!(
            (weights.iter().sum::<f32>() - 1.0).abs() <= 1.0e-6,
            "cell interpolation weights must sum to one"
        );
        let next_corner_index = (corner_index + 1) % corners.len();
        let position = self.cell_centers[cell] * weights[0]
            + self.vertices[corners[corner_index].vertex] * weights[1]
            + self.vertices[corners[next_corner_index].vertex] * weights[2];
        position.normalized() * self.radius
    }
}

/// Collects connected components of eligible cells in ascending root-cell
/// order. The first cell in each component is its lowest-indexed member.
/// `passable` may impose an additional directed edge constraint.
pub fn connected_components(
    mesh: &SphereMesh,
    mut eligible: impl FnMut(usize) -> bool,
    mut passable: impl FnMut(usize, usize) -> bool,
) -> Vec<Vec<usize>> {
    let mut visited = vec![false; mesh.cell_count()];
    let mut components = Vec::new();
    for root in 0..mesh.cell_count() {
        if visited[root] || !eligible(root) {
            continue;
        }
        let mut component = Vec::new();
        let mut queue = VecDeque::from([root]);
        visited[root] = true;
        while let Some(cell) = queue.pop_front() {
            component.push(cell);
            for corner in mesh.cell_corners(cell) {
                let neighbor = corner.neighbor;
                if !visited[neighbor] && eligible(neighbor) && passable(cell, neighbor) {
                    visited[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }
    components
}

/// Computes minimum cell-hop distance from multiple sources while allowing a
/// caller to constrain each graph traversal step.
pub fn multi_source_distances(
    mesh: &SphereMesh,
    sources: &[usize],
    mut passable: impl FnMut(usize, usize) -> bool,
) -> Vec<Option<usize>> {
    let mut distances = vec![None; mesh.cell_count()];
    let mut queue = VecDeque::new();
    for &source in sources {
        assert!(
            source < mesh.cell_count(),
            "distance source must be a mesh cell"
        );
        if distances[source].is_none() {
            distances[source] = Some(0);
            queue.push_back(source);
        }
    }

    while let Some(cell) = queue.pop_front() {
        let next_distance = distances[cell].expect("queued cells have a distance") + 1;
        for corner in mesh.cell_corners(cell) {
            let neighbor = corner.neighbor;
            if distances[neighbor].is_none() && passable(cell, neighbor) {
                distances[neighbor] = Some(next_distance);
                queue.push_back(neighbor);
            }
        }
    }
    distances
}

fn spherical_polygon_area(center: Vec3, polygon: &[CellCorner], vertices: &[Vec3]) -> f32 {
    let mut area = 0.0;
    for index in 0..polygon.len() {
        let a = vertices[polygon[index].vertex];
        let b = vertices[polygon[(index + 1) % polygon.len()].vertex];
        let numerator = center.dot(a.cross(b)).abs();
        let denominator = 1.0 + center.dot(a) + a.dot(b) + b.dot(center);
        area += 2.0 * numerator.atan2(denominator);
    }
    area
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tetrahedron() -> SphereMesh {
        let points = [
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(-1.0, -1.0, 1.0),
            Vec3::new(-1.0, 1.0, -1.0),
            Vec3::new(1.0, -1.0, -1.0),
        ]
        .map(Vec3::normalized)
        .to_vec();
        SphereMesh::from_delaunay(&SphericalDelaunay::build(points).unwrap(), 1.0).unwrap()
    }

    #[test]
    fn multi_source_distance_edges_are_explicit() {
        let mesh = tetrahedron();

        assert_eq!(
            multi_source_distances(&mesh, &[], |_, _| true),
            vec![None; 4]
        );
        assert_eq!(
            multi_source_distances(&mesh, &[0], |_, _| false),
            vec![Some(0), None, None, None]
        );
        let single_source = multi_source_distances(&mesh, &[0], |_, _| true);
        assert_eq!(single_source, vec![Some(0), Some(1), Some(1), Some(1)]);
        assert_eq!(
            multi_source_distances(&mesh, &[0, 0], |_, _| true),
            single_source
        );
    }

    #[test]
    fn cell_triangle_interpolation_stays_in_the_selected_cell() {
        let mesh = tetrahedron();
        let position = mesh.interpolate_cell_triangle(0, 0, [0.25, 0.5, 0.25]);
        let center = mesh.cell_centers[0].normalized();

        assert!((position.length() - mesh.radius).abs() < 1.0e-6);
        for corner in mesh.cell_corners(0) {
            let neighbor = mesh.cell_centers[corner.neighbor].normalized();
            assert!(position.dot(center) + 1.0e-6 >= position.dot(neighbor));
        }
    }

    #[test]
    fn connected_components_are_filtered_and_root_ordered() {
        let mesh = tetrahedron();
        let components = connected_components(
            &mesh,
            |_| true,
            |cell, neighbor| (cell < 2) == (neighbor < 2),
        );
        assert_eq!(components, vec![vec![0, 1], vec![2, 3]]);
        assert!(
            components
                .iter()
                .all(|component| { component.first() == component.iter().min() })
        );

        assert_eq!(
            connected_components(&mesh, |cell| cell % 2 == 0, |_, _| true),
            vec![vec![0, 2]]
        );
        assert!(connected_components(&mesh, |_| false, |_, _| true).is_empty());
    }
}
