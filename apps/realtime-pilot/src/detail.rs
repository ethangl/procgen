//! Bounded CPU source and region mesh reduction for the streaming experiment.
use std::{
    collections::{BTreeMap, BTreeSet},
    mem::size_of,
    sync::atomic::{AtomicBool, Ordering},
};

use procgen_core::Vec3;
use procgen_cubesphere::CubeFace;

use crate::{
    PlanetError, PlanetField, RegionAddress, ShellConfig, SurfaceMesh, SurfaceTriangle,
    contour::contour_source, sample_shell,
};

/// Fixed density resolution for this slice. Only mesh detail streams; the CPU
/// contour source remains resident. Arbitrary-size density paging is not implied.
pub const STREAM_SHELL: ShellConfig = ShellConfig {
    face_quads: 64,
    radial_cells: 16,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DetailLevel {
    Coarse,
    Medium,
    Fine,
}
impl DetailLevel {
    pub const ALL: [Self; 3] = [Self::Coarse, Self::Medium, Self::Fine];
    fn span(self) -> usize {
        match self {
            Self::Coarse => 4,
            Self::Medium => 2,
            Self::Fine => 1,
        }
    }
}

pub struct DetailSource {
    pub(crate) radius: f32,
    pub(crate) mesh: SurfaceMesh,
    addresses: Vec<usize>,
    normals: Vec<Vec3>,
    regions: [Vec<usize>; 6],
}
impl DetailSource {
    pub fn allocated_bytes(&self) -> usize {
        self.mesh.positions.capacity() * size_of::<Vec3>()
            + self.mesh.triangles.capacity() * size_of::<SurfaceTriangle>()
            + self.addresses.capacity() * size_of::<usize>()
            + self.normals.capacity() * size_of::<Vec3>()
            + self
                .regions
                .iter()
                .map(|r| r.capacity() * size_of::<usize>())
                .sum::<usize>()
    }
    pub(crate) fn region_work_bytes(&self, face: CubeFace) -> usize {
        self.regions[face.index()].len() * 512 + self.addresses.len() * 128
    }
    pub fn triangle_count(&self) -> usize {
        self.mesh.triangles.len()
    }
}

/// Cancellation is checked at pipeline boundaries. The caller keeps its working
/// reservation until this function returns, including when a request is cancelled.
pub fn prepare_detail(
    field: &PlanetField,
    cancel: &AtomicBool,
) -> Result<Option<DetailSource>, PlanetError> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let volume = sample_shell(field, STREAM_SHELL)?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let (mesh, addresses) = contour_source(&volume)?;
    drop(volume);
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let mut regions: [Vec<usize>; 6] = std::array::from_fn(|_| Vec::new());
    for (id, triangle) in mesh.triangles.iter().enumerate() {
        regions[triangle.region.face.index()].push(id);
    }
    let normals = mesh.vertex_normals();
    Ok(Some(DetailSource {
        radius: field.config().radius,
        mesh,
        addresses,
        normals,
        regions,
    }))
}

pub struct RegionMesh {
    pub(crate) surface: SurfaceMesh,
    /// Source-cell representatives survive remapping and identify shared borders.
    pub(crate) identities: Vec<usize>,
    normals: Vec<Vec3>,
}
impl RegionMesh {
    pub fn surface(&self) -> &SurfaceMesh {
        &self.surface
    }
    pub fn normals(&self) -> &[Vec3] {
        &self.normals
    }
    pub fn allocated_bytes(&self) -> usize {
        self.surface.positions.capacity() * size_of::<Vec3>()
            + self.surface.triangles.capacity() * size_of::<SurfaceTriangle>()
            + self.identities.capacity() * size_of::<usize>()
            + self.normals.capacity() * size_of::<Vec3>()
    }
}

// Keep a full four-cell collar (the largest reduction block) on every face.
// Its identities, positions, and triangles do not change with the neighbor LOD.
fn boundary(address: usize) -> bool {
    let n = STREAM_SHELL.face_quads;
    let quad = address / STREAM_SHELL.radial_cells;
    let x = quad % n;
    let y = (quad / n) % n;
    let collar = DetailLevel::Coarse.span();
    x < collar || y < collar || x >= n - collar || y >= n - collar
}
fn representative(address: usize, level: DetailLevel) -> usize {
    let n = STREAM_SHELL.face_quads;
    let layers = STREAM_SHELL.radial_cells;
    let radial = address % layers;
    let quad = address / layers;
    let x = quad % n;
    let y = (quad / n) % n;
    let face = quad / (n * n);
    let span = if boundary(address) { 1 } else { level.span() };
    ((face * n * n + (y / span * span) * n + x / span * span) * layers) + radial
}

#[derive(Default)]
struct Cluster {
    sum: Vec3,
    count: usize,
}
struct ReducedVertex {
    identity: usize,
    position: Vec3,
}

fn reduce_vertices(
    source: &DetailSource,
    face: CubeFace,
    level: DetailLevel,
    cancel: &AtomicBool,
) -> Option<Vec<ReducedVertex>> {
    let cells_per_face = STREAM_SHELL.face_quads.pow(2) * STREAM_SHELL.radial_cells;
    let mut groups: BTreeMap<usize, Cluster> = BTreeMap::new();
    for (&address, &position) in source.addresses.iter().zip(&source.mesh.positions) {
        if address / cells_per_face == face.index() {
            let cluster = groups.entry(representative(address, level)).or_default();
            cluster.sum = cluster.sum + position;
            cluster.count += 1;
        }
    }
    let mut rejected = BTreeSet::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let vertices: Vec<_> = source
            .addresses
            .iter()
            .zip(&source.mesh.positions)
            .map(|(&address, &position)| {
                if address / cells_per_face != face.index() {
                    return ReducedVertex {
                        identity: address,
                        position,
                    };
                }
                let identity = representative(address, level);
                if rejected.contains(&identity) {
                    return ReducedVertex {
                        identity: address,
                        position,
                    };
                }
                let cluster = &groups[&identity];
                ReducedVertex {
                    identity,
                    position: cluster.sum * (cluster.count as f32).recip(),
                }
            })
            .collect();
        let before = rejected.len();
        for (offset, &id) in source.regions[face.index()].iter().enumerate() {
            if offset % 1024 == 0 && cancel.load(Ordering::Relaxed) {
                return None;
            }
            let original = source.mesh.triangles[id].vertices.map(|i| i as usize);
            let next = original.map(|i| &vertices[i]);
            if next[0].identity == next[1].identity
                || next[1].identity == next[2].identity
                || next[0].identity == next[2].identity
            {
                continue;
            }
            let p = original.map(|i| source.mesh.positions[i]);
            let normal = (p[1] - p[0]).cross(p[2] - p[0]);
            let new_normal =
                (next[1].position - next[0].position).cross(next[2].position - next[0].position);
            if normal.dot(new_normal) <= 0.0 {
                for i in original {
                    let address = source.addresses[i];
                    let group = representative(address, level);
                    if address / cells_per_face == face.index() && groups[&group].count > 1 {
                        rejected.insert(group);
                    }
                }
            }
        }
        // Each nonterminal pass permanently rejects at least one cluster.
        // Restoring a cluster can affect its neighbors, so recheck until stable.
        if rejected.len() == before {
            return Some(vertices);
        }
    }
}

/// Cluster only tangentially inside each face; radial sheets remain separate.
/// Clusters that reverse or flatten a surviving triangle retain their fine
/// vertices. Triangles collapsed to fewer than three identities are removed. This is a
/// render reduction, not a new density sampling backend or a manifold repair.
pub fn build_region(
    source: &DetailSource,
    face: CubeFace,
    level: DetailLevel,
    cancel: &AtomicBool,
) -> Option<RegionMesh> {
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let reduced = reduce_vertices(source, face, level, cancel)?;
    let mut positions = Vec::new();
    let mut identities = Vec::new();
    let mut local_ids = BTreeMap::new();
    let mut triangles = Vec::new();
    for (offset, &id) in source.regions[face.index()].iter().enumerate() {
        if offset % 1024 == 0 && cancel.load(Ordering::Relaxed) {
            return None;
        }
        let triangle = source.mesh.triangles[id];
        let vertices = triangle.vertices.map(|id| {
            let id = id as usize;
            let reduced = &reduced[id];
            let identity = reduced.identity;
            *local_ids.entry(identity).or_insert_with(|| {
                let position = reduced.position;
                let local = positions.len() as u32;
                positions.push(position);
                identities.push(identity);
                local
            })
        });
        if vertices[0] != vertices[1] && vertices[1] != vertices[2] && vertices[0] != vertices[2] {
            triangles.push(SurfaceTriangle {
                vertices,
                region: RegionAddress { face },
            });
        }
    }
    let surface = SurfaceMesh {
        positions,
        triangles,
    };
    let mut normals = surface.vertex_normals();
    for (&identity, normal) in identities.iter().zip(&mut normals) {
        if boundary(identity) {
            *normal = source.normals[source
                .addresses
                .binary_search(&identity)
                .expect("fixed border cell")];
        }
    }
    Some(RegionMesh {
        surface,
        identities,
        normals,
    })
}

#[cfg(test)]
pub(crate) fn test_source(mesh: SurfaceMesh) -> std::sync::Arc<DetailSource> {
    std::sync::Arc::new(DetailSource {
        radius: 4.0,
        mesh,
        addresses: Vec::new(),
        normals: Vec::new(),
        regions: std::array::from_fn(|_| Vec::new()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PILOT_PLANET;
    #[test]
    fn all_mixed_levels_weld_at_faces_and_corners_without_open_edges() {
        let cancel = AtomicBool::new(false);
        let source = prepare_detail(&PILOT_PLANET.validate(42).unwrap(), &cancel)
            .unwrap()
            .unwrap();
        let products: Vec<_> = CubeFace::ALL
            .into_iter()
            .map(|face| {
                DetailLevel::ALL.map(|level| build_region(&source, face, level, &cancel).unwrap())
            })
            .collect();
        for (face, levels) in products.iter().enumerate() {
            for (level, product) in DetailLevel::ALL.into_iter().zip(levels) {
                let reduced =
                    reduce_vertices(&source, CubeFace::ALL[face], level, &cancel).unwrap();
                let positions: BTreeMap<_, _> = product
                    .identities
                    .iter()
                    .copied()
                    .zip(product.surface.positions().iter().copied())
                    .collect();
                for &id in &source.regions[face] {
                    let original = source.mesh.triangles[id].vertices.map(|v| v as usize);
                    let ids = original.map(|v| reduced[v].identity);
                    if ids[0] == ids[1] || ids[1] == ids[2] || ids[0] == ids[2] {
                        continue;
                    }
                    let a = original.map(|v| source.mesh.positions[v]);
                    let b = ids.map(|id| positions[&id]);
                    assert!(
                        (a[1] - a[0])
                            .cross(a[2] - a[0])
                            .dot((b[1] - b[0]).cross(b[2] - b[0]))
                            > 0.0,
                        "face={face} level={level:?}"
                    );
                }
            }
        }
        // Every neighboring level pairing appears; all 3^6 arrangements have the
        // same fixed border, checked independently against every product below.
        let mut borders = BTreeMap::new();
        for levels in &products {
            for product in levels {
                for ((&identity, &p), &normal) in product
                    .identities
                    .iter()
                    .zip(product.surface.positions())
                    .zip(product.normals())
                {
                    if boundary(identity)
                        && let Some(previous) = borders.insert(identity, (p, normal))
                    {
                        assert_eq!((p, normal), previous);
                    }
                }
            }
        }
        for offset in 0..3 {
            let mut positions = Vec::new();
            let mut triangles = Vec::new();
            let mut ids = BTreeMap::new();
            for (face, levels) in products.iter().enumerate() {
                let product = &levels[(face + offset) % 3];
                let remap: Vec<_> = product
                    .identities
                    .iter()
                    .zip(product.surface.positions())
                    .map(|(&identity, &p)| {
                        *ids.entry(identity).or_insert_with(|| {
                            let i = positions.len() as u32;
                            positions.push(p);
                            i
                        })
                    })
                    .collect();
                triangles.extend(product.surface.triangles().iter().map(|t| SurfaceTriangle {
                    vertices: t.vertices.map(|i| remap[i as usize]),
                    region: t.region,
                }));
            }
            let mixed = SurfaceMesh {
                positions,
                triangles,
            };
            mixed.validate().unwrap();
            let topology = mixed.topology();
            assert_eq!(topology.open_edges, 0, "{topology:?}");
            assert_eq!(topology.unbalanced_edges, 0, "{topology:?}");
        }
        assert!(
            products
                .iter()
                .map(|p| p[0].surface.triangles.len())
                .sum::<usize>()
                < source.triangle_count()
        );
        cancel.store(true, Ordering::Relaxed);
        assert!(build_region(&source, CubeFace::PositiveX, DetailLevel::Fine, &cancel).is_none());
        assert!(
            prepare_detail(&PILOT_PLANET.validate(42).unwrap(), &cancel)
                .unwrap()
                .is_none()
        );
    }
}
