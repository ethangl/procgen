//! Closed-box 2:1 balance includes face, edge, and corner neighbors.
use crate::voxel_leaf_index::LeafIndex;
use crate::{VoxelChunkAddress, VoxelPosition};
use std::{collections::BTreeSet, error::Error, fmt};

pub const MAX_VOXEL_COVERAGE_LEAVES: usize = 4096;
#[derive(Debug)]
pub enum VoxelCoverageError {
    Capacity,
    Overlap,
    Balance,
    Missing,
}
impl fmt::Display for VoxelCoverageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Capacity => "voxel coverage exceeds its leaf budget",
            Self::Overlap => "voxel coverage leaves overlap or repeat",
            Self::Balance => "touching voxel leaves must have at most a 2:1 spacing ratio",
            Self::Missing => "chunk is not in this voxel coverage",
        })
    }
}
impl Error for VoxelCoverageError {}

pub(crate) fn bounds(a: VoxelChunkAddress) -> ([i32; 3], [i32; 3]) {
    let p = a.origin();
    let lo = [p.x_m, p.y_m, p.z_m];
    (lo, lo.map(|v| v + a.span_m()))
}
pub(crate) fn touching(a: VoxelChunkAddress, b: VoxelChunkAddress) -> bool {
    let (al, ah) = bounds(a);
    let (bl, bh) = bounds(b);
    (0..3).all(|i| al[i] <= bh[i] && bl[i] <= ah[i])
}

// A point-only neighbor introduces no new source node or face subdivision.
fn shares_segment(a: VoxelChunkAddress, b: VoxelChunkAddress) -> bool {
    let (al, ah) = bounds(a);
    let (bl, bh) = bounds(b);
    (0..3).any(|i| al[i].max(bl[i]) < ah[i].min(bh[i]))
}

pub(crate) fn contains(a: VoxelChunkAddress, p: [i32; 3]) -> bool {
    let (lo, hi) = bounds(a);
    (0..3).all(|i| lo[i] <= p[i] && p[i] <= hi[i])
}

/// Only finer incident leaves affect a chunk's extraction. Equal/coarser neighbor
/// changes do not invalidate this key. Addresses are sorted for deterministic plans.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct VoxelMeshKey {
    pub(crate) address: VoxelChunkAddress,
    pub(crate) finer: Vec<VoxelChunkAddress>,
}
impl VoxelMeshKey {
    pub fn address(&self) -> VoxelChunkAddress {
        self.address
    }
    pub fn finer_neighbors(&self) -> &[VoxelChunkAddress] {
        &self.finer
    }
}

#[derive(Clone)]
pub struct VoxelCoverage {
    leaves: Vec<VoxelChunkAddress>,
    index: Option<LeafIndex>,
}
impl VoxelCoverage {
    pub fn new(mut leaves: Vec<VoxelChunkAddress>) -> Result<Self, VoxelCoverageError> {
        leaves.sort();
        if leaves.len() > MAX_VOXEL_COVERAGE_LEAVES {
            return Err(VoxelCoverageError::Capacity);
        }
        let set: BTreeSet<_> = leaves.iter().copied().collect();
        if set.len() != leaves.len() {
            return Err(VoxelCoverageError::Overlap);
        }
        check_overlap(&set)?;
        let index = LeafIndex::new(&leaves);
        let result = Self { leaves, index };
        result.validate()?;
        Ok(result)
    }
    /// Refine coarse neighbors rather than removing requested fine detail. A
    /// budget failure leaves the caller's prior coverage available for display.
    pub fn balanced(
        leaves: Vec<VoxelChunkAddress>,
        max_leaves: usize,
    ) -> Result<Self, VoxelCoverageError> {
        if max_leaves == 0 || max_leaves > MAX_VOXEL_COVERAGE_LEAVES || leaves.len() > max_leaves {
            return Err(VoxelCoverageError::Capacity);
        }
        let mut set: BTreeSet<_> = leaves.iter().copied().collect();
        if set.len() != leaves.len() {
            return Err(VoxelCoverageError::Overlap);
        }
        check_overlap(&set)?;
        loop {
            let mut split = BTreeSet::new();
            let addresses: Vec<_> = set.iter().copied().collect();
            if let Some(index) = LeafIndex::new(&addresses) {
                let mut neighbors = Vec::new();
                for &a in &addresses {
                    index.neighbors(a, (a.lod() + 2)..=crate::VOXEL_ROOT_LOD, &mut neighbors);
                }
                split.extend(neighbors);
            }
            if split.is_empty() {
                break;
            }
            if set.len() + 7 * split.len() > max_leaves {
                return Err(VoxelCoverageError::Capacity);
            }
            for a in split {
                set.remove(&a);
                set.extend(a.children().expect("coarse neighbor has children"));
            }
        }
        Self::new(set.into_iter().collect())
    }
    pub fn leaves(&self) -> &[VoxelChunkAddress] {
        &self.leaves
    }
    pub fn validate(&self) -> Result<(), VoxelCoverageError> {
        if self.leaves.len() > MAX_VOXEL_COVERAGE_LEAVES {
            return Err(VoxelCoverageError::Capacity);
        }
        let set: BTreeSet<_> = self.leaves.iter().copied().collect();
        if set.len() != self.leaves.len() {
            return Err(VoxelCoverageError::Overlap);
        }
        check_overlap(&set)?;
        if let Some(index) = &self.index {
            let mut neighbors = Vec::new();
            for &a in &self.leaves {
                index.neighbors(a, (a.lod() + 2)..=crate::VOXEL_ROOT_LOD, &mut neighbors);
                if !neighbors.is_empty() {
                    return Err(VoxelCoverageError::Balance);
                }
            }
        }
        Ok(())
    }
    pub fn key(&self, address: VoxelChunkAddress) -> Result<VoxelMeshKey, VoxelCoverageError> {
        if self.leaves.binary_search(&address).is_err() {
            return Err(VoxelCoverageError::Missing);
        }
        let mut finer = Vec::new();
        if address.lod() > 0 {
            self.index.as_ref().expect("owned leaf index").neighbors(
                address,
                0..=address.lod() - 1,
                &mut finer,
            );
            finer.retain(|&a| shares_segment(address, a));
            finer.sort();
        }
        Ok(VoxelMeshKey { address, finer })
    }

    pub fn keys(&self) -> impl Iterator<Item = VoxelMeshKey> + '_ {
        self.leaves
            .iter()
            .map(|&a| self.key(a).expect("owned leaf"))
    }
}
fn check_overlap(set: &BTreeSet<VoxelChunkAddress>) -> Result<(), VoxelCoverageError> {
    for &a in set {
        if std::iter::successors(a.parent(), |a| a.parent()).any(|a| set.contains(&a)) {
            return Err(VoxelCoverageError::Overlap);
        }
    }
    Ok(())
}

/// Current CPU selection followed by bounded balancing. Exposed separately from
/// residency so consumers can retain old coverage when its budget is insufficient.
pub fn select_voxel_coverage(
    field: &crate::PlanetDesignField,
    camera: VoxelPosition,
    requested_leaves: usize,
    balanced_limit: usize,
) -> Result<VoxelCoverage, VoxelCoverageError> {
    if requested_leaves == 0 || requested_leaves > balanced_limit {
        return Err(VoxelCoverageError::Capacity);
    }
    VoxelCoverage::balanced(
        crate::voxel_selection::select_leaves(field, camera, requested_leaves),
        balanced_limit,
    )
}
