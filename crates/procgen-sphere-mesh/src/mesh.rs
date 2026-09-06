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

    pub fn total_area(&self) -> f64 {
        self.cell_areas.iter().map(|&area| f64::from(area)).sum()
    }

    pub fn area_weighted_mean(&self, values: &[f32]) -> f64 {
        assert_eq!(
            values.len(),
            self.cell_count(),
            "values must match the mesh cell count"
        );
        values
            .iter()
            .zip(&self.cell_areas)
            .map(|(&value, &area)| f64::from(value) * f64::from(area))
            .sum::<f64>()
            / self.total_area()
    }

    /// Fits a tangent gradient for a scalar cell field using each cell's
    /// immediate neighbors. The result is expressed in field units per radian
    /// and is independent of mesh radius. The local basis follows the model's
    /// Y-up convention, with a deterministic X-axis fallback at exact poles.
    pub fn cell_gradients(&self, values: &[f32]) -> Vec<Vec3> {
        assert_eq!(
            values.len(),
            self.cell_count(),
            "values must match the mesh cell count"
        );
        (0..self.cell_count())
            .map(|cell| cell_gradient(self, values, cell))
            .collect()
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

fn cell_gradient(mesh: &SphereMesh, values: &[f32], cell: usize) -> Vec3 {
    let normal = mesh.cell_centers[cell].normalized();
    let (east, north) = local_tangent_basis(normal);
    let mut xx = 0.0_f64;
    let mut xy = 0.0_f64;
    let mut yy = 0.0_f64;
    let mut bx = 0.0_f64;
    let mut by = 0.0_f64;
    for corner in mesh.cell_corners(cell) {
        let neighbor = mesh.cell_centers[corner.neighbor].normalized();
        let cosine = f64::from(normal.dot(neighbor)).clamp(-1.0, 1.0);
        let angle = cosine.acos();
        let tangent = (neighbor - normal * normal.dot(neighbor)).normalized();
        let x = f64::from(tangent.dot(east)) * angle;
        let y = f64::from(tangent.dot(north)) * angle;
        let delta = f64::from(values[corner.neighbor] - values[cell]);
        xx += x * x;
        xy += x * y;
        yy += y * y;
        bx += x * delta;
        by += y * delta;
    }
    let determinant = xx * yy - xy * xy;
    if determinant.abs() <= 1.0e-18 {
        return Vec3::ZERO;
    }
    let eastward = (yy * bx - xy * by) / determinant;
    let northward = (xx * by - xy * bx) / determinant;
    east * eastward as f32 + north * northward as f32
}

fn local_tangent_basis(normal: Vec3) -> (Vec3, Vec3) {
    let axis = Vec3::new(0.0, 1.0, 0.0);
    let mut east = axis.cross(normal);
    if east.length_squared() <= 1.0e-12 {
        east = Vec3::new(1.0, 0.0, 0.0);
    }
    east = east.normalized();
    (east, normal.cross(east).normalized())
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

/// Computes minimum cell-hop distance from both cells of every mesh edge that
/// matches `eligible`. Traversal is unconstrained after selecting source cells.
pub fn edge_cell_distances(
    mesh: &SphereMesh,
    mut eligible: impl FnMut(usize, &VoronoiEdge) -> bool,
) -> Vec<Option<usize>> {
    let mut sources = Vec::new();
    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        if eligible(edge_index, edge) {
            sources.extend(edge.cells);
        }
    }
    multi_source_distances(mesh, &sources, |_, _| true)
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
    fn cell_gradients_are_tangent_and_constant_fields_are_zero() {
        let mesh = tetrahedron();
        assert_eq!(mesh.cell_gradients(&[4.0; 4]), vec![Vec3::ZERO; 4]);

        let values = mesh
            .cell_centers
            .iter()
            .map(|center| center.x + 2.0 * center.y)
            .collect::<Vec<_>>();
        for (cell, gradient) in mesh.cell_gradients(&values).iter().enumerate() {
            assert!(gradient.length() > 0.0);
            assert!(gradient.dot(mesh.cell_centers[cell].normalized()).abs() <= 1.0e-6);
        }
    }

    #[test]
    fn edge_cell_distance_sources_both_cells_of_matching_edges() {
        let mesh = tetrahedron();
        let matching_edge = 0;
        let source_cells = mesh.edges[matching_edge].cells;
        let distances = edge_cell_distances(&mesh, |edge, _| edge == matching_edge);

        for cell in source_cells {
            assert_eq!(distances[cell], Some(0));
        }
        assert!(
            distances
                .iter()
                .enumerate()
                .filter(|(cell, _)| !source_cells.contains(cell))
                .all(|(_, distance)| *distance == Some(1))
        );
        assert_eq!(
            edge_cell_distances(&mesh, |_, _| false),
            vec![None; mesh.cell_count()]
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
