use std::collections::BTreeMap;

use procgen_core::Vec3;
use rayon::prelude::*;

use crate::{
    PlanetError, PlanetField, RegionAddress, ShellVolume,
    qef::{Constraint, fit},
    shell::{Sample, face_grid},
};

// Cube corners use x + 2*y + 4*z. Each edge is listed once.
pub const OVERVIEW_FACE_QUADS: usize = 16;

const EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceTriangle {
    pub vertices: [u32; 3],
    /// Lowest-address incident region owns a polygon, including a face seam.
    pub region: RegionAddress,
}

/// Derived inspection data; not a promise that arbitrary density topology is manifold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshTopology {
    pub open_edges: usize,
    pub nonmanifold_edges: usize,
    pub unbalanced_edges: usize,
    pub degenerate_triangles: usize,
}

pub struct SurfaceMesh {
    positions: Vec<Vec3>,
    triangles: Vec<SurfaceTriangle>,
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

struct CellVertex {
    region: RegionAddress,
    samples: [usize; 8],
    position: Vec3,
    center: Vec3,
    crossings: Vec<[usize; 2]>,
}

/// Contour the trilinear reconstruction of the sampled final density. A cell
/// with several disconnected sheets still has one vertex: thin features need
/// more samples; manifold topology is not guaranteed for arbitrary fields.
pub fn contour_shell(volume: &ShellVolume) -> Result<SurfaceMesh, PlanetError> {
    volume.validate()?;
    let cells: Vec<_> = volume
        .cells()
        .map(|(region, ids)| {
            let samples = ids.map(|i| volume.samples[i]);
            cell_vertex(samples).map(|position| {
                let crossings = EDGES
                    .iter()
                    .filter(|&&(a, b)| crosses(samples[a].density, samples[b].density))
                    .map(|&(a, b)| {
                        let mut key = [ids[a], ids[b]];
                        key.sort();
                        key
                    })
                    .collect();
                CellVertex {
                    region,
                    samples: ids,
                    position,
                    center: interpolate_positions(samples, Vec3::new(0.5, 0.5, 0.5)),
                    crossings,
                }
            })
        })
        .collect();
    // Indexed collection preserves cell order regardless of the Rayon schedule.
    let cells: Vec<_> = cells.into_iter().flatten().collect();
    let mut edges: BTreeMap<[usize; 2], Vec<usize>> = BTreeMap::new();
    for (id, cell) in cells.iter().enumerate() {
        for &edge in &cell.crossings {
            edges.entry(edge).or_default().push(id);
        }
    }
    let mut triangles = Vec::new();
    for (edge, incident) in edges {
        let mut ring = cell_ring(&cells, &incident)?;
        let first = volume.samples[edge[0]];
        let last = volume.samples[edge[1]];
        let outward = if first.density >= 0.0 {
            last.position - first.position
        } else {
            first.position - last.position
        };
        let center = cells[ring[0]].center;
        let normal = (cells[ring[1]].center - center).cross(cells[ring[2]].center - center);
        if normal.dot(outward) < 0.0 {
            ring[1..].reverse();
        }
        let region = incident
            .iter()
            .map(|&i| cells[i].region)
            .min()
            .expect("nonempty ring");
        for i in 1..ring.len() - 1 {
            triangles.push(SurfaceTriangle {
                vertices: [ring[0] as u32, ring[i] as u32, ring[i + 1] as u32],
                region,
            });
        }
    }
    let mesh = SurfaceMesh {
        positions: cells.into_iter().map(|c| c.position).collect(),
        triangles,
    };
    mesh.validate()?;
    Ok(mesh)
}

// Walk cell-face adjacency, not a floating-point angle sort. This handles the
// three-cell radial edge at a cube corner as well as ordinary four-cell edges.
fn cell_ring(cells: &[CellVertex], incident: &[usize]) -> Result<Vec<usize>, PlanetError> {
    if !(3..=4).contains(&incident.len()) {
        return Err(PlanetError::Topology);
    }
    let adjacent = |a: usize, b: usize| {
        cells[a]
            .samples
            .iter()
            .filter(|id| cells[b].samples.contains(id))
            .count()
            == 4
    };
    let mut ring = vec![incident[0]];
    while ring.len() < incident.len() {
        let last = *ring.last().unwrap();
        let next = incident
            .iter()
            .copied()
            .find(|id| !ring.contains(id) && adjacent(last, *id))
            .ok_or(PlanetError::Topology)?;
        ring.push(next);
    }
    if !adjacent(ring[0], *ring.last().unwrap()) {
        return Err(PlanetError::Topology);
    }
    Ok(ring)
}

fn crosses(a: f32, b: f32) -> bool {
    (a >= 0.0) != (b >= 0.0)
}
fn corner(i: usize) -> Vec3 {
    Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32)
}
fn cell_vertex(samples: [Sample; 8]) -> Option<Vec3> {
    let constraints: Vec<_> = EDGES
        .iter()
        .filter_map(|&(a, b)| {
            let da = samples[a].density;
            let db = samples[b].density;
            if !crosses(da, db) {
                return None;
            }
            // Exact root of the reconstructed field along this edge; zero is solid.
            let t = da / (da - db);
            let position = corner(a) + (corner(b) - corner(a)) * t;
            let normal = density_gradient(samples, position).normalized();
            // A zero gradient contributes no plane but still contributes to the mass point.
            Some(Constraint { position, normal })
        })
        .collect();
    if constraints.is_empty() {
        None
    } else {
        Some(interpolate_positions(samples, fit(&constraints)))
    }
}
fn weights(p: Vec3) -> [f32; 8] {
    std::array::from_fn(|i| {
        (if i & 1 == 0 { 1.0 - p.x } else { p.x })
            * (if i & 2 == 0 { 1.0 - p.y } else { p.y })
            * (if i & 4 == 0 { 1.0 - p.z } else { p.z })
    })
}
fn interpolate_positions(samples: [Sample; 8], p: Vec3) -> Vec3 {
    samples
        .iter()
        .zip(weights(p))
        .fold(Vec3::ZERO, |sum, (s, w)| sum + s.position * w)
}
fn density_gradient(samples: [Sample; 8], p: Vec3) -> Vec3 {
    let mut gradient = Vec3::ZERO;
    for (i, sample) in samples.iter().enumerate() {
        let c = corner(i);
        let w = Vec3::new(
            if c.x == 0.0 { 1.0 - p.x } else { p.x },
            if c.y == 0.0 { 1.0 - p.y } else { p.y },
            if c.z == 0.0 { 1.0 - p.z } else { p.z },
        );
        gradient = gradient
            + Vec3::new(
                (2.0 * c.x - 1.0) * w.y * w.z,
                w.x * (2.0 * c.y - 1.0) * w.z,
                w.x * w.y * (2.0 * c.z - 1.0),
            ) * sample.density;
    }
    gradient
}

/// Cheap closed height mesh from the identical broad elevation function.
pub fn planet_overview(field: &PlanetField, face_quads: usize) -> Result<SurfaceMesh, PlanetError> {
    crate::ShellConfig {
        face_quads,
        radial_cells: 2,
    }
    .validate()?;
    let (directions, quads) = face_grid(face_quads);
    let positions = directions
        .par_iter()
        .map(|&d| d * (field.config().radius + field.height(d)))
        .collect();
    let triangles = quads
        .into_iter()
        .flat_map(|q| {
            [[0, 1, 3], [0, 3, 2]].map(|t| SurfaceTriangle {
                vertices: t.map(|i| q.columns[i] as u32),
                region: q.region,
            })
        })
        .collect();
    let mesh = SurfaceMesh {
        positions,
        triangles,
    };
    mesh.validate()?;
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PILOT_PLANET, PILOT_SHELL, PlanetConfig, ShellConfig, TerrainConfig, sample_shell,
    };
    fn closed(mesh: &SurfaceMesh) {
        assert_eq!(
            mesh.topology(),
            MeshTopology {
                open_edges: 0,
                nonmanifold_edges: 0,
                unbalanced_edges: 0,
                degenerate_triangles: 0
            }
        );
    }
    #[test]
    fn sphere_is_closed_outward_and_rebases_without_changing_identity() {
        let field = PlanetConfig {
            terrain: TerrainConfig {
                height_scale: 0.0,
                detail_scale: 0.0,
                cave_density: 0.0,
                ..PILOT_PLANET.terrain
            },
            ..PILOT_PLANET
        }
        .validate(42)
        .unwrap();
        let volume = sample_shell(
            &field,
            ShellConfig {
                face_quads: 8,
                radial_cells: 8,
            },
        )
        .unwrap();
        for mesh in [
            contour_shell(&volume).unwrap(),
            planet_overview(&field, 8).unwrap(),
        ] {
            closed(&mesh);
            for t in mesh.triangles() {
                let p = t.vertices.map(|i| mesh.positions[i as usize]);
                assert!((p[1] - p[0]).cross(p[2] - p[0]).dot(p[0]) > 0.0);
            }
            for origin in [Vec3::ZERO, Vec3::new(16.0, -8.0, 4.0)] {
                for (local, world) in mesh
                    .relative_positions(origin)
                    .unwrap()
                    .iter()
                    .zip(mesh.positions())
                {
                    // Model-scale f32 rendering origins; no galaxy-scale precision claim.
                    assert!((*local + origin - *world).length() < 0.000002);
                }
            }
        }
    }
    #[test]
    fn default_planet_mesh_has_closed_seams_and_schedule_invariance() {
        let field = PILOT_PLANET.validate(42).unwrap();
        let volume = sample_shell(&field, PILOT_SHELL).unwrap();
        let generate = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| contour_shell(&volume).unwrap())
        };
        let a = generate(1);
        let b = generate(4);
        let topology = a.topology();
        assert_eq!(topology.open_edges, 0);
        assert_eq!(topology.unbalanced_edges, 0);
        assert_eq!(topology.degenerate_triangles, 0);
        // One vertex per cell can join unresolved sheets; record this known limit.
        assert_eq!(topology.nonmanifold_edges, 467);
        assert_eq!(a.positions, b.positions);
        assert_eq!(a.triangles, b.triangles);
        let unique: std::collections::BTreeSet<_> = a
            .triangles
            .iter()
            .map(|triangle| {
                let mut ids = triangle.vertices;
                ids.sort();
                ids
            })
            .collect();
        assert_eq!(unique.len(), a.triangles.len());
        let fingerprint = procgen_core::fingerprint(a.triangles.iter().flat_map(|t| {
            t.vertices
                .map(u64::from)
                .into_iter()
                .chain([t.region.face.index() as u64])
        }));
        assert_eq!(fingerprint, 10_149_191_386_720_561_003);
    }
    #[test]
    fn planar_and_exact_zero_samples_use_consistent_roots() {
        for offset in [0.0, 0.25, 1.0] {
            let samples = std::array::from_fn(|i| Sample {
                position: corner(i),
                density: offset - corner(i).x,
            });
            let vertex = cell_vertex(samples);
            if offset == 1.0 {
                assert!(vertex.is_none());
            } else {
                assert_eq!(vertex.unwrap(), Vec3::new(offset, 0.5, 0.5));
            }
        }
    }
}
