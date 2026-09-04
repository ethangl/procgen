use crate::{SphericalDelaunay, TopologyError};
use procgen_core::Vec3;
use rayon::prelude::*;

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
        let opposite_half_edges = delaunay.opposite_half_edges();
        let edge_triangle = SphericalDelaunay::edge_triangle;
        let cell_count = points.len();
        let triangle_count = delaunay.triangle_count();
        let cell_centers: Vec<Vec3> = points.iter().map(|&point| point * radius).collect();
        let unit_vertices: Vec<_> = (0..triangle_count)
            .map(|triangle| delaunay.triangle_circumcenter(triangle))
            .collect();
        let vertex_cells = triangles.to_vec();
        let vertex_neighbors = opposite_half_edges
            .chunks_exact(3)
            .map(|edges| {
                let edges: [usize; 3] = edges.try_into().expect("triangle has three half-edges");
                edges.map(edge_triangle)
            })
            .collect();

        let mut edges = Vec::with_capacity(opposite_half_edges.len() / 2);
        let mut half_edge_to_edge = vec![usize::MAX; opposite_half_edges.len()];
        for edge in delaunay.unique_edges() {
            let opposite = opposite_half_edges[edge];
            let edge_index = edges.len();
            edges.push(VoronoiEdge {
                vertices: [edge_triangle(edge), edge_triangle(opposite)],
                cells: [delaunay.edge_origin(edge), delaunay.edge_destination(edge)],
            });
            half_edge_to_edge[edge] = edge_index;
            half_edge_to_edge[opposite] = edge_index;
        }

        let mut point_to_edge = vec![usize::MAX; cell_count];
        for edge in 0..opposite_half_edges.len() {
            point_to_edge[delaunay.edge_destination(edge)] = edge;
        }
        debug_assert!(point_to_edge.iter().all(|&edge| edge != usize::MAX));

        let mut cell_offsets = Vec::with_capacity(cell_count + 1);
        let mut corners = Vec::with_capacity(opposite_half_edges.len());
        cell_offsets.push(0);
        for (cell, &start_edge) in point_to_edge.iter().enumerate() {
            for edge in delaunay.edges_around_point(start_edge) {
                debug_assert_eq!(delaunay.edge_destination(edge), cell);
                corners.push(CellCorner {
                    vertex: edge_triangle(edge),
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
