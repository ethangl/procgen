//! Conforming tetrahedral extraction over Cartesian chunks of different sizes.
use crate::{VoxelAddressError, VoxelChunkAddress, VoxelPosition, VoxelVolumeError};
use procgen_core::Vec3;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

pub const MAX_SURFACE_CHUNKS: usize = 256;
pub(crate) const MAX_ACTIVE_CELLS: usize = 262_144;
pub(crate) const MAX_SURFACE_FACES: usize = 1_572_864;
pub(crate) const MAX_SURFACE_VERTICES: usize = 2_097_152;
pub(crate) const MAX_SURFACE_TRIANGLES: usize = 4_194_304;

#[derive(Debug)]
pub enum VoxelSurfaceError {
    Volume(VoxelVolumeError),
    Address(VoxelAddressError),
    Chunks,
    Overlap,
    Capacity,
    Topology,
    Cancelled,
}
impl From<VoxelAddressError> for VoxelSurfaceError {
    fn from(e: VoxelAddressError) -> Self {
        Self::Address(e)
    }
}
impl From<VoxelVolumeError> for VoxelSurfaceError {
    fn from(e: VoxelVolumeError) -> Self {
        Self::Volume(e)
    }
}
impl fmt::Display for VoxelSurfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Volume(e) => e.fmt(f),
            Self::Address(e) => e.fmt(f),
            Self::Chunks => write!(f, "surface needs 1..={MAX_SURFACE_CHUNKS} chunks"),
            Self::Overlap => {
                f.write_str("surface chunks must be distinct non-overlapping octree leaves")
            }
            Self::Capacity => {
                f.write_str("surface exceeds the bounded cell, face, vertex, or triangle capacity")
            }
            Self::Topology => {
                f.write_str("surface has an invalid interior edge, vertex link, or triangle")
            }
            Self::Cancelled => f.write_str("surface generation was cancelled"),
        }
    }
}
impl Error for VoxelSurfaceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Volume(e) => Some(e),
            Self::Address(e) => Some(e),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoxelTriangle {
    pub vertices: [u32; 3],
    pub chunk: VoxelChunkAddress,
}

pub struct VoxelSurface {
    pub(crate) origin_m: VoxelPosition,
    pub(crate) positions: Vec<Vec3>,
    pub(crate) triangles: Vec<VoxelTriangle>,
    pub(crate) boundary: BTreeSet<[u32; 2]>,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct VoxelSurfaceTopology {
    pub boundary_edges: usize,
    pub invalid_edges: usize,
    pub invalid_vertices: usize,
    pub degenerate_triangles: usize,
}
impl VoxelSurface {
    pub fn origin_m(&self) -> VoxelPosition {
        self.origin_m
    }
    /// Positions in meters relative to origin_m.
    pub fn positions(&self) -> &[Vec3] {
        &self.positions
    }
    pub fn triangles(&self) -> &[VoxelTriangle] {
        &self.triangles
    }
    /// Geometry vectors and boundary keys; tree-node and allocator overhead
    /// are not included.
    pub fn payload_bytes(&self) -> usize {
        self.positions.capacity() * std::mem::size_of::<Vec3>()
            + self.triangles.capacity() * std::mem::size_of::<VoxelTriangle>()
            + self.boundary.len() * std::mem::size_of::<[u32; 2]>()
    }
    pub fn validate(&self) -> Result<(), VoxelSurfaceError> {
        if self.positions.iter().any(|p| !p.is_finite())
            || self.triangles.iter().any(|t| {
                t.vertices
                    .iter()
                    .any(|&v| v as usize >= self.positions.len())
            })
        {
            return Err(VoxelSurfaceError::Topology);
        }
        let t = self.topology();
        if t.invalid_edges != 0 || t.invalid_vertices != 0 || t.degenerate_triangles != 0 {
            return Err(VoxelSurfaceError::Topology);
        }
        Ok(())
    }
    pub fn topology(&self) -> VoxelSurfaceTopology {
        let mut edges = BTreeMap::<[u32; 2], (usize, i32)>::new();
        let mut degenerate_triangles = 0;
        for t in &self.triangles {
            let [a, b, c] = t.vertices.map(|i| self.positions[i as usize]);
            if (b - a).cross(c - a).length_squared() == 0.0 {
                degenerate_triangles += 1;
            }
            for i in 0..3 {
                let a = t.vertices[i];
                let b = t.vertices[(i + 1) % 3];
                let entry = edges.entry([a.min(b), a.max(b)]).or_default();
                entry.0 += 1;
                entry.1 += if a < b { 1 } else { -1 };
            }
        }
        let open: BTreeSet<_> = edges
            .iter()
            .filter(|(_, (n, _))| *n == 1)
            .map(|(&e, _)| e)
            .collect();
        let boundary_vertices: BTreeSet<_> = self.boundary.iter().flatten().copied().collect();
        VoxelSurfaceTopology {
            boundary_edges: open.len(),
            invalid_edges: open.symmetric_difference(&self.boundary).count()
                + edges
                    .values()
                    .filter(|&&(n, b)| n > 2 || (n == 2 && b != 0))
                    .count(),
            invalid_vertices: crate::mesh::invalid_vertex_links(
                self.triangles.iter().map(|t| t.vertices),
            )
            .difference(&boundary_vertices)
            .count(),
            degenerate_triangles,
        }
    }
}
