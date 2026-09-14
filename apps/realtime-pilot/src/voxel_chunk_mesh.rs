//! Canonical uniform-chunk extraction. Ordering matches deterministic GPU scans.
use std::{error::Error, fmt};

use bytemuck::{Pod, Zeroable};
use procgen_core::Vec3;
use rayon::prelude::*;

use crate::voxel_mesh_topology::{
    self as topology, CELL_COUNT, CELLS, NODES, NONE, VOXEL_MESH_VERTEX_SLOTS,
};
use crate::{MeterPosition, VoxelChunkAddress, VoxelPosition, VoxelVolume, VoxelVolumeError};

/// GPU vertex layout: integer meters relative to the chunk, then a local offset.
/// Keeping the two separate preserves short edges at large planet coordinates.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct VoxelMeshVertex {
    pub anchor_m: [i32; 4],
    pub offset_m: [f32; 4],
    /// Outward density gradient, interpolated before normalization; w is unused.
    /// Zero requests a geometric face normal (critical points and transitions).
    pub normal: [f32; 4],
}
impl VoxelMeshVertex {
    pub fn position(self, address: VoxelChunkAddress) -> MeterPosition {
        let origin = address.origin();
        MeterPosition::new(
            VoxelPosition {
                x_m: origin.x_m + self.anchor_m[0],
                y_m: origin.y_m + self.anchor_m[1],
                z_m: origin.z_m + self.anchor_m[2],
            },
            Vec3::new(self.offset_m[0], self.offset_m[1], self.offset_m[2]),
        )
    }
}

#[derive(Debug)]
pub enum VoxelMeshError {
    Volume(VoxelVolumeError),
    Vertex,
    Indices,
    Triangle,
    Capacity,
    TransitionPlan,
    TransitionCapacity,
    TransitionSamples,
    TransitionAddress,
}
impl From<VoxelVolumeError> for VoxelMeshError {
    fn from(value: VoxelVolumeError) -> Self {
        Self::Volume(value)
    }
}
impl fmt::Display for VoxelMeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Volume(error) => error.fmt(f),
            Self::Vertex => write!(f, "chunk mesh vertices must be finite and within the chunk"),
            Self::Indices => write!(
                f,
                "chunk mesh indices must form triangles and refer to existing vertices"
            ),
            Self::Triangle => write!(f, "chunk mesh triangles must have nonzero area"),
            Self::TransitionPlan => f.write_str("transition plan must contain bounded nodes, sample stencils, and distinct tetrahedron vertices"),
            Self::TransitionCapacity => write!(
                f,
                "transition capacity must be in 1..={} triangles",
                crate::MAX_TRANSITION_TETRAHEDRA * 2
            ),
            Self::TransitionSamples => {
                f.write_str("transition samples must be finite and match the plan")
            }
            Self::TransitionAddress => f.write_str("transition plan must match the source chunk"),
            Self::Capacity => write!(
                f,
                "mesh capacities must be positive and at most {} vertices and {} triangles",
                VOXEL_MESH_VERTEX_SLOTS,
                topology::VOXEL_MESH_MAX_TRIANGLES
            ),
        }
    }
}
impl Error for VoxelMeshError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Volume(e) => Some(e),
            _ => None,
        }
    }
}

pub struct VoxelChunkMesh {
    address: VoxelChunkAddress,
    vertices: Vec<VoxelMeshVertex>,
    indices: Vec<u32>,
}
impl VoxelChunkMesh {
    /// Assemble canonical CPU output or GPU audit readback. Rendering uses the
    /// GPU buffers directly and does not construct this CPU audit container.
    pub fn from_parts(
        address: VoxelChunkAddress,
        vertices: Vec<VoxelMeshVertex>,
        indices: Vec<u32>,
    ) -> Result<Self, VoxelMeshError> {
        let result = Self {
            address,
            vertices,
            indices,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn address(&self) -> VoxelChunkAddress {
        self.address
    }
    pub fn vertices(&self) -> &[VoxelMeshVertex] {
        &self.vertices
    }
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }
    pub fn validate(&self) -> Result<(), VoxelMeshError> {
        let span = self.address.span_m();
        for v in &self.vertices {
            for axis in 0..3 {
                if !(0..=span).contains(&v.anchor_m[axis])
                    || !v.offset_m[axis].is_finite()
                    || !v.normal[axis].is_finite()
                    || v.offset_m[axis].abs() > self.address.spacing_m() as f32
                    || !(0.0..=span as f64)
                        .contains(&(v.anchor_m[axis] as f64 + v.offset_m[axis] as f64))
                {
                    return Err(VoxelMeshError::Vertex);
                }
            }
        }
        if !self.indices.len().is_multiple_of(3)
            || self
                .indices
                .iter()
                .any(|&i| i as usize >= self.vertices.len())
        {
            return Err(VoxelMeshError::Indices);
        }
        for triangle in self.indices.chunks_exact(3) {
            let [a, b, c] =
                [triangle[0], triangle[1], triangle[2]].map(|i| self.vertices[i as usize]);
            // Subtract integer anchors before converting to floats.
            let relative = |v: VoxelMeshVertex| {
                Vec3::new(
                    (v.anchor_m[0] - a.anchor_m[0]) as f32 + (v.offset_m[0] - a.offset_m[0]),
                    (v.anchor_m[1] - a.anchor_m[1]) as f32 + (v.offset_m[1] - a.offset_m[1]),
                    (v.anchor_m[2] - a.anchor_m[2]) as f32 + (v.offset_m[2] - a.offset_m[2]),
                )
            };
            if relative(b).cross(relative(c)).length_squared() == 0.0 {
                return Err(VoxelMeshError::Triangle);
            }
        }
        Ok(())
    }
}

pub(crate) fn candidate(volume: &VoxelVolume, slot: usize) -> Option<VoxelMeshVertex> {
    let p = topology::point(slot / 8, NODES);
    let a = topology::potential(volume, p);
    let spacing = volume.address().spacing_m();
    if slot % 8 == 7 {
        if a != 0.0 {
            return None;
        }
        let touches_air = (1..8).any(|mask| {
            let delta = topology::corner(mask);
            [-1, 1].into_iter().any(|sign| {
                let q = std::array::from_fn(|i| p[i] + sign * delta[i]);
                q.iter().all(|&v| (0..=CELLS as i32).contains(&v))
                    && topology::potential(volume, q) < 0.0
            })
        });
        return touches_air.then(|| VoxelMeshVertex {
            anchor_m: [p[0] * spacing, p[1] * spacing, p[2] * spacing, 0],
            offset_m: [0.0; 4],
            normal: density_normal(volume, p),
        });
    }
    let q = topology::add(p, topology::corner((slot % 8 + 1) as u32));
    if q.iter().any(|&v| v > CELLS as i32) {
        return None;
    }
    let b = topology::potential(volume, q);
    if !((a < 0.0 && b > 0.0) || (a > 0.0 && b < 0.0)) {
        return None;
    }
    let mut vertex = edge_vertex(p.map(|v| v * spacing), q.map(|v| v * spacing), a, b);
    let (start, end, near, far) = if a.abs() <= b.abs() {
        (p, q, a, b)
    } else {
        (q, p, b, a)
    };
    let start_normal = density_normal(volume, start);
    let end_normal = density_normal(volume, end);
    let fraction = edge_fraction(near, far);
    vertex.normal =
        std::array::from_fn(|i| start_normal[i] + (end_normal[i] - start_normal[i]) * fraction);
    Some(vertex)
}

fn density_normal(volume: &VoxelVolume, point: [i32; 3]) -> [f32; 4] {
    let mut result = [0.0; 4];
    for axis in 0..3 {
        let mut lo = point;
        let mut hi = point;
        lo[axis] -= 1;
        hi[axis] += 1;
        // Positive density is solid: its negative gradient points outward.
        // The existing halo supplies both sides even on chunk boundaries.
        result[axis] = (topology::potential(volume, lo) - topology::potential(volume, hi))
            / (2.0 * volume.address().spacing_m() as f32);
    }
    result
}

fn edge_fraction(near: f32, far: f32) -> f32 {
    let ratio = (near / far).abs();
    ratio / (1.0 + ratio)
}

pub(crate) fn edge_vertex(a: [i32; 3], b: [i32; 3], da: f32, db: f32) -> VoxelMeshVertex {
    // Canonical endpoint order also covers non-monotone transition edges.
    let (a, b, da, db) = if a <= b {
        (a, b, da, db)
    } else {
        (b, a, db, da)
    };
    let (start, end, near, far) = if da.abs() <= db.abs() {
        (a, b, da, db)
    } else {
        (b, a, db, da)
    };
    let t = edge_fraction(near, far);
    VoxelMeshVertex {
        anchor_m: [start[0], start[1], start[2], 0],
        offset_m: [
            (end[0] - start[0]) as f32 * t,
            (end[1] - start[1]) as f32 * t,
            (end[2] - start[2]) as f32 * t,
            0.0,
        ],
        // Transition stencils have no regular density halo. Their consumer uses
        // face normals; regular extraction supplies density normals afterward.
        normal: [0.0; 4],
    }
}

pub fn build_voxel_chunk_mesh(volume: &VoxelVolume) -> Result<VoxelChunkMesh, VoxelMeshError> {
    build_regular_mesh(
        volume,
        &[0; crate::voxel_transition_plan::VOXEL_CELL_MASK_WORDS],
    )
}

pub(crate) fn build_regular_mesh(
    volume: &VoxelVolume,
    blocked: &[u32; crate::voxel_transition_plan::VOXEL_CELL_MASK_WORDS],
) -> Result<VoxelChunkMesh, VoxelMeshError> {
    volume.validate()?;
    let candidates: Vec<_> = (0..VOXEL_MESH_VERTEX_SLOTS)
        .into_par_iter()
        .map(|i| candidate(volume, i))
        .collect();
    let mut vertices = Vec::new();
    let mut offsets = vec![NONE; VOXEL_MESH_VERTEX_SLOTS];
    for (i, vertex) in candidates.into_iter().enumerate() {
        if let Some(vertex) = vertex {
            offsets[i] = vertices.len() as u32;
            vertices.push(vertex);
        }
    }
    let cells: Vec<_> = (0..CELL_COUNT)
        .into_par_iter()
        .map(|i| {
            if blocked[i / 32] & (1 << (i % 32)) == 0 {
                topology::cell_triangles(volume, i)
            } else {
                topology::CellTriangles {
                    roots: [[0; 3]; 12],
                    count: 0,
                }
            }
        })
        .collect();
    let indices = cells
        .iter()
        .flat_map(|c| c.roots[..c.count].iter().flatten())
        .map(|&root| {
            let index = offsets[root as usize];
            debug_assert_ne!(index, NONE);
            index
        })
        .collect();
    VoxelChunkMesh::from_parts(volume.address(), vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn exact_zero_plane_is_wound_and_has_no_interior_boundaries() {
        let address = VoxelChunkAddress::containing(
            VoxelPosition {
                x_m: 0,
                y_m: 0,
                z_m: 0,
            },
            0,
        )
        .unwrap();
        let volume = VoxelVolume::fixture(address, |p| (16 - p.x_m) as f32);
        let one = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(|| build_voxel_chunk_mesh(&volume).unwrap());
        let many = build_voxel_chunk_mesh(&volume).unwrap();
        assert_eq!(one.vertices(), many.vertices());
        assert_eq!(one.indices(), many.indices());
        assert_eq!(many.indices().len() / 3, 32 * 32 * 2);
        let positions: Vec<_> = many
            .vertices()
            .iter()
            .map(|v| v.position(address).relative_to(address.origin()))
            .collect();
        assert!(positions.iter().all(|p| p.x == 16.0));
        assert!(
            many.vertices()
                .iter()
                .all(|v| v.normal == [1.0, 0.0, 0.0, 0.0])
        );
        let mut edges = BTreeMap::<[u32; 2], usize>::new();
        for t in many.indices().chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| positions[i as usize]);
            assert!((b - a).cross(c - a).x > 0.0);
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *edges.entry([a.min(b), a.max(b)]).or_default() += 1;
            }
        }
        for ([a, b], count) in edges {
            if count == 1 {
                let (a, b) = (positions[a as usize], positions[b as usize]);
                assert!(
                    [0.0, 32.0]
                        .into_iter()
                        .any(|v| (a.y == v && b.y == v) || (a.z == v && b.z == v))
                );
            } else {
                assert_eq!(count, 2);
            }
        }
    }

    #[test]
    fn density_normals_follow_a_curved_field_across_chunk_boundaries() {
        for x in [0, 32] {
            let address = VoxelChunkAddress::containing(
                VoxelPosition {
                    x_m: x,
                    y_m: 0,
                    z_m: 0,
                },
                0,
            )
            .unwrap();
            // A quadratic has an independent analytic gradient at every root.
            let volume = VoxelVolume::fixture(address, |p| {
                let d = Vec3::new(
                    (p.x_m - 32) as f32,
                    (p.y_m - 16) as f32,
                    (p.z_m - 16) as f32,
                );
                22.0 * 22.0 - d.length_squared()
            });
            let mesh = build_voxel_chunk_mesh(&volume).unwrap();
            assert!(!mesh.vertices().is_empty());
            for v in mesh.vertices() {
                let p = v.position(address).relative_to(VoxelPosition {
                    x_m: 32,
                    y_m: 16,
                    z_m: 16,
                });
                let n = Vec3::new(v.normal[0], v.normal[1], v.normal[2]);
                assert!(
                    (n - p * 2.0).length() < 0.0001,
                    "outward analytic gradient, including halo endpoints"
                );
            }
        }
    }

    #[test]
    fn invalid_readback_is_rejected_by_its_owner() {
        let a = VoxelChunkAddress::root();
        assert!(VoxelVolume::from_potentials(a, vec![0.0]).is_err());
        assert!(
            VoxelVolume::from_potentials(a, vec![f32::NAN; crate::VOXEL_SAMPLE_COUNT]).is_err()
        );
        let vertex = VoxelMeshVertex::zeroed();
        let bad_normal = VoxelMeshVertex {
            normal: [f32::NAN; 4],
            ..vertex
        };
        assert!(matches!(
            VoxelChunkMesh::from_parts(a, vec![bad_normal], vec![]),
            Err(VoxelMeshError::Vertex)
        ));
        assert!(matches!(
            VoxelChunkMesh::from_parts(a, vec![vertex], vec![0, 0, 1]),
            Err(VoxelMeshError::Indices)
        ));
        assert!(matches!(
            VoxelChunkMesh::from_parts(a, vec![vertex], vec![0, 0, 0]),
            Err(VoxelMeshError::Triangle)
        ));
        let invalid = VoxelMeshVertex {
            offset_m: [f32::NAN; 4],
            ..vertex
        };
        assert!(matches!(
            VoxelChunkMesh::from_parts(a, vec![invalid], vec![]),
            Err(VoxelMeshError::Vertex)
        ));
    }
}
