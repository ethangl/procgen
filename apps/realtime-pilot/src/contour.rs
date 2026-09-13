use std::collections::BTreeMap;

use procgen_core::Vec3;
use rayon::prelude::*;

use crate::{
    PlanetError, PlanetField, RegionAddress, ShellVolume, SurfaceMesh, SurfaceTriangle,
    contour_cells::{EDGES, interpolate_positions, sheets},
    shell::face_grid,
};

pub const OVERVIEW_FACE_QUADS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct VertexAddress {
    pub cell: usize,
    pub part: usize,
}

struct CellVertex {
    address: VertexAddress,
    region: RegionAddress,
    samples: [usize; 8],
    position: Vec3,
    center: Vec3,
    crossings: Vec<[usize; 2]>,
}

/// Contour the sampled density with separate vertices for each cell-boundary
/// cycle. Shared bilinear face decisions keep adjacent sheets connected.
pub fn contour_shell(volume: &ShellVolume) -> Result<SurfaceMesh, PlanetError> {
    Ok(contour_source(volume)?.0)
}

pub(crate) fn contour_source(
    volume: &ShellVolume,
) -> Result<(SurfaceMesh, Vec<VertexAddress>), PlanetError> {
    volume.validate()?;
    let cells: Vec<_> = volume
        .cells()
        .enumerate()
        .map(|(address, (region, ids))| {
            let samples = ids.map(|i| volume.samples[i]);
            sheets(samples, ids)
                .into_iter()
                .enumerate()
                .map(|(sheet, part)| {
                    let crossings = part
                        .edges
                        .iter()
                        .map(|&e| {
                            let (a, b) = EDGES[e];
                            let mut key = [ids[a], ids[b]];
                            key.sort();
                            key
                        })
                        .collect();
                    CellVertex {
                        address: VertexAddress {
                            cell: address,
                            part: sheet,
                        },
                        region,
                        samples: ids,
                        position: part.position,
                        center: interpolate_positions(samples, Vec3::new(0.5, 0.5, 0.5)),
                        crossings,
                    }
                })
                .collect::<Vec<_>>()
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
            triangles.push(ContourTriangle {
                triangle: SurfaceTriangle {
                    vertices: [ring[0] as u32, ring[i] as u32, ring[i + 1] as u32],
                    region,
                },
                crossing: edge,
            });
        }
    }
    let (mesh, addresses) = triangulate_sheets(volume, &cells, triangles)?;
    mesh.validate()?;
    if !mesh.topology().is_closed_manifold() {
        return Err(PlanetError::Topology);
    }
    Ok((mesh, addresses))
}

struct ContourTriangle {
    triangle: SurfaceTriangle,
    crossing: [usize; 2],
}

// Two cell sheets can meet twice across an ambiguous face. Those are distinct
// arcs, even when their dual edges have the same endpoints. Insert one shared
// face vertex per arc before assembling an indexed triangle mesh.
fn triangulate_sheets(
    volume: &ShellVolume,
    cells: &[CellVertex],
    input: Vec<ContourTriangle>,
) -> Result<(SurfaceMesh, Vec<VertexAddress>), PlanetError> {
    let mut incidence = BTreeMap::<[u32; 2], usize>::new();
    for t in &input {
        let v = t.triangle.vertices;
        for (a, b) in [(v[0], v[1]), (v[1], v[2]), (v[2], v[0])] {
            *incidence.entry([a.min(b), a.max(b)]).or_default() += 1;
        }
    }
    let mut positions: Vec<_> = cells.iter().map(|c| c.position).collect();
    let mut addresses: Vec<_> = cells.iter().map(|c| c.address).collect();
    let mut next_part = BTreeMap::<usize, usize>::new();
    for address in &addresses {
        next_part
            .entry(address.cell)
            .and_modify(|n| *n = (*n).max(address.part + 1))
            .or_insert(address.part + 1);
    }
    let mut arcs = BTreeMap::new();
    let mut triangles = Vec::new();
    for t in input {
        let v = t.triangle.vertices;
        let mut parts = vec![v];
        for (a, b) in [(v[0], v[1]), (v[1], v[2]), (v[2], v[0])] {
            if incidence[&[a.min(b), a.max(b)]] <= 2 {
                continue;
            }
            let left = &cells[a as usize];
            let right = &cells[b as usize];
            let face: Vec<_> = left
                .samples
                .iter()
                .copied()
                .filter(|i| right.samples.contains(i))
                .collect();
            if face.len() != 4 {
                return Err(PlanetError::Topology);
            }
            let index = left
                .crossings
                .iter()
                .position(|e| *e == t.crossing)
                .ok_or(PlanetError::Topology)?;
            let n = left.crossings.len();
            let other = [
                left.crossings[(index + 1) % n],
                left.crossings[(index + n - 1) % n],
            ]
            .into_iter()
            .find(|edge| edge.iter().all(|i| face.contains(i)))
            .ok_or(PlanetError::Topology)?;
            let mut key = [t.crossing, other];
            key.sort();
            let midpoint = *arcs.entry(key).or_insert_with(|| {
                let root = |edge: [usize; 2]| {
                    let [a, b] = edge.map(|i| volume.samples[i]);
                    a.position + (b.position - a.position) * (a.density / (a.density - b.density))
                };
                let id = positions.len() as u32;
                positions.push((root(key[0]) + root(key[1])) * 0.5);
                let cell = left.address.cell.min(right.address.cell);
                let part = next_part.get_mut(&cell).unwrap();
                addresses.push(VertexAddress { cell, part: *part });
                *part += 1;
                id
            });
            let i = parts
                .iter()
                .position(|p| p.contains(&a) && p.contains(&b))
                .ok_or(PlanetError::Topology)?;
            let p = parts.remove(i);
            let j = (0..3)
                .find(|&j| p[j] == a && p[(j + 1) % 3] == b)
                .ok_or(PlanetError::Topology)?;
            let c = p[(j + 2) % 3];
            parts.push([a, midpoint, c]);
            parts.push([midpoint, b, c]);
        }
        triangles.extend(parts.into_iter().map(|vertices| SurfaceTriangle {
            vertices,
            region: t.triangle.region,
        }));
    }
    // Keep canonical address order for stable border lookup and fingerprints.
    let mut order: Vec<_> = (0..addresses.len()).collect();
    order.sort_by_key(|&i| addresses[i]);
    let mut remap = vec![0; order.len()];
    for (new, &old) in order.iter().enumerate() {
        remap[old] = new as u32;
    }
    for triangle in &mut triangles {
        triangle.vertices = triangle.vertices.map(|i| remap[i as usize]);
    }
    Ok((
        SurfaceMesh {
            positions: order.iter().map(|&i| positions[i]).collect(),
            triangles,
        },
        order.iter().map(|&i| addresses[i]).collect(),
    ))
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
    use crate::MeshTopology;
    use crate::{
        PILOT_PLANET, PILOT_SHELL, PlanetConfig, ShellConfig, TerrainConfig, sample_shell,
    };
    fn closed(mesh: &SurfaceMesh) {
        assert_eq!(
            mesh.topology(),
            MeshTopology {
                open_edges: 0,
                nonmanifold_edges: 0,
                nonmanifold_vertices: 0,
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
        assert_eq!(topology.nonmanifold_edges, 0);
        assert_eq!(topology.nonmanifold_vertices, 0);
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
        // Sheet separation and explicit face arcs change canonical triangle identities.
        assert_eq!(fingerprint, 15_074_258_157_278_690_721);
    }
    #[test]
    fn ambiguous_shells_remain_manifold_across_cube_seams() {
        for seed in 0..32 {
            let field = PILOT_PLANET.validate(seed).unwrap();
            let mut volume = sample_shell(
                &field,
                ShellConfig {
                    face_quads: 4,
                    radial_cells: 4,
                },
            )
            .unwrap();
            for (i, sample) in volume.samples.iter_mut().enumerate() {
                if i % 5 != 0 && i % 5 != 4 {
                    let hash = procgen_core::hash_u32(seed as u32, i as u32, 0, 0);
                    sample.density = if hash & 1 == 0 { -1.0 } else { 1.0 }
                        * (0.1 + (hash % 1000) as f32 / 1000.0);
                }
            }
            let mesh = contour_shell(&volume).unwrap();
            closed(&mesh);
        }
    }
}
