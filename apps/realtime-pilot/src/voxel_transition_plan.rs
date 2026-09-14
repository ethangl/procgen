//! Density-independent transition topology for one 2:1 chunk neighborhood.
use crate::voxel_mesh_topology::{CELL_COUNT, CELLS, point};
use crate::voxel_neighborhood::{bounds, contains};
use crate::{VoxelChunkAddress, VoxelMeshError, VoxelMeshKey, VoxelPosition};
use bytemuck::{Pod, Zeroable};
use std::collections::BTreeMap;

pub const VOXEL_CELL_MASK_WORDS: usize = CELL_COUNT / 32;
const BOUNDARY_CELLS: usize = CELL_COUNT - (CELLS - 2).pow(3);
pub(crate) const MAX_TRANSITION_NODES: usize = BOUNDARY_CELLS * 27;
pub const MAX_TRANSITION_TETRAHEDRA: usize = BOUNDARY_CELLS * 48;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct VoxelTransitionNode {
    pub(crate) position_m: [i32; 4],
    pub(crate) samples: [u32; 8],
    pub(crate) sample_count: u32,
    pub(crate) reserved: [u32; 3],
}

/// Bounded boundary topology, compiled without inspecting density. Interiors keep
/// G2's six tetrahedra. Each affected cell cones shared face triangles to its center.
pub struct VoxelTransitionPlan {
    key: VoxelMeshKey,
    pub(crate) blocked: [u32; VOXEL_CELL_MASK_WORDS],
    nodes: Vec<VoxelTransitionNode>,
    tetrahedra: Vec<[u32; 4]>,
    samples: Vec<VoxelPosition>,
}
impl VoxelTransitionPlan {
    pub fn new(key: VoxelMeshKey) -> Self {
        let mut builder = Builder {
            plan: Self {
                key,
                blocked: [0; VOXEL_CELL_MASK_WORDS],
                nodes: Vec::new(),
                tetrahedra: Vec::new(),
                samples: Vec::new(),
            },
            nodes: BTreeMap::new(),
            samples: BTreeMap::new(),
        };
        if !builder.plan.key.finer.is_empty() {
            for cell in 0..CELL_COUNT {
                let p = point(cell, CELLS);
                if p.iter().any(|&v| v == 0 || v == CELLS as i32 - 1) {
                    builder.cell(cell, p);
                }
            }
        }
        debug_assert!(builder.plan.tetrahedra.len() <= MAX_TRANSITION_TETRAHEDRA);
        builder.plan
    }
    pub fn validate(&self) -> Result<(), VoxelMeshError> {
        let span = self.key.address.span_m();
        if self.nodes.len() > MAX_TRANSITION_NODES
            || self.samples.len() > self.nodes.len() * 8
            || self.tetrahedra.len() > MAX_TRANSITION_TETRAHEDRA
            || self.replaced_cells() > BOUNDARY_CELLS
            || self.nodes.iter().any(|n| {
                ![1, 2, 4, 8].contains(&n.sample_count)
                    || n.position_m[..3].iter().any(|&p| !(0..=span).contains(&p))
                    || n.samples[..n.sample_count as usize]
                        .iter()
                        .any(|&i| i as usize >= self.samples.len())
            })
            || self.tetrahedra.iter().any(|t| {
                t.iter().any(|&i| i as usize >= self.nodes.len())
                    || (0..4).any(|i| t[..i].contains(&t[i]))
            })
        {
            return Err(VoxelMeshError::TransitionPlan);
        }
        Ok(())
    }
    pub fn key(&self) -> &VoxelMeshKey {
        &self.key
    }
    pub fn nodes(&self) -> &[VoxelTransitionNode] {
        &self.nodes
    }
    pub fn tetrahedra(&self) -> &[[u32; 4]] {
        &self.tetrahedra
    }
    pub fn sample_points(&self) -> &[VoxelPosition] {
        &self.samples
    }
    pub fn replaced_cells(&self) -> usize {
        self.blocked.iter().map(|w| w.count_ones() as usize).sum()
    }
    pub fn allocated_bytes(&self) -> usize {
        self.nodes.capacity() * size_of::<VoxelTransitionNode>()
            + self.tetrahedra.capacity() * 16
            + self.samples.capacity() * size_of::<VoxelPosition>()
            + size_of_val(&self.blocked)
    }
}
struct Builder {
    plan: VoxelTransitionPlan,
    nodes: BTreeMap<[i32; 3], u32>,
    samples: BTreeMap<[i32; 3], u32>,
}
impl Builder {
    fn cell(&mut self, cell: usize, p: [i32; 3]) {
        let address = self.plan.key.address;
        let step = address.spacing_m();
        let (origin, _) = bounds(address);
        let min = std::array::from_fn(|i| origin[i] + p[i] * step);
        let faces: Vec<_> = (0..3)
            .flat_map(|axis| (0..2).map(move |side| (axis, side)))
            .map(|(axis, side)| Face::new(min, step, axis, side, &self.plan.key.finer))
            .collect();
        if !faces.iter().any(|f| f.refined || f.split_edges != 0) {
            return;
        }
        self.plan.blocked[cell / 32] |= 1 << (cell % 32);
        let center = self.node(min.map(|v| v + step / 2));
        for face in faces {
            for triangle in face.triangles() {
                let [a, b, c] = triangle.map(|p| self.node(p));
                self.plan.tetrahedra.push([center, a, b, c]);
            }
        }
    }
    fn node(&mut self, p: [i32; 3]) -> u32 {
        if let Some(&id) = self.nodes.get(&p) {
            return id;
        }
        let source = self
            .plan
            .key
            .finer
            .iter()
            .copied()
            .find(|&a| contains(a, p))
            .unwrap_or(self.plan.key.address);
        let (lo, _) = bounds(source);
        let step = source.spacing_m();
        let base: [i32; 3] = std::array::from_fn(|i| lo[i] + (p[i] - lo[i]) / step * step);
        let fractional: u32 = (0..3).filter(|&i| p[i] != base[i]).map(|i| 1 << i).sum();
        debug_assert!((0..3).all(|i| p[i] == base[i] || 2 * (p[i] - base[i]) == step));
        let mut samples = [0; 8];
        let mut sample_count = 0;
        for corner in 0..8u32 {
            if corner & !fractional != 0 {
                continue;
            }
            let sample = std::array::from_fn(|i| base[i] + ((corner >> i) & 1) as i32 * step);
            let id = *self.samples.entry(sample).or_insert_with(|| {
                let id = self.plan.samples.len() as u32;
                self.plan.samples.push(VoxelPosition {
                    x_m: sample[0],
                    y_m: sample[1],
                    z_m: sample[2],
                });
                id
            });
            samples[sample_count] = id;
            sample_count += 1;
        }
        let id = self.plan.nodes.len() as u32;
        let (origin, _) = bounds(self.plan.key.address);
        self.plan.nodes.push(VoxelTransitionNode {
            position_m: [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2], 0],
            samples,
            sample_count: sample_count as u32,
            reserved: [0; 3],
        });
        self.nodes.insert(p, id);
        id
    }
}
struct Face {
    corners: [[i32; 3]; 4],
    axis: usize,
    side: usize,
    refined: bool,
    split_edges: u32,
}
impl Face {
    fn new(
        min: [i32; 3],
        step: i32,
        axis: usize,
        side: usize,
        finer: &[VoxelChunkAddress],
    ) -> Self {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        let corners = [[0, 0], [1, 0], [1, 1], [0, 1]].map(|c| {
            let mut p = min;
            p[axis] += side as i32 * step;
            p[u] += c[0] * step;
            p[v] += c[1] * step;
            p
        });
        let refined = finer.iter().any(|&a| {
            let (lo, hi) = bounds(a);
            lo[axis] <= corners[0][axis]
                && corners[0][axis] <= hi[axis]
                && [u, v]
                    .into_iter()
                    .all(|i| corners[0][i] < hi[i] && corners[2][i] > lo[i])
        });
        let mut split_edges = 0;
        for edge in 0..4 {
            let (a, b) = (corners[edge], corners[(edge + 1) % 4]);
            if finer.iter().any(|&c| {
                let (lo, hi) = bounds(c);
                (0..3).all(|i| {
                    if a[i] == b[i] {
                        lo[i] <= a[i] && a[i] <= hi[i]
                    } else {
                        a[i].min(b[i]) < hi[i] && a[i].max(b[i]) > lo[i]
                    }
                })
            }) {
                split_edges |= 1 << edge;
            }
        }
        Self {
            corners,
            axis,
            side,
            refined,
            split_edges,
        }
    }
    fn triangles(&self) -> Vec<[[i32; 3]; 3]> {
        let mut triangles;
        if self.refined {
            let (u, v) = ((self.axis + 1) % 3, (self.axis + 2) % 3);
            let half = (self.corners[2][u] - self.corners[0][u]) / 2;
            triangles = Vec::with_capacity(8);
            for y in 0..2 {
                for x in 0..2 {
                    let corners = [[0, 0], [1, 0], [1, 1], [0, 1]].map(|c| {
                        let mut p = self.corners[0];
                        p[u] += (x + c[0]) * half;
                        p[v] += (y + c[1]) * half;
                        p
                    });
                    triangles.extend([
                        [corners[0], corners[1], corners[2]],
                        [corners[0], corners[2], corners[3]],
                    ]);
                }
            }
        } else {
            triangles = vec![
                [self.corners[0], self.corners[1], self.corners[2]],
                [self.corners[0], self.corners[2], self.corners[3]],
            ];
            for edge in 0..4 {
                if self.split_edges & (1 << edge) == 0 {
                    continue;
                }
                let (a, b) = (self.corners[edge], self.corners[(edge + 1) % 4]);
                let midpoint = std::array::from_fn(|i| (a[i] + b[i]) / 2);
                let index = triangles
                    .iter()
                    .position(|t| t.contains(&a) && t.contains(&b))
                    .expect("face perimeter edge");
                let t = triangles.remove(index);
                let i = (0..3)
                    .find(|&i| {
                        (t[i] == a && t[(i + 1) % 3] == b) || (t[i] == b && t[(i + 1) % 3] == a)
                    })
                    .unwrap();
                triangles.extend([
                    [t[i], midpoint, t[(i + 2) % 3]],
                    [midpoint, t[(i + 1) % 3], t[(i + 2) % 3]],
                ]);
            }
        }
        if self.side == 0 {
            for t in &mut triangles {
                t.swap(1, 2);
            }
        }
        triangles
    }
}
