//! Immutable sparse octree for neighborhood queries; no density or LOD policy.
use crate::VoxelChunkAddress;
use crate::voxel_neighborhood::{bounds, touching};

#[derive(Clone)]
pub(crate) struct LeafIndex {
    address: VoxelChunkAddress,
    children: Vec<LeafIndex>,
}
impl LeafIndex {
    pub fn new(leaves: &[VoxelChunkAddress]) -> Option<Self> {
        (!leaves.is_empty()).then(|| Self::build(VoxelChunkAddress::root(), leaves))
    }
    fn build(address: VoxelChunkAddress, leaves: &[VoxelChunkAddress]) -> Self {
        if leaves.len() == 1 && leaves[0] == address {
            return Self {
                address,
                children: Vec::new(),
            };
        }
        let children = address
            .children()
            .expect("non-overlapping descendant leaves");
        let (lo, _) = bounds(address);
        let center = lo.map(|v| v + address.span_m() / 2);
        let mut groups: [Vec<VoxelChunkAddress>; 8] = std::array::from_fn(|_| Vec::new());
        for &leaf in leaves {
            let p = leaf.origin();
            let index = usize::from(p.x_m >= center[0])
                | (usize::from(p.y_m >= center[1]) << 1)
                | (usize::from(p.z_m >= center[2]) << 2);
            groups[index].push(leaf);
        }
        Self {
            address,
            children: groups
                .iter()
                .enumerate()
                .filter(|(_, g)| !g.is_empty())
                .map(|(i, g)| Self::build(children[i], g))
                .collect(),
        }
    }
    pub fn neighbors(
        &self,
        address: VoxelChunkAddress,
        lods: std::ops::RangeInclusive<u8>,
        out: &mut Vec<VoxelChunkAddress>,
    ) {
        if self.address.lod() < *lods.start() || !touching(self.address, address) {
            return;
        }
        if self.children.is_empty() {
            if self.address != address && lods.contains(&self.address.lod()) {
                out.push(self.address);
            }
        } else {
            for child in &self.children {
                child.neighbors(address, lods.clone(), out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_queries_match_exhaustive_faces_edges_and_corners() {
        let root = VoxelChunkAddress::containing(
            crate::VoxelPosition {
                x_m: -1,
                y_m: -1,
                z_m: -1,
            },
            5,
        )
        .unwrap();
        let mut leaves = vec![root];
        for i in 0..40 {
            let index = (i * 17) % leaves.len();
            if let Some(children) = leaves[index].children() {
                leaves.remove(index);
                leaves.extend(children);
            }
        }
        leaves = leaves
            .into_iter()
            .enumerate()
            .filter_map(|(i, a)| (i % 5 != 0).then_some(a))
            .collect();
        let index = LeafIndex::new(&leaves).unwrap();
        for &a in &leaves {
            for lods in [
                0..=crate::VOXEL_ROOT_LOD,
                (a.lod() + 2)..=crate::VOXEL_ROOT_LOD,
            ] {
                let mut actual = Vec::new();
                index.neighbors(a, lods.clone(), &mut actual);
                actual.sort();
                let mut expected: Vec<_> = leaves
                    .iter()
                    .copied()
                    .filter(|&b| a != b && lods.contains(&b.lod()) && touching(a, b))
                    .collect();
                expected.sort();
                assert_eq!(actual, expected);
            }
        }
    }
}
