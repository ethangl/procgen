//! Integer, planet-centered Cartesian octree. No allocation or traversal policy.
use procgen_core::Vec3;
use serde::Serialize;
use std::{error::Error, fmt};

/// Every chunk has the same number of cells; LOD zero has one-meter cell edges.
pub const VOXEL_CHUNK_CELLS: i32 = 32;
pub const VOXEL_ROOT_LOD: u8 = 19;
/// Covers the supported 8,000 km radius plus all configured relief.
pub const VOXEL_WORLD_HALF_EXTENT_M: i32 = (VOXEL_CHUNK_CELLS << VOXEL_ROOT_LOD) / 2;
pub const VOXEL_HALO: i32 = 1;
pub const VOXEL_SAMPLE_SIDE: usize = (VOXEL_CHUNK_CELLS + 1 + 2 * VOXEL_HALO) as usize;
pub const VOXEL_SAMPLE_COUNT: usize = VOXEL_SAMPLE_SIDE.pow(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct VoxelPosition {
    pub x_m: i32,
    pub y_m: i32,
    pub z_m: i32,
}
impl VoxelPosition {
    /// Subtract integer origins before any float conversion. The difference
    /// must fit in i32, as it does for all root and halo positions.
    pub fn relative_to(self, origin: Self) -> Self {
        let difference =
            |a: i32, b: i32| a.checked_sub(b).expect("relative coordinate must fit i32");
        Self {
            x_m: difference(self.x_m, origin.x_m),
            y_m: difference(self.y_m, origin.y_m),
            z_m: difference(self.z_m, origin.z_m),
        }
    }
    pub(crate) fn as_vec3(self) -> Vec3 {
        Vec3::new(self.x_m as f32, self.y_m as f32, self.z_m as f32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ChunkIndex {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

/// Sample coordinates include the shared endpoints and a one-cell halo:
/// -1..=33 on every axis. This is a grid index, not a physical distance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoxelSampleIndex {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}
impl VoxelSampleIndex {
    pub(crate) fn in_chunk(self) -> bool {
        [self.x, self.y, self.z]
            .into_iter()
            .all(|v| (-VOXEL_HALO..=VOXEL_CHUNK_CELLS + VOXEL_HALO).contains(&v))
    }
    pub(crate) fn from_linear(i: usize) -> Self {
        Self {
            x: (i % VOXEL_SAMPLE_SIDE) as i32 - VOXEL_HALO,
            y: (i / VOXEL_SAMPLE_SIDE % VOXEL_SAMPLE_SIDE) as i32 - VOXEL_HALO,
            z: (i / VOXEL_SAMPLE_SIDE.pow(2)) as i32 - VOXEL_HALO,
        }
    }
    pub(crate) fn linear(self) -> usize {
        assert!(
            self.in_chunk(),
            "sample index must be within the chunk and halo"
        );
        ((self.z + VOXEL_HALO) as usize * VOXEL_SAMPLE_SIDE + (self.y + VOXEL_HALO) as usize)
            * VOXEL_SAMPLE_SIDE
            + (self.x + VOXEL_HALO) as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoxelAddressError {
    Lod,
    Index,
    Position,
}
impl fmt::Display for VoxelAddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lod => write!(f, "voxel LOD must be in 0..={VOXEL_ROOT_LOD}"),
            Self::Index => write!(f, "chunk index is outside its octree level"),
            Self::Position => write!(
                f,
                "voxel position must be in [-{VOXEL_WORLD_HALF_EXTENT_M}, {VOXEL_WORLD_HALF_EXTENT_M}) meters"
            ),
        }
    }
}
impl Error for VoxelAddressError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct VoxelChunkAddress {
    lod: u8,
    index: ChunkIndex,
}
impl VoxelChunkAddress {
    pub const fn root() -> Self {
        Self {
            lod: VOXEL_ROOT_LOD,
            index: ChunkIndex { x: 0, y: 0, z: 0 },
        }
    }
    pub fn new(lod: u8, index: ChunkIndex) -> Result<Self, VoxelAddressError> {
        if lod > VOXEL_ROOT_LOD {
            return Err(VoxelAddressError::Lod);
        }
        let side = 1_u32 << (VOXEL_ROOT_LOD - lod);
        if [index.x, index.y, index.z].into_iter().any(|v| v >= side) {
            return Err(VoxelAddressError::Index);
        }
        Ok(Self { lod, index })
    }
    pub fn containing(position: VoxelPosition, lod: u8) -> Result<Self, VoxelAddressError> {
        if lod > VOXEL_ROOT_LOD {
            return Err(VoxelAddressError::Lod);
        }
        if [position.x_m, position.y_m, position.z_m]
            .into_iter()
            .any(|v| !(-VOXEL_WORLD_HALF_EXTENT_M..VOXEL_WORLD_HALF_EXTENT_M).contains(&v))
        {
            return Err(VoxelAddressError::Position);
        }
        let span = VOXEL_CHUNK_CELLS << lod;
        Self::new(
            lod,
            ChunkIndex {
                x: ((position.x_m + VOXEL_WORLD_HALF_EXTENT_M) / span) as u32,
                y: ((position.y_m + VOXEL_WORLD_HALF_EXTENT_M) / span) as u32,
                z: ((position.z_m + VOXEL_WORLD_HALF_EXTENT_M) / span) as u32,
            },
        )
    }
    pub fn lod(self) -> u8 {
        self.lod
    }
    pub fn index(self) -> ChunkIndex {
        self.index
    }
    pub fn spacing_m(self) -> i32 {
        1 << self.lod
    }
    pub fn span_m(self) -> i32 {
        VOXEL_CHUNK_CELLS << self.lod
    }
    pub fn origin(self) -> VoxelPosition {
        let coordinate = |index: u32| index as i32 * self.span_m() - VOXEL_WORLD_HALF_EXTENT_M;
        VoxelPosition {
            x_m: coordinate(self.index.x),
            y_m: coordinate(self.index.y),
            z_m: coordinate(self.index.z),
        }
    }
    pub fn parent(self) -> Option<Self> {
        (self.lod < VOXEL_ROOT_LOD).then(|| Self {
            lod: self.lod + 1,
            index: ChunkIndex {
                x: self.index.x / 2,
                y: self.index.y / 2,
                z: self.index.z / 2,
            },
        })
    }
    /// Children have half the cell spacing. X varies fastest, then Y, then Z.
    pub fn children(self) -> Option<[Self; 8]> {
        (self.lod > 0).then(|| {
            std::array::from_fn(|i| Self {
                lod: self.lod - 1,
                index: ChunkIndex {
                    x: self.index.x * 2 + (i as u32 & 1),
                    y: self.index.y * 2 + ((i as u32 >> 1) & 1),
                    z: self.index.z * 2 + ((i as u32 >> 2) & 1),
                },
            })
        })
    }
    pub fn sample_position(self, index: VoxelSampleIndex) -> VoxelPosition {
        assert!(
            index.in_chunk(),
            "sample index must be within the chunk and halo"
        );
        let origin = self.origin();
        let step = self.spacing_m();
        VoxelPosition {
            x_m: origin.x_m + index.x * step,
            y_m: origin.y_m + index.y * step,
            z_m: origin.z_m + index.z * step,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hierarchy_partitions_root_and_assigns_negative_boundaries() {
        let mut parent = VoxelChunkAddress::root();
        assert!(parent.parent().is_none());
        while let Some(children) = parent.children() {
            for child in children {
                assert_eq!(child.parent(), Some(parent));
                assert_eq!(child.span_m() * 2, parent.span_m());
                assert_eq!(
                    VoxelChunkAddress::containing(child.origin(), child.lod()).unwrap(),
                    child
                );
            }
            parent = children[7];
        }
        assert_eq!(parent.spacing_m(), 1);
        for x in [
            -VOXEL_WORLD_HALF_EXTENT_M,
            -33,
            -32,
            -1,
            0,
            31,
            32,
            VOXEL_WORLD_HALF_EXTENT_M - 1,
        ] {
            let p = VoxelPosition {
                x_m: x,
                y_m: 0,
                z_m: 0,
            };
            let a = VoxelChunkAddress::containing(p, 0).unwrap();
            assert!(a.origin().x_m <= x && x < a.origin().x_m + a.span_m());
        }
        assert!(
            VoxelChunkAddress::containing(
                VoxelPosition {
                    x_m: VOXEL_WORLD_HALF_EXTENT_M,
                    y_m: 0,
                    z_m: 0
                },
                0
            )
            .is_err()
        );
    }
    #[test]
    fn shared_faces_edges_corners_halos_and_parent_nodes_have_one_integer_address() {
        for lod in [0, 1, 8, VOXEL_ROOT_LOD - 1] {
            let a = VoxelChunkAddress::new(lod, ChunkIndex { x: 0, y: 0, z: 0 }).unwrap();
            for bits in 1..8 {
                let b = VoxelChunkAddress::new(
                    lod,
                    ChunkIndex {
                        x: bits & 1,
                        y: (bits >> 1) & 1,
                        z: (bits >> 2) & 1,
                    },
                )
                .unwrap();
                for offset in [-1, 0, 1] {
                    let point = VoxelSampleIndex {
                        x: 32 + offset,
                        y: 32 + offset,
                        z: 32 + offset,
                    };
                    let corresponding = VoxelSampleIndex {
                        x: point.x - b.index.x as i32 * 32,
                        y: point.y - b.index.y as i32 * 32,
                        z: point.z - b.index.z as i32 * 32,
                    };
                    assert_eq!(a.sample_position(point), b.sample_position(corresponding));
                }
            }
            let parent = a.parent().unwrap();
            assert_eq!(
                a.sample_position(VoxelSampleIndex {
                    x: 32,
                    y: 32,
                    z: 32
                }),
                parent.sample_position(VoxelSampleIndex {
                    x: 16,
                    y: 16,
                    z: 16
                })
            );
        }
    }
    #[test]
    fn integer_addresses_have_a_pinned_cross_lod_fingerprint() {
        let words = (0..64).flat_map(|i| {
            let coordinate = |k| (i * k) % 16_777_216 - 8_388_608;
            let address = VoxelChunkAddress::containing(
                VoxelPosition {
                    x_m: coordinate(127_901),
                    y_m: coordinate(314_159),
                    z_m: coordinate(271_829),
                },
                (i % 20) as u8,
            )
            .unwrap();
            let index = address.index();
            let p = address.sample_position(VoxelSampleIndex {
                x: -1,
                y: 17,
                z: 33,
            });
            [
                address.lod() as u64,
                index.x as u64,
                index.y as u64,
                index.z as u64,
                p.x_m as u32 as u64,
                p.y_m as u32 as u64,
                p.z_m as u32 as u64,
            ]
        });
        assert_eq!(procgen_core::fingerprint(words), 2109539101999377449);
    }

    #[test]
    fn far_origin_keeps_unit_steps_exact_after_rebasing() {
        let a = VoxelChunkAddress::containing(
            VoxelPosition {
                x_m: 8_000_000,
                y_m: -7_000_000,
                z_m: 6_000_000,
            },
            0,
        )
        .unwrap();
        for i in 0..=32 {
            let point = a.sample_position(VoxelSampleIndex { x: i, y: i, z: i });
            assert_eq!(
                point.relative_to(a.origin()).as_vec3(),
                Vec3::new(i as f32, i as f32, i as f32)
            );
        }
    }
}
