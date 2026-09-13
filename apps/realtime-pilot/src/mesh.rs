use crate::{PlanetError, RegionAddress};
use procgen_core::Vec3;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceTriangle {
    pub vertices: [u32; 3],
    /// Lowest-address incident region owns a polygon, including a face seam.
    pub region: RegionAddress,
}

/// Closed-surface diagnostics. Invalid vertex links include boundary vertices;
/// edge counts distinguish an open boundary from an edge shared by extra sheets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshTopology {
    pub open_edges: usize,
    pub nonmanifold_edges: usize,
    pub nonmanifold_vertices: usize,
    pub unbalanced_edges: usize,
    pub degenerate_triangles: usize,
}

impl MeshTopology {
    pub fn is_closed_manifold(self) -> bool {
        self.open_edges == 0
            && self.nonmanifold_edges == 0
            && self.nonmanifold_vertices == 0
            && self.unbalanced_edges == 0
            && self.degenerate_triangles == 0
    }
}

pub struct SurfaceMesh {
    pub(crate) positions: Vec<Vec3>,
    pub(crate) triangles: Vec<SurfaceTriangle>,
}
impl SurfaceMesh {
    pub fn positions(&self) -> &[Vec3] {
        &self.positions
    }
    pub fn triangles(&self) -> &[SurfaceTriangle] {
        &self.triangles
    }
    pub fn validate(&self) -> Result<(), PlanetError> {
        if self.positions.iter().any(|p| !p.is_finite())
            || self.triangles.iter().any(|t| {
                t.vertices
                    .iter()
                    .any(|&i| i as usize >= self.positions.len())
                    || t.vertices[0] == t.vertices[1]
                    || t.vertices[1] == t.vertices[2]
                    || t.vertices[2] == t.vertices[0]
            })
        {
            return Err(PlanetError::Mesh);
        }
        Ok(())
    }
    pub fn vertex_normals(&self) -> Vec<Vec3> {
        let mut normals = vec![Vec3::ZERO; self.positions.len()];
        for triangle in &self.triangles {
            let [a, b, c] = triangle.vertices.map(|i| self.positions[i as usize]);
            let normal = (b - a).cross(c - a);
            for id in triangle.vertices {
                normals[id as usize] = normals[id as usize] + normal;
            }
        }
        normals.into_iter().map(Vec3::normalized).collect()
    }

    pub fn topology(&self) -> MeshTopology {
        let mut edges = BTreeMap::<[u32; 2], (usize, i32)>::new();
        let mut degenerate_triangles = 0;
        for triangle in &self.triangles {
            let t = triangle.vertices;
            let [a, b, c] = t.map(|i| self.positions[i as usize]);
            if (b - a).cross(c - a).length_squared() == 0.0 {
                degenerate_triangles += 1;
            }
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let entry = edges.entry([a.min(b), a.max(b)]).or_default();
                entry.0 += 1;
                entry.1 += if a < b { 1 } else { -1 };
            }
        }
        MeshTopology {
            open_edges: edges.values().filter(|&&(count, _)| count == 1).count(),
            nonmanifold_edges: edges.values().filter(|&&(count, _)| count > 2).count(),
            unbalanced_edges: edges.values().filter(|&&(_, balance)| balance != 0).count(),
            nonmanifold_vertices: invalid_vertex_links(self.triangles.iter().map(|t| t.vertices))
                .len(),
            degenerate_triangles,
        }
    }

    /// Rendering changes coordinates, never field inputs or mesh identities.
    pub fn relative_positions(&self, origin: Vec3) -> Result<Vec<Vec3>, PlanetError> {
        if !origin.is_finite() {
            return Err(PlanetError::Mesh);
        }
        let positions: Vec<_> = self.positions.iter().map(|&p| p - origin).collect();
        if positions.iter().any(|p| !p.is_finite()) {
            return Err(PlanetError::Mesh);
        }
        Ok(positions)
    }
}

/// A closed manifold vertex has one circular link: each neighboring vertex
/// occurs twice and the link is connected. Edge counts alone miss pinched sheets.
pub(crate) fn invalid_vertex_links(triangles: impl Iterator<Item = [u32; 3]>) -> BTreeSet<u32> {
    let mut links: Vec<Vec<[u32; 2]>> = Vec::new();
    for t in triangles {
        let size = *t.iter().max().unwrap() as usize + 1;
        if size > links.len() {
            links.resize_with(size, Vec::new);
        }
        for i in 0..3 {
            links[t[i] as usize].push([t[(i + 1) % 3], t[(i + 2) % 3]]);
        }
    }
    links
        .into_iter()
        .enumerate()
        .filter_map(|(vertex, edges)| {
            if edges.is_empty() {
                return None;
            }
            let mut neighbors: Vec<_> = edges
                .into_iter()
                .flat_map(|[a, b]| [(a, b), (b, a)])
                .collect();
            neighbors.sort_unstable();
            if neighbors
                .chunks_exact(2)
                .any(|p| p[0].0 != p[1].0 || p[0].1 == p[1].1)
                || neighbors.windows(3).any(|p| p[0].0 == p[2].0)
            {
                return Some(vertex as u32);
            }
            let start = neighbors[0].0;
            let mut previous = start;
            let mut current = neighbors[0].1;
            let mut count = 1;
            while current != start {
                let i = neighbors.partition_point(|&(v, _)| v < current);
                let next = if neighbors[i].1 != previous {
                    neighbors[i].1
                } else {
                    neighbors[i + 1].1
                };
                previous = current;
                current = next;
                count += 1;
            }
            (count * 2 != neighbors.len()).then_some(vertex as u32)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RegionAddress;
    use procgen_cubesphere::CubeFace;
    #[test]
    fn closed_edge_counts_do_not_hide_a_pinched_vertex() {
        let tetrahedron = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
        assert!(invalid_vertex_links(tetrahedron.into_iter()).is_empty());
        let triangles = tetrahedron
            .into_iter()
            .chain(tetrahedron.map(|t| t.map(|i| if i == 0 { 0 } else { i + 3 })));
        let mesh = SurfaceMesh {
            positions: vec![
                Vec3::ZERO,
                Vec3::X,
                Vec3::Y,
                Vec3::Z,
                -Vec3::X,
                -Vec3::Y,
                -Vec3::Z,
            ],
            triangles: triangles
                .map(|vertices| SurfaceTriangle {
                    vertices,
                    region: RegionAddress {
                        face: CubeFace::PositiveX,
                    },
                })
                .collect(),
        };
        let topology = mesh.topology();
        assert_eq!(topology.open_edges, 0);
        assert_eq!(topology.nonmanifold_edges, 0);
        assert_eq!(topology.unbalanced_edges, 0);
        assert_eq!(topology.nonmanifold_vertices, 1);
        assert_eq!(
            invalid_vertex_links(mesh.triangles.iter().map(|t| t.vertices)),
            BTreeSet::from([0])
        );
    }
}
