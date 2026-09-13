//! Integer identities, chunk lookup, and active cells for conforming extraction.
use crate::voxel_surface::{MAX_ACTIVE_CELLS, MAX_SURFACE_CHUNKS};
use crate::{VoxelChunkAddress, VoxelPosition, VoxelSampleIndex, VoxelSurfaceError, VoxelVolume};
use procgen_core::Vec3;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicBool, Ordering},
};

/// Half-meter node coordinates accommodate derived face and cell centers.
/// Source voxel corners remain integer meters at their original LOD.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Node(pub [i32; 3]);
impl Node {
    pub fn relative(self, origin: VoxelPosition) -> Vec3 {
        Vec3::new(
            (self.0[0] - 2 * origin.x_m) as f32,
            (self.0[1] - 2 * origin.y_m) as f32,
            (self.0[2] - 2 * origin.z_m) as f32,
        ) * 0.5
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Cell {
    pub min: [i32; 3],
    pub chunk: VoxelChunkAddress,
}
impl Cell {
    pub fn step(self) -> i32 {
        self.chunk.spacing_m()
    }
    pub fn center(self) -> Node {
        Node(self.min.map(|p| 2 * p + self.step()))
    }
}
pub(crate) struct Grid<'a> {
    pub volumes: BTreeMap<VoxelChunkAddress, &'a VoxelVolume>,
}
fn coords(p: VoxelPosition) -> [i32; 3] {
    [p.x_m, p.y_m, p.z_m]
}
pub(crate) fn contains(chunk: VoxelChunkAddress, node: Node) -> bool {
    let min = coords(chunk.origin());
    (0..3).all(|a| node.0[a] >= 2 * min[a] && node.0[a] <= 2 * (min[a] + chunk.span_m()))
}
pub(crate) fn touching(a: VoxelChunkAddress, b: VoxelChunkAddress) -> bool {
    let am = coords(a.origin());
    let bm = coords(b.origin());
    (0..3).all(|i| am[i] <= bm[i] + b.span_m() && bm[i] <= am[i] + a.span_m())
}
impl<'a> Grid<'a> {
    pub fn new(volumes: &[&'a VoxelVolume]) -> Result<Self, VoxelSurfaceError> {
        if !(1..=MAX_SURFACE_CHUNKS).contains(&volumes.len()) {
            return Err(VoxelSurfaceError::Chunks);
        }
        let mut result = Self {
            volumes: BTreeMap::new(),
        };
        for &v in volumes {
            v.validate()?;
            if result.volumes.insert(v.address(), v).is_some() {
                return Err(VoxelSurfaceError::Overlap);
            }
        }
        for &a in result.volumes.keys() {
            if std::iter::successors(a.parent(), |a| a.parent())
                .any(|p| result.volumes.contains_key(&p))
            {
                return Err(VoxelSurfaceError::Overlap);
            }
        }
        Ok(result)
    }
    pub fn potential(&self, point: Node) -> f32 {
        // Address ordering puts the finest incident volume first. On a shared
        // boundary both sides thus use one interpolant and one sample identity.
        let (&a, v) = self
            .volumes
            .iter()
            .find(|(a, _)| contains(**a, point))
            .expect("node belongs to an input chunk");
        let min = coords(a.origin());
        let local: [f32; 3] =
            std::array::from_fn(|i| (point.0[i] - 2 * min[i]) as f32 / (2 * a.spacing_m()) as f32);
        let base = local.map(|p| if p == 32.0 { 31 } else { p.floor() as i32 });
        let frac: [f32; 3] = std::array::from_fn(|i| local[i] - base[i] as f32);
        let mut result = 0.0;
        for corner in 0..8 {
            let offset = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
            let w: f32 = (0..3)
                .map(|i| {
                    if offset[i] == 0 {
                        1.0 - frac[i]
                    } else {
                        frac[i]
                    }
                })
                .product();
            result += w * v.potential(VoxelSampleIndex {
                x: base[0] + offset[0],
                y: base[1] + offset[1],
                z: base[2] + offset[2],
            });
        }
        result
    }
    fn cell_values(&self, cell: Cell) -> [f32; 8] {
        let v = self.volumes[&cell.chunk];
        let min = coords(cell.chunk.origin());
        let base: [i32; 3] = std::array::from_fn(|i| (cell.min[i] - min[i]) / cell.step());
        std::array::from_fn(|i| {
            v.potential(VoxelSampleIndex {
                x: base[0] + (i as i32 & 1),
                y: base[1] + ((i as i32 >> 1) & 1),
                z: base[2] + ((i as i32 >> 2) & 1),
            })
        })
    }
    pub fn active_cells(&self, cancel: &AtomicBool) -> Result<BTreeSet<Cell>, VoxelSurfaceError> {
        let mut active = BTreeSet::new();
        for (&chunk, volume) in &self.volumes {
            check_cancel(cancel)?;
            let min = coords(chunk.origin());
            let step = chunk.spacing_m();
            for z in 0..32 {
                check_cancel(cancel)?;
                for y in 0..32 {
                    for x in 0..32 {
                        let cell = Cell {
                            min: [min[0] + x * step, min[1] + y * step, min[2] + z * step],
                            chunk,
                        };
                        let values = self.cell_values(cell);
                        if values.iter().any(|&v| (v >= 0.0) != (values[0] >= 0.0)) {
                            include_cell(&mut active, cell)?;
                        }
                    }
                }
            }
            // A fine boundary can cross a coarse cell whose eight corners all
            // have one sign. Include that cell too; corner-only culling loses it.
            let coarse: Vec<_> = self
                .volumes
                .keys()
                .copied()
                .filter(|&a| a.lod() > chunk.lod() && touching(a, chunk))
                .collect();
            if !coarse.is_empty() {
                for z in 0..=32 {
                    check_cancel(cancel)?;
                    for y in 0..=32 {
                        for x in 0..=32 {
                            if ![x, y, z].into_iter().any(|i| i == 0 || i == 32) {
                                continue;
                            }
                            let p = [min[0] + x * step, min[1] + y * step, min[2] + z * step];
                            let node = Node(p.map(|p| 2 * p));
                            let density = volume.potential(VoxelSampleIndex { x, y, z });
                            for &chunk in &coarse {
                                if !contains(chunk, node) {
                                    continue;
                                }
                                let cm = coords(chunk.origin());
                                let step = chunk.spacing_m();
                                let rel: [i32; 3] = std::array::from_fn(|i| p[i] - cm[i]);
                                let base = rel.map(|p| p / step);
                                for mask in 0..8 {
                                    let index: [i32; 3] =
                                        std::array::from_fn(|i| base[i] - ((mask >> i) & 1));
                                    if (0..3).any(|i| {
                                        !(0..32).contains(&index[i])
                                            || (((mask >> i) & 1) == 1 && rel[i] % step != 0)
                                    }) {
                                        continue;
                                    }
                                    let cell = Cell {
                                        min: std::array::from_fn(|i| cm[i] + index[i] * step),
                                        chunk,
                                    };
                                    if self
                                        .cell_values(cell)
                                        .iter()
                                        .any(|&v| (v >= 0.0) != (density >= 0.0))
                                    {
                                        include_cell(&mut active, cell)?;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(active)
    }
}
fn include_cell(active: &mut BTreeSet<Cell>, cell: Cell) -> Result<(), VoxelSurfaceError> {
    if active.len() == MAX_ACTIVE_CELLS && !active.contains(&cell) {
        return Err(VoxelSurfaceError::Capacity);
    }
    active.insert(cell);
    Ok(())
}
pub(crate) fn check_cancel(cancel: &AtomicBool) -> Result<(), VoxelSurfaceError> {
    if cancel.load(Ordering::Relaxed) {
        Err(VoxelSurfaceError::Cancelled)
    } else {
        Ok(())
    }
}
