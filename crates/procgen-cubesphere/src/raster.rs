//! Integer cell addressing and adjacency for cube-sphere rasters.

use crate::mapping::{CubeFace, FaceEdge, seam_neighbor};
use std::fmt;

/// Axis-link length in the raster's integer chamfer metric.
pub const AXIS_LINK_LENGTH: u32 = 5;
/// Diagonal-link length in the raster's integer chamfer metric.
pub const DIAGONAL_LINK_LENGTH: u32 = 7;
/// Largest power-of-two face resolution whose six-face cell count fits in `u32`.
pub const MAX_RASTER_RESOLUTION: u32 = 1 << 14;
/// Sentinel returned by WGSL when a cube-corner diagonal has no neighbor.
pub const NO_RASTER_CELL: u32 = u32::MAX;

const _: () =
    assert!(6_u64 * MAX_RASTER_RESOLUTION as u64 * MAX_RASTER_RESOLUTION as u64 <= u32::MAX as u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterError {
    InvalidResolution,
    TexelOutOfBounds,
    CellOutOfBounds,
}

impl fmt::Display for RasterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidResolution => write!(
                formatter,
                "raster face resolution must be a power of two between 1 and {MAX_RASTER_RESOLUTION}"
            ),
            Self::TexelOutOfBounds => {
                formatter.write_str("raster texel coordinates are outside their face")
            }
            Self::CellOutOfBounds => formatter.write_str("raster cell id is outside the cube"),
        }
    }
}

impl std::error::Error for RasterError {}

/// One texel address on a cube face at a fixed raster resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaceTexel {
    face: CubeFace,
    resolution: u32,
    x: u32,
    y: u32,
}

impl FaceTexel {
    pub fn new(face: CubeFace, resolution: u32, x: u32, y: u32) -> Result<Self, RasterError> {
        validate_resolution(resolution)?;
        if x >= resolution || y >= resolution {
            return Err(RasterError::TexelOutOfBounds);
        }
        Ok(Self {
            face,
            resolution,
            x,
            y,
        })
    }

    pub fn from_cell_id(cell_id: u32, resolution: u32) -> Result<Self, RasterError> {
        let cell_count = Self::cell_count(resolution)?;
        if cell_id >= cell_count {
            return Err(RasterError::CellOutOfBounds);
        }
        let face_size = resolution * resolution;
        let face = CubeFace::ALL[(cell_id / face_size) as usize];
        let local = cell_id % face_size;
        Ok(Self {
            face,
            resolution,
            x: local % resolution,
            y: local / resolution,
        })
    }

    pub const fn face(self) -> CubeFace {
        self.face
    }

    pub const fn resolution(self) -> u32 {
        self.resolution
    }

    pub const fn x(self) -> u32 {
        self.x
    }

    pub const fn y(self) -> u32 {
        self.y
    }

    /// Returns the validated cell count for a six-face raster.
    pub fn cell_count(resolution: u32) -> Result<u32, RasterError> {
        validate_resolution(resolution)?;
        Ok(6 * resolution * resolution)
    }

    /// Returns the face-major, raster-scan cell id.
    pub fn cell_id(self) -> u32 {
        self.face.index() as u32 * self.resolution * self.resolution
            + self.y * self.resolution
            + self.x
    }

    /// Resolves one of the eight raster links across cube seams.
    ///
    /// The outward diagonal at each cube corner has no unique adjacent cell,
    /// so that link returns `None`. At resolution one every texel is a cube
    /// corner and has the four border links; at larger resolutions corner
    /// texels have seven links. Every other link has exactly one neighbor.
    pub fn neighbor(self, link: TexelLink) -> Option<Self> {
        let (dx, dy) = link.offset();
        let x = self.x as i32 + dx;
        let y = self.y as i32 + dy;
        let resolution = self.resolution as i32;
        let x_outside = !(0..resolution).contains(&x);
        let y_outside = !(0..resolution).contains(&y);

        if !x_outside && !y_outside {
            return Some(Self {
                x: x as u32,
                y: y as u32,
                ..self
            });
        }
        if x_outside && y_outside {
            return None;
        }

        let (edge, along) = if x_outside {
            (
                if x < 0 {
                    FaceEdge::Left
                } else {
                    FaceEdge::Right
                },
                y as u32,
            )
        } else {
            (
                if y < 0 {
                    FaceEdge::Bottom
                } else {
                    FaceEdge::Top
                },
                x as u32,
            )
        };
        let (face, x, y) = seam_neighbor(self.face, edge, along, self.resolution - 1);
        Some(Self { face, x, y, ..self })
    }
}

/// Fixed raster-link order shared by Rust and WGSL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum TexelLink {
    Left = 0,
    Right = 1,
    Bottom = 2,
    Top = 3,
    BottomLeft = 4,
    BottomRight = 5,
    TopLeft = 6,
    TopRight = 7,
}

impl TexelLink {
    pub const ALL: [Self; 8] = [
        Self::Left,
        Self::Right,
        Self::Bottom,
        Self::Top,
        Self::BottomLeft,
        Self::BottomRight,
        Self::TopLeft,
        Self::TopRight,
    ];

    pub const fn index(self) -> u32 {
        self as u32
    }

    pub const fn link_length(self) -> u32 {
        match self {
            Self::Left | Self::Right | Self::Bottom | Self::Top => AXIS_LINK_LENGTH,
            Self::BottomLeft | Self::BottomRight | Self::TopLeft | Self::TopRight => {
                DIAGONAL_LINK_LENGTH
            }
        }
    }

    const fn offset(self) -> (i32, i32) {
        match self {
            Self::Left => (-1, 0),
            Self::Right => (1, 0),
            Self::Bottom => (0, -1),
            Self::Top => (0, 1),
            Self::BottomLeft => (-1, -1),
            Self::BottomRight => (1, -1),
            Self::TopLeft => (-1, 1),
            Self::TopRight => (1, 1),
        }
    }
}

pub(crate) fn validate_resolution(resolution: u32) -> Result<(), RasterError> {
    if resolution == 0 || resolution > MAX_RASTER_RESOLUTION || !resolution.is_power_of_two() {
        return Err(RasterError::InvalidResolution);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_ids_round_trip_in_face_major_raster_order() {
        for resolution in [1, 8, 1_024, MAX_RASTER_RESOLUTION] {
            let face_size = resolution * resolution;
            for face in CubeFace::ALL {
                for (x, y) in [
                    (0, 0),
                    (resolution - 1, 0),
                    (0, resolution - 1),
                    (resolution - 1, resolution - 1),
                ] {
                    let texel = FaceTexel::new(face, resolution, x, y).unwrap();
                    assert_eq!(
                        texel.cell_id(),
                        face.index() as u32 * face_size + y * resolution + x
                    );
                    assert_eq!(
                        FaceTexel::from_cell_id(texel.cell_id(), resolution).unwrap(),
                        texel
                    );
                }
            }
        }
    }

    #[test]
    fn seam_neighbors_are_reciprocal_and_unique() {
        let resolution = 8;
        for face in CubeFace::ALL {
            for y in 0..resolution {
                for x in 0..resolution {
                    let texel = FaceTexel::new(face, resolution, x, y).unwrap();
                    let neighbors: Vec<_> = TexelLink::ALL
                        .into_iter()
                        .filter_map(|link| texel.neighbor(link))
                        .collect();
                    let corner = (x == 0 || x + 1 == resolution) && (y == 0 || y + 1 == resolution);
                    assert_eq!(neighbors.len(), if corner { 7 } else { 8 });
                    for (index, neighbor) in neighbors.iter().enumerate() {
                        assert!(!neighbors[..index].contains(neighbor));
                        assert!(
                            TexelLink::ALL
                                .into_iter()
                                .any(|link| neighbor.neighbor(link) == Some(texel))
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn resolution_one_has_four_border_neighbors() {
        for face in CubeFace::ALL {
            let texel = FaceTexel::new(face, 1, 0, 0).unwrap();
            let neighbors: Vec<_> = TexelLink::ALL
                .into_iter()
                .filter_map(|link| texel.neighbor(link))
                .collect();
            assert_eq!(neighbors.len(), 4);
        }
    }
}
