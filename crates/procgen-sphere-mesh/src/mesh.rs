use crate::{SphericalDelaunay, TopologyError};
use procgen_sphere::Vec3;
use rayon::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoronoiEdge {
    pub vertices: [usize; 2],
    pub cells: [usize; 2],
}

#[derive(Clone, Debug)]
pub struct SphereMesh {
    pub radius: f32,
    pub cell_centers: Vec<Vec3>,
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
        let vertices: Vec<_> = (0..triangle_count)
            .map(|triangle| delaunay.triangle_circumcenter(triangle) * radius)
            .collect();
        let vertex_cells = delaunay.triangles.clone();
        let vertex_neighbors = (0..triangle_count)
            .map(|triangle| {
                let base = triangle * 3;
                [
                    SphericalDelaunay::half_edge_triangle(delaunay.opposite_half_edges[base]),
                    SphericalDelaunay::half_edge_triangle(delaunay.opposite_half_edges[base + 1]),
                    SphericalDelaunay::half_edge_triangle(delaunay.opposite_half_edges[base + 2]),
                ]
            })
            .collect();

        let mut point_to_edge = vec![usize::MAX; cell_count];
        for edge in 0..delaunay.opposite_half_edges.len() {
            let triangle = SphericalDelaunay::half_edge_triangle(edge);
            let local_edge = edge % 3;
            let endpoint = delaunay.triangles[triangle][(local_edge + 1) % 3];
            point_to_edge[endpoint] = edge;
        }

        let mut cell_vertices = vec![Vec::new(); cell_count];
        let mut cell_neighbors = vec![Vec::new(); cell_count];
        for cell in 0..cell_count {
            if point_to_edge[cell] == usize::MAX {
                return Err(TopologyError::OpenHull);
            }
            let around = delaunay.edges_around_point(point_to_edge[cell]);
            cell_vertices[cell] = around
                .iter()
                .map(|&edge| SphericalDelaunay::half_edge_triangle(edge))
                .collect();
            cell_neighbors[cell] = around
                .iter()
                .map(|&edge| {
                    delaunay.triangles[SphericalDelaunay::half_edge_triangle(edge)][edge % 3]
                })
                .collect();
        }

        let mut edges = Vec::with_capacity(delaunay.opposite_half_edges.len() / 2);
        let mut cell_edges = vec![Vec::new(); cell_count];
        for edge in 0..delaunay.opposite_half_edges.len() {
            let opposite = delaunay.opposite_half_edges[edge];
            if opposite < edge {
                continue;
            }
            let triangle = SphericalDelaunay::half_edge_triangle(edge);
            let local_edge = edge % 3;
            let cells = [
                delaunay.triangles[triangle][local_edge],
                delaunay.triangles[triangle][(local_edge + 1) % 3],
            ];
            let edge_index = edges.len();
            edges.push(VoronoiEdge {
                vertices: [triangle, SphericalDelaunay::half_edge_triangle(opposite)],
                cells,
            });
            cell_edges[cells[0]].push(edge_index);
            cell_edges[cells[1]].push(edge_index);
        }

        let cell_areas = cell_vertices
            .par_iter()
            .enumerate()
            .map(|(cell, polygon)| {
                spherical_polygon_area(cell_centers[cell], polygon, &vertices, radius)
            })
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

fn spherical_polygon_area(center: Vec3, polygon: &[usize], vertices: &[Vec3], radius: f32) -> f32 {
    let center = center * radius.recip();
    let mut area = 0.0;
    for index in 0..polygon.len() {
        let a = vertices[polygon[index]] * radius.recip();
        let b = vertices[polygon[(index + 1) % polygon.len()]] * radius.recip();
        let numerator = center.dot(a.cross(b)).abs();
        let denominator = 1.0 + center.dot(a) + a.dot(b) + b.dot(center);
        area += 2.0 * numerator.atan2(denominator);
    }
    area * radius * radius
}
