//! Bounded CPU source and region mesh reduction for the streaming experiment.
use rayon::prelude::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    mem::size_of,
    sync::atomic::{AtomicBool, Ordering},
};

use procgen_core::Vec3;
use procgen_cubesphere::CubeFace;

use crate::{
    PlanetError, PlanetField, RegionAddress, ShellConfig, SurfaceMesh, SurfaceTriangle,
    contour::{VertexAddress, contour_source},
    sample_shell,
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
    pub(crate) field: PlanetField,
    pub(crate) mesh: SurfaceMesh,
    addresses: Vec<VertexAddress>,
    normals: Vec<Vec3>,
    regions: [Vec<usize>; 6],
}
impl DetailSource {
    pub fn allocated_bytes(&self) -> usize {
        self.mesh.positions.capacity() * size_of::<Vec3>()
            + self.mesh.triangles.capacity() * size_of::<SurfaceTriangle>()
            + self.addresses.capacity() * size_of::<VertexAddress>()
            + self.normals.capacity() * size_of::<Vec3>()
            + self
                .regions
                .iter()
                .map(|r| r.capacity() * size_of::<usize>())
                .sum::<usize>()
    }
    pub(crate) fn region_work_bytes(&self, face: CubeFace) -> usize {
        self.mesh.triangles.len() * 256
            + self.regions[face.index()].len() * 512
            + self.addresses.len() * 256
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
        field: field.clone(),
        mesh,
        addresses,
        normals,
        regions,
    }))
}

pub struct RegionMesh {
    pub(crate) surface: SurfaceMesh,
    /// Cell and contour-part representatives identify shared borders after remapping.
    pub(crate) identities: Vec<VertexAddress>,
    normals: Vec<Vec3>,
    density_normals: Vec<Vec3>,
}
impl RegionMesh {
    pub fn surface(&self) -> &SurfaceMesh {
        &self.surface
    }
    pub fn normals(&self) -> &[Vec3] {
        &self.normals
    }
    pub fn density_normals(&self) -> &[Vec3] {
        &self.density_normals
    }
    pub fn allocated_bytes(&self) -> usize {
        self.surface.positions.capacity() * size_of::<Vec3>()
            + self.surface.triangles.capacity() * size_of::<SurfaceTriangle>()
            + self.identities.capacity() * size_of::<VertexAddress>()
            + self.normals.capacity() * size_of::<Vec3>()
            + self.density_normals.capacity() * size_of::<Vec3>()
    }
}

// Keep a full four-cell collar (the largest reduction block) on every face.
// Its identities, positions, and triangles do not change with the neighbor LOD.
fn boundary(address: VertexAddress) -> bool {
    let n = STREAM_SHELL.face_quads;
    let quad = address.cell / STREAM_SHELL.radial_cells;
    let x = quad % n;
    let y = (quad / n) % n;
    let collar = DetailLevel::Coarse.span();
    x < collar || y < collar || x >= n - collar || y >= n - collar
}
fn representative(address: VertexAddress, level: DetailLevel) -> VertexAddress {
    let n = STREAM_SHELL.face_quads;
    let layers = STREAM_SHELL.radial_cells;
    let radial = address.cell % layers;
    let quad = address.cell / layers;
    let x = quad % n;
    let y = (quad / n) % n;
    let face = quad / (n * n);
    let span = if boundary(address) { 1 } else { level.span() };
    VertexAddress {
        cell: ((face * n * n + (y / span * span) * n + x / span * span) * layers) + radial,
        part: address.part,
    }
}

#[derive(Default)]
struct Cluster {
    sum: Vec3,
    count: usize,
}
#[derive(Clone, Copy)]
struct ReducedVertex {
    identity: VertexAddress,
    position: Vec3,
}

fn reduce_vertices(
    source: &DetailSource,
    face: CubeFace,
    level: DetailLevel,
    cancel: &AtomicBool,
) -> Option<Vec<ReducedVertex>> {
    let fine: Vec<_> = source
        .addresses
        .iter()
        .zip(&source.mesh.positions)
        .map(|(&identity, &position)| ReducedVertex { identity, position })
        .collect();
    if level == DetailLevel::Fine {
        return Some(fine);
    }
    let medium = collapse_clusters(source, face, DetailLevel::Medium, &fine, cancel)?;
    if level == DetailLevel::Medium {
        return Some(medium);
    }
    collapse_clusters(source, face, DetailLevel::Coarse, &medium, cancel)
}

/// Coarse clusters contain complete medium clusters. Rejected collapses retain
/// the preceding level, so a coarser product cannot restore removed triangles.
fn collapse_clusters(
    source: &DetailSource,
    face: CubeFace,
    level: DetailLevel,
    previous: &[ReducedVertex],
    cancel: &AtomicBool,
) -> Option<Vec<ReducedVertex>> {
    let cells_per_face = STREAM_SHELL.face_quads.pow(2) * STREAM_SHELL.radial_cells;
    let mut groups: BTreeMap<VertexAddress, Cluster> = BTreeMap::new();
    // Weight by original vertices at both levels so partition changes do not
    // alter the weighting of a surviving medium cluster.
    for (&address, vertex) in source.addresses.iter().zip(previous) {
        let position = vertex.position;
        if address.cell / cells_per_face == face.index() {
            let cluster = groups.entry(representative(address, level)).or_default();
            cluster.sum = cluster.sum + position;
            cluster.count += 1;
        }
    }
    let mut rejected = BTreeSet::new();
    // Component numbers are local to a cell, not correspondence across cells.
    // Keep every reduction block containing a split cell at its preceding level.
    let protected: BTreeSet<_> = source
        .addresses
        .iter()
        .filter(|a| a.part != 0)
        .map(|a| representative(*a, level).cell)
        .collect();
    rejected.extend(
        groups
            .keys()
            .filter(|k| protected.contains(&k.cell))
            .copied(),
    );
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let vertices: Vec<_> = source
            .addresses
            .iter()
            .zip(previous)
            .map(|(&address, &vertex)| {
                if address.cell / cells_per_face != face.index() {
                    return vertex;
                }
                let identity = representative(address, level);
                if rejected.contains(&identity) {
                    return vertex;
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
            let prior = original.map(|i| previous[i].position);
            let prior_normal = (prior[1] - prior[0]).cross(prior[2] - prior[0]);
            let new_normal =
                (next[1].position - next[0].position).cross(next[2].position - next[0].position);
            if normal.dot(new_normal) <= 0.0 || prior_normal.dot(new_normal) <= 0.0 {
                for i in original {
                    let address = source.addresses[i];
                    let group = representative(address, level);
                    if address.cell / cells_per_face == face.index() && groups[&group].count > 1 {
                        rejected.insert(group);
                    }
                }
            }
        }
        // Check the complete closed mesh so region borders are not mistaken for
        // holes. Reject the participating clusters around any invalid link.
        let mut ids = BTreeMap::new();
        let remap: Vec<_> = vertices
            .iter()
            .map(|v| {
                let next = ids.len() as u32;
                *ids.entry(v.identity).or_insert(next)
            })
            .collect();
        let mapped: Vec<_> = source
            .mesh
            .triangles
            .iter()
            .map(|t| t.vertices.map(|i| remap[i as usize]))
            .collect();
        let bad = crate::mesh::invalid_vertex_links(
            mapped
                .iter()
                .copied()
                .filter(|t| t[0] != t[1] && t[1] != t[2] && t[2] != t[0]),
        );
        for (triangle, mapped) in source.mesh.triangles.iter().zip(mapped) {
            if mapped.iter().any(|v| bad.contains(v)) {
                for i in triangle.vertices {
                    let address = source.addresses[i as usize];
                    if address.cell / cells_per_face == face.index() {
                        let group = representative(address, level);
                        if groups[&group].count > 1 {
                            rejected.insert(group);
                        }
                    }
                }
            }
        }
        // Each nonterminal pass permanently rejects at least one cluster.
        // Restoring a cluster can affect its neighbors, so recheck until stable.
        if rejected.len() == before {
            debug_assert!(
                bad.is_empty(),
                "rejection must recover the preceding manifold mesh"
            );
            return Some(vertices);
        }
    }
}

/// Cluster only tangentially inside each face; radial sheets remain separate.
/// Medium reduces the fine source; coarse reduces medium. Clusters that reverse
/// or flatten a surviving triangle retain their preceding-level vertices.
/// Triangles collapsed to fewer than three identities are removed. This is a
/// render reduction; invalid closed vertex neighborhoods reject their clusters.
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
    let sample = |p: &Vec3| source.field.density_normal(*p);
    let density_normals = if surface.positions.len() >= 1024 {
        surface.positions.par_iter().map(sample).collect()
    } else {
        surface.positions.iter().map(sample).collect()
    };
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    Some(RegionMesh {
        density_normals,
        surface,
        identities,
        normals,
    })
}

pub(crate) fn join_regions<'a>(regions: impl IntoIterator<Item = &'a RegionMesh>) -> SurfaceMesh {
    let mut positions = Vec::new();
    let mut triangles = Vec::new();
    let mut ids = BTreeMap::new();
    for product in regions {
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
    SurfaceMesh {
        positions,
        triangles,
    }
}

#[cfg(test)]
pub(crate) fn test_source(mesh: SurfaceMesh) -> std::sync::Arc<DetailSource> {
    std::sync::Arc::new(DetailSource {
        field: crate::PILOT_PLANET.validate(42).unwrap(),
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
            let counts = levels.each_ref().map(|p| p.surface.triangles.len());
            assert!(
                counts[0] <= counts[1] && counts[1] <= counts[2],
                "face={face}: coarse, medium, fine triangle counts {counts:?}"
            );
            // No medium vertex can split into several coarse vertices. This
            // checks the local hierarchy as well as the whole-face counts.
            let medium =
                reduce_vertices(&source, CubeFace::ALL[face], DetailLevel::Medium, &cancel)
                    .unwrap();
            let coarse =
                reduce_vertices(&source, CubeFace::ALL[face], DetailLevel::Coarse, &cancel)
                    .unwrap();
            let mut parents = BTreeMap::new();
            for (m, c) in medium.iter().zip(&coarse) {
                if let Some(parent) = parents.insert(m.identity, c.identity) {
                    assert_eq!(parent, c.identity, "medium vertex split at coarse LOD");
                }
            }
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
                for (((&identity, &p), &normal), &density_normal) in product
                    .identities
                    .iter()
                    .zip(product.surface.positions())
                    .zip(product.normals())
                    .zip(product.density_normals())
                {
                    assert!(density_normal.is_finite());
                    let length = density_normal.length();
                    assert!(length == 0.0 || (length - 1.0).abs() < 0.001);
                    if boundary(identity)
                        && let Some(previous) =
                            borders.insert(identity, (p, normal, density_normal))
                    {
                        assert_eq!((p, normal, density_normal), previous);
                    }
                }
            }
        }
        for offset in 0..3 {
            let mixed = join_regions(
                products
                    .iter()
                    .enumerate()
                    .map(|(face, levels)| &levels[(face + offset) % 3]),
            );
            mixed.validate().unwrap();
            let topology = mixed.topology();
            assert_eq!(topology.nonmanifold_edges, 0, "{topology:?}");
            assert_eq!(topology.nonmanifold_vertices, 0, "{topology:?}");
            assert_eq!(topology.degenerate_triangles, 0, "{topology:?}");
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
