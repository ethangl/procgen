use std::ops::{Add, AddAssign, Mul, Sub};

use crate::Vec3;

/// A scalar field value and its spatial derivative in three dimensions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScalarFieldSample3 {
    pub value: f32,
    pub derivative: Vec3,
}

impl ScalarFieldSample3 {
    pub const fn constant(value: f32) -> Self {
        Self {
            value,
            derivative: Vec3::ZERO,
        }
    }
}

impl Add for ScalarFieldSample3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            value: self.value + rhs.value,
            derivative: self.derivative + rhs.derivative,
        }
    }
}

impl AddAssign for ScalarFieldSample3 {
    fn add_assign(&mut self, rhs: Self) {
        self.value += rhs.value;
        self.derivative = self.derivative + rhs.derivative;
    }
}

impl Sub for ScalarFieldSample3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            value: self.value - rhs.value,
            derivative: self.derivative - rhs.derivative,
        }
    }
}

impl Mul<f32> for ScalarFieldSample3 {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self::Output {
        Self {
            value: self.value * rhs,
            derivative: self.derivative * rhs,
        }
    }
}

impl Mul for ScalarFieldSample3 {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self {
            value: self.value * rhs.value,
            derivative: self.derivative * rhs.value + rhs.derivative * self.value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_propagates_derivatives() {
        let left = ScalarFieldSample3 {
            value: 2.0,
            derivative: Vec3::new(1.0, 2.0, 3.0),
        };
        let right = ScalarFieldSample3 {
            value: 3.0,
            derivative: Vec3::new(4.0, 5.0, 6.0),
        };

        assert_eq!(
            left * right,
            ScalarFieldSample3 {
                value: 6.0,
                derivative: Vec3::new(11.0, 16.0, 21.0),
            }
        );
        assert_eq!(
            left - right,
            ScalarFieldSample3 {
                value: -1.0,
                derivative: Vec3::new(-3.0, -3.0, -3.0),
            }
        );
    }
}
