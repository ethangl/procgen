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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    let a = (coordinates.u * FRAC_PI_4).tan();
    let b = (coordinates.v * FRAC_PI_4).tan();
    (normal + u_axis * a + v_axis * b).normalized()
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
}
