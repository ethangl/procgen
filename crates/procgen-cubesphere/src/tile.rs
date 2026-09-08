//! Power-of-two quadtree tile addressing on cube faces.

use crate::mapping::{CubeFace, FaceCoordinates, FaceEdge, seam_neighbor, unit_direction};
use procgen_core::Vec3;
use std::fmt;

/// The number of quads along one side of a terrain tile.
pub const TILE_QUADS: u32 = 64;
/// The number of vertices along one side of a terrain tile.
pub const TILE_VERTICES: u32 = TILE_QUADS + 1;
/// The finest level at which every face grid coordinate, up to
/// `TILE_QUADS << level`, converts to `f32` exactly.
///
/// Beyond this the canonical `-1 + 2 * numerator / quads_per_axis` quantization
/// would round its integer inputs, and the numerators would no longer fit the
/// `u32` arithmetic that shader backends mirror.
pub const MAX_TILE_LEVEL: u8 = 18;

const _: () = assert!((TILE_QUADS as u64) << MAX_TILE_LEVEL <= 1 << f32::MANTISSA_DIGITS);

/// Nominal angular spacing between adjacent tile vertices at face center.
pub fn vertex_spacing(level: u8) -> f32 {
    assert!(level <= MAX_TILE_LEVEL, "tile level must be valid");
    std::f32::consts::FRAC_PI_2 / (TILE_QUADS << level) as f32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileError {
    LevelTooLarge,
    TileOutOfLevel,
    VertexOutOfTile,
}

impl fmt::Display for TileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LevelTooLarge => write!(formatter, "tile level must not exceed {MAX_TILE_LEVEL}"),
            Self::TileOutOfLevel => formatter.write_str("tile coordinates are outside their level"),
            Self::VertexOutOfTile => write!(
                formatter,
                "tile-local vertex coordinates must be between 0 and {TILE_QUADS}"
            ),
        }
    }
}

impl std::error::Error for TileError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TileAddress {
    face: CubeFace,
    level: u8,
    x: u32,
    y: u32,
}

impl TileAddress {
    pub const fn root(face: CubeFace) -> Self {
        Self {
            face,
            level: 0,
            x: 0,
            y: 0,
        }
    }

    pub fn new(face: CubeFace, level: u8, x: u32, y: u32) -> Result<Self, TileError> {
        if level > MAX_TILE_LEVEL {
            return Err(TileError::LevelTooLarge);
        }
        let tiles_per_axis = 1_u32 << level;
        if x >= tiles_per_axis || y >= tiles_per_axis {
            return Err(TileError::TileOutOfLevel);
        }
        Ok(Self { face, level, x, y })
    }

    pub const fn face(self) -> CubeFace {
        self.face
    }

    pub const fn level(self) -> u8 {
        self.level
    }

    pub const fn x(self) -> u32 {
        self.x
    }

    pub const fn y(self) -> u32 {
        self.y
    }

    /// Encodes the storage-buffer words decoded by `cubesphere_tile_direction`.
    pub const fn gpu_words(self) -> [u32; 4] {
        [self.face.index() as u32, self.level as u32, self.x, self.y]
    }

    pub fn parent(self) -> Option<Self> {
        (self.level > 0).then(|| Self {
            face: self.face,
            level: self.level - 1,
            x: self.x / 2,
            y: self.y / 2,
        })
    }

    pub fn child(self, quadrant: TileQuadrant) -> Option<Self> {
        let (x_offset, y_offset) = quadrant.offsets();
        (self.level < MAX_TILE_LEVEL).then(|| Self {
            face: self.face,
            level: self.level + 1,
            x: self.x * 2 + x_offset,
            y: self.y * 2 + y_offset,
        })
    }

    pub fn children(self) -> Option<[Self; 4]> {
        (self.level < MAX_TILE_LEVEL)
            .then(|| TileQuadrant::ALL.map(|quadrant| self.child(quadrant).unwrap()))
    }

    /// Returns the same-level tile across one edge, including cube-face seams.
    pub fn edge_neighbor(self, edge: FaceEdge) -> Self {
        let tiles_per_axis = 1_u32 << self.level;
        match edge {
            FaceEdge::Left if self.x > 0 => Self {
                x: self.x - 1,
                ..self
            },
            FaceEdge::Right if self.x + 1 < tiles_per_axis => Self {
                x: self.x + 1,
                ..self
            },
            FaceEdge::Bottom if self.y > 0 => Self {
                y: self.y - 1,
                ..self
            },
            FaceEdge::Top if self.y + 1 < tiles_per_axis => Self {
                y: self.y + 1,
                ..self
            },
            _ => {
                let (face, x, y) = seam_neighbor(
                    self.face,
                    edge,
                    edge.along(self.x, self.y),
                    tiles_per_axis - 1,
                );
                Self { face, x, y, ..self }
            }
        }
    }

    pub fn is_ancestor_of(self, mut descendant: Self) -> bool {
        if self.face != descendant.face || self.level > descendant.level {
            return false;
        }
        while descendant.level > self.level {
            descendant = descendant.parent().unwrap();
        }
        self == descendant
    }

    /// Converts a tile-local vertex into its exact integer address on the face.
    ///
    /// Adjacent tiles therefore produce the same integer numerator for a
    /// shared vertex instead of reaching it through accumulated float steps.
    /// [`FaceGridVertex::face_coordinates`] then performs the one canonical
    /// f32 conversion, `-1 + 2 * numerator / quads_per_axis`, in that order.
    pub fn grid_vertex(self, local_x: u32, local_y: u32) -> Result<FaceGridVertex, TileError> {
        if local_x >= TILE_VERTICES || local_y >= TILE_VERTICES {
            return Err(TileError::VertexOutOfTile);
        }
        Ok(FaceGridVertex {
            face: self.face,
            level: self.level,
            x: self.x * TILE_QUADS + local_x,
            y: self.y * TILE_QUADS + local_y,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TileQuadrant {
    LowerLeft,
    LowerRight,
    UpperLeft,
    UpperRight,
}

impl TileQuadrant {
    pub const ALL: [Self; 4] = [
        Self::LowerLeft,
        Self::LowerRight,
        Self::UpperLeft,
        Self::UpperRight,
    ];

    const fn offsets(self) -> (u32, u32) {
        match self {
            Self::LowerLeft => (0, 0),
            Self::LowerRight => (1, 0),
            Self::UpperLeft => (0, 1),
            Self::UpperRight => (1, 1),
        }
    }
}

/// An exact integer vertex position on a face's level grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FaceGridVertex {
    face: CubeFace,
    level: u8,
    x: u32,
    y: u32,
}

impl FaceGridVertex {
    pub const fn face(self) -> CubeFace {
        self.face
    }

    pub const fn level(self) -> u8 {
        self.level
    }

    pub const fn x(self) -> u32 {
        self.x
    }

    pub const fn y(self) -> u32 {
        self.y
    }

    pub const fn quads_per_axis(self) -> u32 {
        TILE_QUADS << self.level
    }

    pub fn face_coordinates(self) -> FaceCoordinates {
        let denominator = self.quads_per_axis() as f32;
        FaceCoordinates {
            face: self.face,
            u: -1.0 + 2.0 * self.x as f32 / denominator,
            v: -1.0 + 2.0 * self.y as f32 / denominator,
        }
    }

    pub fn direction(self) -> Vec3 {
        unit_direction(self.face_coordinates())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_addresses_validate_bounds() {
        assert_eq!(
            TileAddress::root(CubeFace::PositiveX),
            TileAddress::new(CubeFace::PositiveX, 0, 0, 0).unwrap()
        );
        let edge = (1_u32 << MAX_TILE_LEVEL) - 1;
        assert!(TileAddress::new(CubeFace::NegativeZ, MAX_TILE_LEVEL, edge, edge).is_ok());
        assert_eq!(
            TileAddress::new(CubeFace::PositiveX, 2, 4, 0),
            Err(TileError::TileOutOfLevel)
        );
        assert_eq!(
            TileAddress::new(CubeFace::PositiveX, MAX_TILE_LEVEL + 1, 0, 0),
            Err(TileError::LevelTooLarge)
        );
    }

    #[test]
    fn gpu_words_follow_the_mapping_shader_contract() {
        let address = TileAddress::new(CubeFace::NegativeY, 4, 11, 7).unwrap();
        assert_eq!(
            address.gpu_words(),
            [CubeFace::NegativeY.index() as u32, 4, 11, 7]
        );
    }

    #[test]
    fn parent_and_children_preserve_quadtree_relationships() {
        let parent = TileAddress::new(CubeFace::PositiveY, 3, 5, 2).unwrap();
        for (quadrant, x, y) in [
            (TileQuadrant::LowerLeft, 10, 4),
            (TileQuadrant::LowerRight, 11, 4),
            (TileQuadrant::UpperLeft, 10, 5),
            (TileQuadrant::UpperRight, 11, 5),
        ] {
            let child = parent.child(quadrant).unwrap();
            assert_eq!((child.x(), child.y()), (x, y));
            assert_eq!(child.parent(), Some(parent));
        }
        assert_eq!(parent.parent().unwrap().level(), 2);
        assert!(
            TileAddress::new(CubeFace::PositiveY, 0, 0, 0)
                .unwrap()
                .parent()
                .is_none()
        );
        assert!(
            TileAddress::new(CubeFace::PositiveY, MAX_TILE_LEVEL, 0, 0)
                .unwrap()
                .child(TileQuadrant::LowerLeft)
                .is_none()
        );
        assert_eq!(
            parent.children().unwrap().map(|child| child.parent()),
            [Some(parent); 4]
        );
        assert!(parent.is_ancestor_of(parent));
        assert!(parent.is_ancestor_of(parent.children().unwrap()[3]));
        assert!(!parent.is_ancestor_of(TileAddress::root(CubeFace::NegativeY)));
    }

    #[test]
    fn edge_neighbors_are_exact_and_reciprocal_at_every_level() {
        for level in [0, 1, 8, 12, MAX_TILE_LEVEL] {
            let edge = (1_u32 << level) - 1;
            for face in CubeFace::ALL {
                for tile in [
                    TileAddress::new(face, level, 0, 0).unwrap(),
                    TileAddress::new(face, level, edge, edge).unwrap(),
                ] {
                    for side in FaceEdge::ALL {
                        let neighbor = tile.edge_neighbor(side);
                        assert_eq!(neighbor.level(), level);
                        assert!(
                            FaceEdge::ALL
                                .into_iter()
                                .any(|neighbor_side| neighbor.edge_neighbor(neighbor_side) == tile)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn nominal_vertex_spacing_halves_with_each_level() {
        assert_eq!(
            vertex_spacing(0),
            std::f32::consts::FRAC_PI_2 / TILE_QUADS as f32
        );
        for level in 1..=MAX_TILE_LEVEL {
            assert_eq!(vertex_spacing(level), vertex_spacing(level - 1) * 0.5);
        }
    }

    #[test]
    fn tile_vertices_follow_the_settled_grid_contract() {
        let tile = TileAddress::new(CubeFace::PositiveZ, 4, 7, 9).unwrap();
        let first = tile.grid_vertex(0, 0).unwrap();
        let last = tile.grid_vertex(TILE_QUADS, TILE_QUADS).unwrap();
        assert_eq!((first.x(), first.y()), (7 * 64, 9 * 64));
        assert_eq!((last.x(), last.y()), (8 * 64, 10 * 64));
        assert_eq!(first.quads_per_axis(), 64 * 16);
        assert_eq!(
            tile.grid_vertex(TILE_VERTICES, 0),
            Err(TileError::VertexOutOfTile)
        );
    }

    #[test]
    fn same_level_neighbors_share_exact_vertex_inputs() {
        let left = TileAddress::new(CubeFace::PositiveZ, 12, 1_999, 781).unwrap();
        let right = TileAddress::new(CubeFace::PositiveZ, 12, 2_000, 781).unwrap();
        for local_y in 0..TILE_VERTICES {
            let from_left = left.grid_vertex(TILE_QUADS, local_y).unwrap();
            let from_right = right.grid_vertex(0, local_y).unwrap();
            assert_eq!(from_left, from_right);
            assert_eq!(from_left.face_coordinates(), from_right.face_coordinates());
            assert_eq!(from_left.direction(), from_right.direction());
        }
    }

    #[test]
    fn finest_level_corner_tile_quantizes_exactly() {
        // At MAX_TILE_LEVEL the corner-most tile's numerators reach 2^24; every one
        // must survive the u32 -> f32 conversion bit-exactly. One level finer would not.
        let edge = (1_u32 << MAX_TILE_LEVEL) - 1;
        let tile = TileAddress::new(CubeFace::PositiveX, MAX_TILE_LEVEL, edge, edge).unwrap();
        for local in 0..TILE_VERTICES {
            let vertex = tile.grid_vertex(local, local).unwrap();
            assert_eq!(vertex.x() as f32 as u32, vertex.x());
            let expected_u = -1.0 + 2.0 * (vertex.x() as f64) / (vertex.quads_per_axis() as f64);
            assert_eq!(vertex.face_coordinates().u as f64, expected_u);
        }
        let one_finer = (TILE_QUADS as u64) << (MAX_TILE_LEVEL + 1);
        assert_ne!(
            (one_finer - 1) as f32 as u64,
            one_finer - 1,
            "level 19 numerators round in f32"
        );
    }
}
