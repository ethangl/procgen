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

/// Base density resolution. Fine geometry adds field-guided edge vertices.
/// Canonical collision geometry stays resident; this is not density paging.
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
    /// Canonical refined collision surface. Fine render products use these triangles.
    pub(crate) mesh: SurfaceMesh,
    base_mesh: SurfaceMesh,
    fine_addresses: Vec<VertexAddress>,
    fine_regions: [Vec<usize>; 6],
    base_addresses: Vec<VertexAddress>,
    base_normals: Vec<Vec3>,
    base_regions: [Vec<usize>; 6],
}
impl DetailSource {
    pub fn allocated_bytes(&self) -> usize {
        self.mesh.positions.capacity() * size_of::<Vec3>()
            + self.mesh.triangles.capacity() * size_of::<SurfaceTriangle>()
            + self.base_mesh.positions.capacity() * size_of::<Vec3>()
            + self.base_mesh.triangles.capacity() * size_of::<SurfaceTriangle>()
            + self.fine_addresses.capacity() * size_of::<VertexAddress>()
            + self
                .fine_regions
                .iter()
                .map(|r| r.capacity() * size_of::<usize>())
                .sum::<usize>()
            + self.base_addresses.capacity() * size_of::<VertexAddress>()
            + self.base_normals.capacity() * size_of::<Vec3>()
            + self
                .base_regions
                .iter()
                .map(|r| r.capacity() * size_of::<usize>())
                .sum::<usize>()
    }
    pub(crate) fn region_work_bytes(&self, face: CubeFace) -> usize {
        let reduction = self.base_mesh.triangles.len() * 256
            + self.base_regions[face.index()].len() * 512
            + self.base_addresses.len() * 256;
        let fine = self.fine_regions[face.index()].len() * 512
            + self.mesh.positions.len() * size_of::<ReducedVertex>();
        reduction.max(fine)
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
    let (mesh, base_addresses) = contour_source(&volume)?;
    drop(volume);
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let eligible: Vec<_> = mesh
        .triangles
        .iter()
        .map(|t| {
            t.vertices
                .iter()
                .all(|&v| !boundary(base_addresses[v as usize]))
        })
        .collect();
    let normals = mesh.vertex_normals();
    let Some(refined) =
        crate::refinement::refine_surface(field, &mesh, &eligible, &normals, cancel)
    else {
        return Ok(None);
    };
    refined.mesh.validate()?;
    if !refined.mesh.topology().is_closed_manifold() {
        return Err(PlanetError::Topology);
    }
    let mut next_part = BTreeMap::<usize, usize>::new();
    for address in &base_addresses {
        next_part
            .entry(address.cell)
            .and_modify(|n| *n = (*n).max(address.part + 1))
            .or_insert(address.part + 1);
    }
    let mut fine_addresses = base_addresses.clone();
    for edge in &refined.edges {
        let cell = edge
            .map(|i| base_addresses[i as usize].cell)
            .into_iter()
            .min()
            .unwrap();
        let part = next_part.get_mut(&cell).unwrap();
        fine_addresses.push(VertexAddress { cell, part: *part });
        *part += 1;
    }
    Ok(Some(DetailSource {
        field: field.clone(),
        base_regions: region_triangles(&mesh),
        fine_regions: region_triangles(&refined.mesh),
        mesh: refined.mesh,
        base_mesh: mesh,
        fine_addresses,
        base_addresses,
        base_normals: normals,
    }))
}

fn region_triangles(mesh: &SurfaceMesh) -> [Vec<usize>; 6] {
    let mut regions: [Vec<usize>; 6] = std::array::from_fn(|_| Vec::new());
    for (i, t) in mesh.triangles.iter().enumerate() {
        regions[t.region.face.index()].push(i);
    }
    regions
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
    let base: Vec<_> = source
        .base_addresses
        .iter()
        .zip(&source.base_mesh.positions)
        .map(|(&identity, &position)| ReducedVertex { identity, position })
        .collect();
    debug_assert_ne!(level, DetailLevel::Fine);
    let medium = collapse_clusters(source, face, DetailLevel::Medium, &base, cancel)?;
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
    for (&address, vertex) in source.base_addresses.iter().zip(previous) {
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
        .base_addresses
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
            .base_addresses
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
        for (offset, &id) in source.base_regions[face.index()].iter().enumerate() {
            if offset % 1024 == 0 && cancel.load(Ordering::Relaxed) {
                return None;
            }
            let original = source.base_mesh.triangles[id].vertices.map(|i| i as usize);
            let next = original.map(|i| &vertices[i]);
            if next[0].identity == next[1].identity
                || next[1].identity == next[2].identity
                || next[0].identity == next[2].identity
            {
                continue;
            }
            let p = original.map(|i| source.base_mesh.positions[i]);
            let normal = (p[1] - p[0]).cross(p[2] - p[0]);
            let prior = original.map(|i| previous[i].position);
            let prior_normal = (prior[1] - prior[0]).cross(prior[2] - prior[0]);
            let new_normal =
                (next[1].position - next[0].position).cross(next[2].position - next[0].position);
            if normal.dot(new_normal) <= 0.0 || prior_normal.dot(new_normal) <= 0.0 {
                for i in original {
                    let address = source.base_addresses[i];
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
            .base_mesh
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
        for (triangle, mapped) in source.base_mesh.triangles.iter().zip(mapped) {
            if mapped.iter().any(|v| bad.contains(v)) {
                for i in triangle.vertices {
                    let address = source.base_addresses[i as usize];
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
/// Fine uses the refined collision surface. Medium reduces the base contour;
/// coarse reduces medium. Clusters that reverse
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
    let (mesh, region, reduced) = if level == DetailLevel::Fine {
        (
            &source.mesh,
            &source.fine_regions[face.index()],
            source
                .fine_addresses
                .iter()
                .zip(&source.mesh.positions)
                .map(|(&identity, &position)| ReducedVertex { identity, position })
                .collect(),
        )
    } else {
        (
            &source.base_mesh,
            &source.base_regions[face.index()],
            reduce_vertices(source, face, level, cancel)?,
        )
    };
    let mut positions = Vec::new();
    let mut identities = Vec::new();
    let mut local_ids = BTreeMap::new();
    let mut triangles = Vec::new();
    for (offset, &id) in region.iter().enumerate() {
        if offset % 1024 == 0 && cancel.load(Ordering::Relaxed) {
            return None;
        }
        let triangle = mesh.triangles[id];
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
            *normal = source.base_normals[source
                .base_addresses
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
        base_mesh: SurfaceMesh {
            positions: Vec::new(),
            triangles: Vec::new(),
        },
        fine_addresses: Vec::new(),
        fine_regions: std::array::from_fn(|_| Vec::new()),
        base_addresses: Vec::new(),
        base_normals: Vec::new(),
        base_regions: std::array::from_fn(|_| Vec::new()),
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
            let fine = &levels[2].surface;
            assert_eq!(fine.triangles.len(), source.fine_regions[face].len());
            assert!(fine.triangles.len() > source.base_regions[face].len() * 2);
            for (render, &id) in fine.triangles.iter().zip(&source.fine_regions[face]) {
                assert_eq!(
                    render.vertices.map(|i| fine.positions[i as usize]),
                    source.mesh.triangles[id]
                        .vertices
                        .map(|i| source.mesh.positions[i as usize]),
                    "fine rendering and collision must use the same triangles"
                );
            }

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
            for (level, product) in DetailLevel::ALL.into_iter().zip(levels).take(2) {
                let reduced =
                    reduce_vertices(&source, CubeFace::ALL[face], level, &cancel).unwrap();
                let positions: BTreeMap<_, _> = product
                    .identities
                    .iter()
                    .copied()
                    .zip(product.surface.positions().iter().copied())
                    .collect();
                for &id in &source.base_regions[face] {
                    let original = source.base_mesh.triangles[id].vertices.map(|v| v as usize);
                    let ids = original.map(|v| reduced[v].identity);
                    if ids[0] == ids[1] || ids[1] == ids[2] || ids[0] == ids[2] {
                        continue;
                    }
                    let a = original.map(|v| source.base_mesh.positions[v]);
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
