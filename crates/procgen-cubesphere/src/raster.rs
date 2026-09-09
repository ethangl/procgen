//! Integer cell addressing and adjacency for cube-sphere rasters.

use crate::mapping::{
    CubeFace, FaceCoordinates, FaceEdge, equiangular_tangent, seam_neighbor, unit_direction,
};
use procgen_core::Vec3;
use std::{f32::consts::FRAC_PI_4, fmt};

/// Axis-link length in the raster's integer chamfer metric.
pub const AXIS_LINK_LENGTH: u32 = 5;
/// Diagonal-link length in the raster's integer chamfer metric.
pub const DIAGONAL_LINK_LENGTH: u32 = 7;
/// Border edges each cell carries: its four axis links, in [`TexelLink`] order.
pub const BORDER_LINKS_PER_CELL: u32 = 4;
/// Largest power-of-two face resolution whose six-face cell count fits in `u32`.
pub const MAX_RASTER_RESOLUTION: u32 = 1 << 14;
/// Largest face resolution whose border-edge ids fit in `u32`, since a raster
/// addresses [`BORDER_LINKS_PER_CELL`] of them per cell.
pub const MAX_BORDER_RESOLUTION: u32 = MAX_RASTER_RESOLUTION / 2;
/// Sentinel returned by WGSL when a cube-corner diagonal has no neighbor.
pub const NO_RASTER_CELL: u32 = u32::MAX;

/// Relative Rust/WGSL tolerance for [`FaceTexel::solid_angle`].
///
/// Every other raster quantity is integer or exact composition, so the mirrors
/// agree bit for bit. The area element divides by a square root, which the two
/// compilers round differently. On 2026-09-08,
/// `wgsl_raster_adjacency_and_area_match_rust_across_every_seam_and_corner`
/// measured a maximum relative divergence of `3.8344808e-7` at 8 through 256
/// texels per face on Apple M1 Max via Metal. The tolerance is ten times that
/// maximum, rounded upward, and awaits Vulkan calibration.
pub const TEXEL_SOLID_ANGLE_TOLERANCE: f32 = 4.0e-6;

const _: () =
    assert!(6_u64 * MAX_RASTER_RESOLUTION as u64 * MAX_RASTER_RESOLUTION as u64 <= u32::MAX as u64);
const _: () = assert!(
    BORDER_LINKS_PER_CELL as u64 * 6 * MAX_BORDER_RESOLUTION as u64 * MAX_BORDER_RESOLUTION as u64
        <= u32::MAX as u64
);

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

    /// Returns the unit direction of this texel's center.
    ///
    /// The equi-angular mapping spaces texel centers evenly in angle, so this
    /// is the cell center of the raster. `cubesphere_texel_direction` mirrors
    /// the same expression order in WGSL.
    pub fn center_direction(self) -> Vec3 {
        unit_direction(FaceCoordinates {
            face: self.face,
            u: texel_center(self.x, self.resolution),
            v: texel_center(self.y, self.resolution),
        })
    }

    /// Returns the face-major, raster-scan cell id.
    pub fn cell_id(self) -> u32 {
        self.face.index() as u32 * self.resolution * self.resolution
            + self.y * self.resolution
            + self.x
    }

    /// Returns the solid angle this texel covers, in steradians.
    ///
    /// The equi-angular mapping spaces texel centers evenly in angle, so a
    /// texel's solid angle is the gnomonic area element at its center over the
    /// angular width of one texel. That is a midpoint approximation of the
    /// exact spherical quadrilateral, whose closed form needs an arc tangent;
    /// this expression uses the polynomial tangent and arithmetic only, so it
    /// is the same value everywhere. `cubesphere_texel_solid_angle` mirrors its
    /// expression order in WGSL.
    pub fn solid_angle(self) -> f32 {
        let a = equiangular_tangent(texel_center(self.x, self.resolution));
        let b = equiangular_tangent(texel_center(self.y, self.resolution));
        let width = 2.0 * FRAC_PI_4 / self.resolution as f32;
        let squared = 1.0 + a * a + b * b;
        width * width * (1.0 + a * a) * (1.0 + b * b) / (squared * squared.sqrt())
    }

    /// Returns the canonical id of the border edge `link` crosses.
    ///
    /// Both cells of a border resolve to the same id. The lower cell id owns
    /// the border and the id is [`BORDER_LINKS_PER_CELL`] times the owner plus
    /// the owner's own direction toward the other cell, so every border-edge id
    /// is a pure function of texel coordinates. Seams may rotate which
    /// direction that is, which is why the owner's link is searched for rather
    /// than assumed opposite.
    ///
    /// # Panics
    ///
    /// Panics unless `link` is a border link and the raster's resolution is at
    /// most [`MAX_BORDER_RESOLUTION`].
    pub fn border_edge(self, link: TexelLink) -> u32 {
        assert!(link.is_border(), "border edges cross the axis links");
        assert!(
            self.resolution <= MAX_BORDER_RESOLUTION,
            "border-edge ids are addressable up to {MAX_BORDER_RESOLUTION} texels per face"
        );
        let neighbor = self
            .neighbor(link)
            .expect("every border link has a neighbor");
        let cell = self.cell_id();
        if cell < neighbor.cell_id() {
            return BORDER_LINKS_PER_CELL * cell + link.index();
        }
        BORDER_LINKS_PER_CELL * neighbor.cell_id() + self.link_back(link).index()
    }

    /// Returns the border link the neighbor across `link` traverses to reach
    /// this texel.
    ///
    /// Border adjacency is reciprocal, so exactly one of the neighbor's border
    /// links leads back. A seam can rotate which one, so it is searched for
    /// rather than taken as the opposite direction.
    ///
    /// # Panics
    ///
    /// Panics unless `link` is a border link.
    pub fn link_back(self, link: TexelLink) -> TexelLink {
        assert!(link.is_border(), "only border links are reciprocal");
        let neighbor = self
            .neighbor(link)
            .expect("every border link has a neighbor");
        TexelLink::BORDERS
            .into_iter()
            .find(|&back| neighbor.neighbor(back) == Some(self))
            .expect("border adjacency is reciprocal")
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

    /// The border links, which are the four sides a cell shares with a
    /// neighbor. Stages that act across a physical boundary use these; stages
    /// that grow, flood, or measure distance use all of [`TexelLink::ALL`].
    pub const BORDERS: [Self; BORDER_LINKS_PER_CELL as usize] =
        [Self::Left, Self::Right, Self::Bottom, Self::Top];

    pub const fn index(self) -> u32 {
        self as u32
    }

    pub const fn is_border(self) -> bool {
        matches!(self, Self::Left | Self::Right | Self::Bottom | Self::Top)
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

fn texel_center(index: u32, resolution: u32) -> f32 {
    -1.0 + (2 * index + 1) as f32 / resolution as f32
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
    fn texel_centers_are_unit_length_and_reproject_onto_their_own_texel() {
        let resolution = 16;
        for cell_id in 0..FaceTexel::cell_count(resolution).unwrap() {
            let texel = FaceTexel::from_cell_id(cell_id, resolution).unwrap();
            let direction = texel.center_direction();
            assert!((direction.length() - 1.0).abs() <= f32::EPSILON);
            let coordinates = crate::direction_to_face(direction).unwrap();
            let reprojected = FaceTexel::new(
                coordinates.face,
                resolution,
                ((coordinates.u + 1.0) * 0.5 * resolution as f32) as u32,
                ((coordinates.v + 1.0) * 0.5 * resolution as f32) as u32,
            )
            .unwrap();
            assert_eq!(reprojected, texel);
        }
    }

    #[test]
    fn border_edges_are_shared_by_both_cells_and_number_two_per_cell() {
        let resolution = 8;
        let cell_count = FaceTexel::cell_count(resolution).unwrap();
        let mut owners = std::collections::HashMap::new();
        for cell_id in 0..cell_count {
            let texel = FaceTexel::from_cell_id(cell_id, resolution).unwrap();
            for link in TexelLink::BORDERS {
                let neighbor = texel.neighbor(link).unwrap();
                let edge = texel.border_edge(link);
                assert_eq!(
                    edge,
                    neighbor.border_edge(texel.link_back(link)),
                    "{texel:?} and {neighbor:?} disagree on their shared border"
                );
                owners
                    .entry(edge)
                    .or_insert_with(Vec::new)
                    .push(texel.cell_id());
            }
        }
        assert_eq!(owners.len() as u32, 2 * cell_count);
        assert!(owners.values().all(|cells| cells.len() == 2));
        assert!(owners.keys().all(|&edge| edge < 4 * cell_count));
    }

    #[test]
    fn texel_solid_angles_cover_the_sphere() {
        for resolution in [1, 8, 64] {
            let total: f32 = (0..FaceTexel::cell_count(resolution).unwrap())
                .map(|cell_id| {
                    FaceTexel::from_cell_id(cell_id, resolution)
                        .unwrap()
                        .solid_angle()
                })
                .sum();
            // The midpoint rule converges on the sphere from below as the
            // texels shrink; one texel per face is the coarsest it ever is.
            let tolerance = 4.0 / (resolution * resolution) as f32;
            assert!(
                (total - 4.0 * std::f32::consts::PI).abs() < tolerance,
                "{resolution} texels per face covered {total} steradians"
            );
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
