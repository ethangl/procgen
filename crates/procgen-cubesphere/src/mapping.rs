//! Equi-angular mapping between cube-face coordinates and unit directions.
//!
//! Face coordinates use `u` from left to right and `v` from bottom to top,
//! both in `[-1, 1]`. Each face's right-handed frame is defined once in
//! [`CubeFace::frame`], and both mapping directions read from it.
//!
//! Direction-to-face ties are resolved by axis priority X, then Y, then Z;
//! the sign of the selected component chooses the positive or negative face.

use procgen_core::Vec3;
use std::f32::consts::FRAC_PI_4;
use std::fmt;

/// Stable cross-backend vectors for the polynomial equi-angular tangent.
pub const EQUIANGULAR_TANGENT_TEST_VECTORS: [(f32, u32); 17] = [
    (-1.0, 0xbf80_0000),
    (-0.875, 0xbf52_17f1),
    (-0.75, 0xbf2b_0db5),
    (-0.625, 0xbf08_d5cf),
    (-0.5, 0xbed4_13cc),
    (-0.375, 0xbe9b_5023),
    (-0.25, 0xbe4b_afb0),
    (-0.125, 0xbdc9_b63c),
    (0.0, 0x0000_0000),
    (0.125, 0x3dc9_b63c),
    (0.25, 0x3e4b_afb0),
    (0.375, 0x3e9b_5023),
    (0.5, 0x3ed4_13cc),
    (0.625, 0x3f08_d5cf),
    (0.75, 0x3f2b_0db5),
    (0.875, 0x3f52_17f1),
    (1.0, 0x3f80_0000),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CubeFace {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}

/// The right-handed frame of one cube face.
///
/// `normal` points at the face center; `u_axis` and `v_axis` span the face
/// plane. All three are signed unit axes, so composing them is exact in `f32`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceFrame {
    pub normal: Vec3,
    pub u_axis: Vec3,
    pub v_axis: Vec3,
}

impl CubeFace {
    pub const ALL: [Self; 6] = [
        Self::PositiveX,
        Self::NegativeX,
        Self::PositiveY,
        Self::NegativeY,
        Self::PositiveZ,
        Self::NegativeZ,
    ];

    pub const fn index(self) -> usize {
        match self {
            Self::PositiveX => 0,
            Self::NegativeX => 1,
            Self::PositiveY => 2,
            Self::NegativeY => 3,
            Self::PositiveZ => 4,
            Self::NegativeZ => 5,
        }
    }

    pub(crate) fn from_normal(normal: Vec3) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|face| face.frame().normal == normal)
    }

    /// The single definition of every face's `(normal, +u, +v)` frame.
    pub fn frame(self) -> FaceFrame {
        let (x, y, z) = (Vec3::X, Vec3::Y, Vec3::Z);
        let (normal, u_axis, v_axis) = match self {
            Self::PositiveX => (x, -z, y),
            Self::NegativeX => (-x, z, y),
            Self::PositiveY => (y, x, -z),
            Self::NegativeY => (-y, x, z),
            Self::PositiveZ => (z, x, y),
            Self::NegativeZ => (-z, -x, y),
        };
        FaceFrame {
            normal,
            u_axis,
            v_axis,
        }
    }
}

/// Fixed cube-face edge order shared by tiles, rasters, and WGSL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum FaceEdge {
    Left = 0,
    Right = 1,
    Bottom = 2,
    Top = 3,
}

impl FaceEdge {
    pub const ALL: [Self; 4] = [Self::Left, Self::Right, Self::Bottom, Self::Top];

    pub const fn index(self) -> u32 {
        self as u32
    }

    pub(crate) const fn along(self, x: u32, y: u32) -> u32 {
        match self {
            Self::Left | Self::Right => y,
            Self::Bottom | Self::Top => x,
        }
    }
}

/// Maps one face-edge position to the adjacent face's coordinates.
pub(crate) fn seam_neighbor(
    face: CubeFace,
    edge: FaceEdge,
    along: u32,
    edge_index: u32,
) -> (CubeFace, u32, u32) {
    let source = face.frame();
    let (neighbor_normal, along_axis) = match edge {
        FaceEdge::Left => (-source.u_axis, source.v_axis),
        FaceEdge::Right => (source.u_axis, source.v_axis),
        FaceEdge::Bottom => (-source.v_axis, source.u_axis),
        FaceEdge::Top => (source.v_axis, source.u_axis),
    };
    let face = CubeFace::from_normal(neighbor_normal).expect("every cube axis has one face");
    let neighbor = face.frame();
    let normal_on_u = source.normal.dot(neighbor.u_axis).abs() > 0.5;
    let (fixed_axis, running_axis) = if normal_on_u {
        (neighbor.u_axis, neighbor.v_axis)
    } else {
        debug_assert!(source.normal.dot(neighbor.v_axis).abs() > 0.5);
        (neighbor.v_axis, neighbor.u_axis)
    };
    let fixed = if source.normal.dot(fixed_axis) > 0.5 {
        edge_index
    } else {
        0
    };
    let running = if along_axis.dot(running_axis) > 0.5 {
        along
    } else {
        debug_assert!(along_axis.dot(running_axis) < -0.5);
        edge_index - along
    };
    let (x, y) = if normal_on_u {
        (fixed, running)
    } else {
        (running, fixed)
    };
    (face, x, y)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceCoordinates {
    pub face: CubeFace,
    pub u: f32,
    pub v: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MappingError {
    NonFiniteCoordinates,
    CoordinatesOutOfBounds,
    NonFiniteDirection,
    ZeroDirection,
}

impl fmt::Display for MappingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteCoordinates => formatter.write_str("face coordinates must be finite"),
            Self::CoordinatesOutOfBounds => {
                formatter.write_str("face coordinates must be between -1 and 1")
            }
            Self::NonFiniteDirection => formatter.write_str("direction must be finite"),
            Self::ZeroDirection => formatter.write_str("direction must be nonzero"),
        }
    }
}

impl std::error::Error for MappingError {}

/// Maps equi-angular face coordinates to a unit direction.
pub fn face_to_direction(coordinates: FaceCoordinates) -> Result<Vec3, MappingError> {
    if !coordinates.u.is_finite() || !coordinates.v.is_finite() {
        return Err(MappingError::NonFiniteCoordinates);
    }
    if !(-1.0..=1.0).contains(&coordinates.u) || !(-1.0..=1.0).contains(&coordinates.v) {
        return Err(MappingError::CoordinatesOutOfBounds);
    }
    Ok(unit_direction(coordinates))
}

/// The mapping without validation. Coordinates normally lie on the selected
/// face, but may extend by one texel for seamless filtering across its edges.
pub(crate) fn unit_direction(coordinates: FaceCoordinates) -> Vec3 {
    let FaceFrame {
        normal,
        u_axis,
        v_axis,
    } = coordinates.face.frame();
    let a = equiangular_tangent(coordinates.u);
    let b = equiangular_tangent(coordinates.v);
    (normal + u_axis * a + v_axis * b).normalized()
}

/// Deterministically approximates `tan(PI / 4 * coordinate)` on `[-1, 1]`.
///
/// This is a degree-nine odd polynomial constrained to equal `-1`, `0`, and
/// `1` at the corresponding endpoints. Its coefficients are a least-squares
/// fit of `tan(PI / 4 * x) / x` over 200,001 uniformly spaced samples on
/// `[0, 1]`; the maximum absolute error is below `4e-6` on `[-1, 1]`. Its fixed
/// sequence of `f32` additions and multiplications is mirrored by
/// `cubesphere_equiangular_tangent` in WGSL, so transcendental implementations
/// cannot change raster directions.
pub fn equiangular_tangent(coordinate: f32) -> f32 {
    let squared = coordinate * coordinate;
    let mut correction = -0.005_166_28_f32;
    correction = correction * squared - 0.012_217_19;
    correction = correction * squared - 0.053_305_17;
    correction = correction * squared - 0.214_593_27;
    coordinate * (1.0 + (1.0 - squared) * correction)
}

/// Maps a nonzero direction to its deterministic dominant face and coordinates.
///
/// The input need not already have unit length.
pub fn direction_to_face(direction: Vec3) -> Result<FaceCoordinates, MappingError> {
    if !direction.is_finite() {
        return Err(MappingError::NonFiniteDirection);
    }
    if direction == Vec3::ZERO {
        return Err(MappingError::ZeroDirection);
    }

    Ok(canonical_face_coordinates(direction))
}

pub(crate) fn canonical_face_coordinates(direction: Vec3) -> FaceCoordinates {
    project_direction_onto_face(direction, dominant_face(direction))
}

pub(crate) fn project_direction_onto_face(direction: Vec3, face: CubeFace) -> FaceCoordinates {
    let FaceFrame {
        normal,
        u_axis,
        v_axis,
    } = face.frame();
    let depth = direction.dot(normal);
    FaceCoordinates {
        face,
        u: (direction.dot(u_axis) / depth).atan() / FRAC_PI_4,
        v: (direction.dot(v_axis) / depth).atan() / FRAC_PI_4,
    }
}

/// Derivatives of a face projection's `u` and `v` coordinates with respect to
/// the input direction. Kept beside [`project_direction_onto_face`] so the
/// equi-angular mapping and its Jacobian share one expression contract.
pub(crate) struct FaceCoordinateDerivatives {
    pub(crate) u: Vec3,
    pub(crate) v: Vec3,
}

pub(crate) fn face_coordinate_derivatives(
    direction: Vec3,
    face: CubeFace,
) -> FaceCoordinateDerivatives {
    let FaceFrame {
        normal,
        u_axis,
        v_axis,
    } = face.frame();
    let depth = direction.dot(normal);
    let side_u = direction.dot(u_axis);
    let side_v = direction.dot(v_axis);
    let inverse_depth_squared = (depth * depth).recip();
    let derivative = |axis: Vec3, side: f32| {
        let ratio = side / depth;
        (axis * depth - normal * side)
            * (inverse_depth_squared / (FRAC_PI_4 * (1.0 + ratio * ratio)))
    };
    FaceCoordinateDerivatives {
        u: derivative(u_axis, side_u),
        v: derivative(v_axis, side_v),
    }
}

fn dominant_face(direction: Vec3) -> CubeFace {
    let Vec3 { x, y, z } = direction;
    if x.abs() >= y.abs() && x.abs() >= z.abs() {
        if x >= 0.0 {
            CubeFace::PositiveX
        } else {
            CubeFace::NegativeX
        }
    } else if y.abs() >= z.abs() {
        if y >= 0.0 {
            CubeFace::PositiveY
        } else {
            CubeFace::NegativeY
        }
    } else if z >= 0.0 {
        CubeFace::PositiveZ
    } else {
        CubeFace::NegativeZ
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 2.0e-6;

    fn fc(face: CubeFace, u: f32, v: f32) -> FaceCoordinates {
        FaceCoordinates { face, u, v }
    }

    fn assert_direction_close(left: Vec3, right: Vec3) {
        assert!(
            (left - right).length() <= EPSILON,
            "left={left:?}, right={right:?}"
        );
    }

    #[test]
    fn face_mapping_round_trips_and_normalizes() {
        for face in CubeFace::ALL {
            for u in [-1.0, -0.75, 0.0, 0.25, 1.0] {
                for v in [-1.0, -0.4, 0.0, 0.6, 1.0] {
                    let direction = face_to_direction(fc(face, u, v)).unwrap();
                    assert!((direction.length() - 1.0).abs() <= f32::EPSILON);
                    let canonical = direction_to_face(direction).unwrap();
                    assert_direction_close(direction, face_to_direction(canonical).unwrap());
                }
            }
        }
    }

    #[test]
    fn polynomial_tangent_is_odd_exact_at_endpoints_and_close_to_tangent() {
        for (coordinate, expected_bits) in EQUIANGULAR_TANGENT_TEST_VECTORS {
            assert_eq!(equiangular_tangent(coordinate).to_bits(), expected_bits);
        }
        for step in -1_000..=1_000 {
            let coordinate = step as f32 / 1_000.0;
            let actual = equiangular_tangent(coordinate);
            let expected = (coordinate * FRAC_PI_4).tan();
            assert!((actual - expected).abs() <= 4.0e-6);
            assert_eq!(actual, -equiangular_tangent(-coordinate));
        }
        assert_eq!(equiangular_tangent(-1.0), -1.0);
        assert_eq!(equiangular_tangent(0.0), 0.0);
        assert_eq!(equiangular_tangent(1.0), 1.0);
    }

    #[test]
    fn face_indices_match_all_order() {
        for (index, face) in CubeFace::ALL.into_iter().enumerate() {
            assert_eq!(face.index(), index);
        }
    }

    #[test]
    fn rejects_invalid_mapping_inputs() {
        assert_eq!(
            face_to_direction(fc(CubeFace::PositiveX, f32::NAN, 0.0)),
            Err(MappingError::NonFiniteCoordinates)
        );
        assert_eq!(
            face_to_direction(fc(CubeFace::PositiveX, 1.01, 0.0)),
            Err(MappingError::CoordinatesOutOfBounds)
        );
        assert_eq!(
            direction_to_face(Vec3::ZERO),
            Err(MappingError::ZeroDirection)
        );
        assert_eq!(
            direction_to_face(Vec3::new(-0.0, 0.0, -0.0)),
            Err(MappingError::ZeroDirection)
        );
        assert_eq!(
            direction_to_face(Vec3::new(f32::INFINITY, 0.0, 0.0)),
            Err(MappingError::NonFiniteDirection)
        );
    }

    #[test]
    fn dominant_face_ties_use_x_then_y_then_z() {
        let sign = |component: f32, positive, negative| {
            if component > 0.0 { positive } else { negative }
        };

        for x in [-1.0, 1.0] {
            for y in [-1.0, 1.0] {
                for z in [-1.0, 1.0] {
                    assert_eq!(
                        direction_to_face(Vec3::new(x, y, z)).unwrap().face,
                        sign(x, CubeFace::PositiveX, CubeFace::NegativeX)
                    );
                }
            }
        }

        for x in [-1.0, 1.0] {
            for y in [-1.0, 1.0] {
                assert_eq!(
                    direction_to_face(Vec3::new(x, y, 0.0)).unwrap().face,
                    sign(x, CubeFace::PositiveX, CubeFace::NegativeX)
                );
            }
        }

        for y in [-1.0, 1.0] {
            for z in [-1.0, 1.0] {
                assert_eq!(
                    direction_to_face(Vec3::new(0.0, y, z)).unwrap().face,
                    sign(y, CubeFace::PositiveY, CubeFace::NegativeY)
                );
            }
        }
    }

    #[test]
    fn both_faces_produce_identical_directions_on_all_twelve_seams() {
        use CubeFace::*;
        for t in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            for (left, right) in [
                (fc(PositiveX, -1.0, t), fc(PositiveZ, 1.0, t)),
                (fc(PositiveX, 1.0, t), fc(NegativeZ, -1.0, t)),
                (fc(PositiveX, t, -1.0), fc(NegativeY, 1.0, -t)),
                (fc(PositiveX, t, 1.0), fc(PositiveY, 1.0, t)),
                (fc(NegativeX, -1.0, t), fc(NegativeZ, 1.0, t)),
                (fc(NegativeX, 1.0, t), fc(PositiveZ, -1.0, t)),
                (fc(NegativeX, t, -1.0), fc(NegativeY, -1.0, t)),
                (fc(NegativeX, t, 1.0), fc(PositiveY, -1.0, -t)),
                (fc(PositiveY, t, -1.0), fc(PositiveZ, t, 1.0)),
                (fc(PositiveY, t, 1.0), fc(NegativeZ, -t, 1.0)),
                (fc(NegativeY, t, -1.0), fc(NegativeZ, -t, -1.0)),
                (fc(NegativeY, t, 1.0), fc(PositiveZ, t, -1.0)),
            ] {
                assert_eq!(
                    face_to_direction(left).unwrap(),
                    face_to_direction(right).unwrap(),
                    "left={left:?}, right={right:?}"
                );
            }
        }
    }

    #[test]
    fn seam_neighbors_share_the_same_geometric_border() {
        let resolution = 8;
        let edge_index = resolution - 1;
        for face in CubeFace::ALL {
            for edge in FaceEdge::ALL {
                for along in 0..resolution {
                    let (x, y) = boundary_texel(edge, edge_index, along);
                    let (neighbor_face, neighbor_x, neighbor_y) =
                        seam_neighbor(face, edge, along, edge_index);
                    let reciprocal = FaceEdge::ALL
                        .into_iter()
                        .find(|candidate| {
                            seam_neighbor(
                                neighbor_face,
                                *candidate,
                                candidate.along(neighbor_x, neighbor_y),
                                edge_index,
                            ) == (face, x, y)
                        })
                        .unwrap();
                    assert_eq!(
                        border_midpoint(face, edge, x, y, resolution),
                        border_midpoint(
                            neighbor_face,
                            reciprocal,
                            neighbor_x,
                            neighbor_y,
                            resolution,
                        )
                    );
                }
            }
        }
    }

    const fn boundary_texel(edge: FaceEdge, edge_index: u32, along: u32) -> (u32, u32) {
        match edge {
            FaceEdge::Left => (0, along),
            FaceEdge::Right => (edge_index, along),
            FaceEdge::Bottom => (along, 0),
            FaceEdge::Top => (along, edge_index),
        }
    }

    fn border_midpoint(face: CubeFace, edge: FaceEdge, x: u32, y: u32, resolution: u32) -> Vec3 {
        let resolution = resolution as f32;
        let center_u = -1.0 + (2 * x + 1) as f32 / resolution;
        let center_v = -1.0 + (2 * y + 1) as f32 / resolution;
        let (u, v) = match edge {
            FaceEdge::Left => (-1.0 + 2.0 * x as f32 / resolution, center_v),
            FaceEdge::Right => (-1.0 + 2.0 * (x + 1) as f32 / resolution, center_v),
            FaceEdge::Bottom => (center_u, -1.0 + 2.0 * y as f32 / resolution),
            FaceEdge::Top => (center_u, -1.0 + 2.0 * (y + 1) as f32 / resolution),
        };
        unit_direction(FaceCoordinates { face, u, v })
    }
}
