use std::ops::{Add, Mul, Neg, Sub};

/// A compact, backend-neutral three-dimensional vector.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    pub const X: Self = Self::new(1.0, 0.0, 0.0);
    pub const Y: Self = Self::new(0.0, 1.0, 0.0);
    pub const Z: Self = Self::new(0.0, 0.0, 1.0);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    pub fn length_squared(self) -> f32 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    pub fn normalized(self) -> Self {
        let length = self.length();
        if length > 1.0e-9 {
            self * length.recip()
        } else {
            Self::ZERO
        }
    }

    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    pub fn distance_squared(self, other: Self) -> f32 {
        (self - other).length_squared()
    }

    /// Rotates toward `perpendicular` by the angle whose half-angle tangent
    /// is `half_tangent`, in the plane the two vectors span. `perpendicular`
    /// must be perpendicular to `self` and of the same length; the result
    /// then has that length too, because the two coefficients are the
    /// cosine and sine of the angle written as rational functions of the
    /// half-angle tangent.
    ///
    /// The half-angle form exists so that a rotation costs only add,
    /// multiply, and divide. A sine and a cosine would put libm on the path,
    /// and libm differs between machines, so any integer a rotated vector
    /// goes on to decide would differ with it.
    pub fn rotated_toward(self, perpendicular: Self, half_tangent: f32) -> Self {
        let square = half_tangent * half_tangent;
        let scale = (1.0 + square).recip();
        self * ((1.0 - square) * scale) + perpendicular * (2.0 * half_tangent * scale)
    }
}

impl Add for Vec3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Neg for Vec3 {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self::new(-self.x, -self.y, -self.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_operations_obey_basic_invariants() {
        let x = Vec3::X;
        let y = Vec3::Y;

        assert_eq!(x.cross(y), Vec3::Z);
        assert_eq!(x.dot(y), 0.0);
        assert!(x.is_finite());
        assert!(!Vec3::new(f32::NAN, 0.0, 0.0).is_finite());
        assert!(((x + y).normalized().length() - 1.0).abs() < 1.0e-6);
        assert_eq!(Vec3::ZERO.normalized(), Vec3::ZERO);
    }

    #[test]
    fn rotation_toward_a_perpendicular_keeps_length_and_spans_a_half_turn() {
        let x = Vec3::X * 3.0;
        let y = Vec3::Y * 3.0;

        assert_eq!(x.rotated_toward(y, 0.0), x);
        // The half-angle tangent of a quarter turn is one, and of a half turn
        // is unbounded, so the whole finite range is one half turn either way.
        assert!((x.rotated_toward(y, 1.0) - y).length() < 1.0e-6);
        assert!((x.rotated_toward(y, -1.0) + y).length() < 1.0e-6);
        for half_tangent in [-4.0, -0.3, 0.7, 12.0] {
            let rotated = x.rotated_toward(y, half_tangent);
            assert!((rotated.length() - x.length()).abs() < 1.0e-6);
            assert!(rotated.dot(x.cross(y)).abs() < 1.0e-6);
        }
    }
}
