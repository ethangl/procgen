use crate::{SphericalDelaunay, TopologyError};
use procgen_core::Vec3;
use rayon::prelude::*;

const PARALLEL_THRESHOLD: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoronoiEdge {
    pub vertices: [usize; 2],
    pub cells: [usize; 2],
}

#[derive(Clone, Debug)]
pub struct SphereMesh {
    pub radius: f32,
    pub cell_centers: Vec<Vec3>,
    /// Per-cell rings. At index `i`, all three entries describe the same corner:
    /// its Voronoi vertex, neighboring cell, and separating Voronoi edge.
    pub cell_vertices: Vec<Vec<usize>>,
    pub cell_neighbors: Vec<Vec<usize>>,
    pub cell_edges: Vec<Vec<usize>>,
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

        let cell_count = delaunay.points.len();
        let triangle_count = delaunay.triangle_count();
        let cell_centers: Vec<Vec3> = delaunay
            .points
            .iter()
            .map(|&point| point * radius)
            .collect();
        let unit_vertices: Vec<_> = (0..triangle_count)
            .map(|triangle| delaunay.triangle_circumcenter(triangle))
            .collect();
        let vertex_cells = delaunay.triangles.clone();
        let vertex_neighbors = (0..triangle_count)
            .map(|triangle| {
                let base = triangle * 3;
                [
                    delaunay.edge_triangle(delaunay.opposite_half_edges[base]),
                    delaunay.edge_triangle(delaunay.opposite_half_edges[base + 1]),
                    delaunay.edge_triangle(delaunay.opposite_half_edges[base + 2]),
                ]
            })
            .collect();

        let mut edges = Vec::with_capacity(delaunay.opposite_half_edges.len() / 2);
        let mut half_edge_to_edge = vec![usize::MAX; delaunay.opposite_half_edges.len()];
        for edge in delaunay.unique_edges() {
            let opposite = delaunay.opposite_half_edges[edge];
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
        for edge in 0..delaunay.opposite_half_edges.len() {
            point_to_edge[delaunay.edge_destination(edge)] = edge;
        }

        let mut cell_vertices = vec![Vec::new(); cell_count];
        let mut cell_neighbors = vec![Vec::new(); cell_count];
        let mut cell_edges = vec![Vec::new(); cell_count];
        for cell in 0..cell_count {
            if point_to_edge[cell] == usize::MAX {
                return Err(TopologyError::OpenHull);
            }
            let around = delaunay.edges_around_point(point_to_edge[cell]);
            debug_assert!(
                around
                    .iter()
                    .all(|&edge| delaunay.edge_destination(edge) == cell)
            );
            cell_vertices[cell] = around
                .iter()
                .map(|&edge| delaunay.edge_triangle(edge))
                .collect();
            cell_neighbors[cell] = around
                .iter()
                .map(|&edge| delaunay.edge_origin(edge))
                .collect();
            cell_edges[cell] = around.iter().map(|&edge| half_edge_to_edge[edge]).collect();
        }

        let cell_areas = cell_vertices
            .par_iter()
            .with_min_len(PARALLEL_THRESHOLD)
            .enumerate()
            .map(|(cell, polygon)| {
                spherical_polygon_area(delaunay.points[cell], polygon, &unit_vertices)
                    * radius
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
            cell_vertices,
            cell_neighbors,
            cell_edges,
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
}

fn spherical_polygon_area(center: Vec3, polygon: &[usize], vertices: &[Vec3]) -> f32 {
    let mut area = 0.0;
    for index in 0..polygon.len() {
        let a = vertices[polygon[index]];
        let b = vertices[polygon[(index + 1) % polygon.len()]];
        let numerator = center.dot(a.cross(b)).abs();
        let denominator = 1.0 + center.dot(a) + a.dot(b) + b.dot(center);
        area += 2.0 * numerator.atan2(denominator);
    }
    area
}
