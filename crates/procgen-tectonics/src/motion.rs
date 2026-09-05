use crate::random_streams::{ANGULAR_SPEED, ROTATION_AXIS};
use procgen_core::{RandomStream, Vec3};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateKinematicsConfig {
    pub seed: u64,
    pub minimum_angular_speed: f32,
    pub maximum_angular_speed: f32,
}

impl PlateKinematicsConfig {
    pub const fn new(seed: u64) -> Self {
        Self {
            seed,
            minimum_angular_speed: 0.5,
            maximum_angular_speed: 1.0,
        }
    }

    /// Upper bound on relative normal closing speed for the configured motion
    /// at the given sphere radius.
    pub fn maximum_convergence(&self, radius: f32) -> f32 {
        self.maximum_angular_speed * radius * 2.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlateKinematics {
    /// Euler rotation vector per plate. Direction is the rotation axis and
    /// magnitude is angular speed in model radians per unit time.
    pub angular_velocities: Vec<Vec3>,
}

impl PlateKinematics {
    /// Derives the instantaneous Cartesian velocity at a point fixed to a
    /// rigidly rotating plate. The result is tangent to the sphere at `position`.
    pub fn velocity_at(&self, plate: usize, position: Vec3) -> Vec3 {
        self.angular_velocities[plate].cross(position)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlateKinematicsError {
    InvalidAngularSpeedRange,
}

impl fmt::Display for PlateKinematicsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "angular speeds must be finite, non-negative, and ordered minimum to maximum",
        )
    }
}

impl std::error::Error for PlateKinematicsError {}

/// Generates deterministic rigid-rotation vectors independently for each plate.
pub fn generate_plate_kinematics(
    plate_count: usize,
    config: PlateKinematicsConfig,
) -> Result<PlateKinematics, PlateKinematicsError> {
    if !config.minimum_angular_speed.is_finite()
        || !config.maximum_angular_speed.is_finite()
        || config.minimum_angular_speed < 0.0
        || config.minimum_angular_speed > config.maximum_angular_speed
    {
        return Err(PlateKinematicsError::InvalidAngularSpeedRange);
    }

    let axes = RandomStream::new(config.seed, ROTATION_AXIS);
    let speeds = RandomStream::new(config.seed, ANGULAR_SPEED);
    let angular_velocities = (0..plate_count)
        .map(|plate| {
            let item = plate as u64;
            let axis = random_unit_axis(axes, item);
            let speed = config.minimum_angular_speed
                + speeds.unit_f32(item, 0)
                    * (config.maximum_angular_speed - config.minimum_angular_speed);
            axis * speed
        })
        .collect();

    Ok(PlateKinematics { angular_velocities })
}

fn random_unit_axis(stream: RandomStream, item: u64) -> Vec3 {
    let z = stream.signed_f32(item, 0);
    let theta = stream.unit_f32(item, 1) * std::f32::consts::TAU;
    let ring = (1.0 - z * z).max(0.0).sqrt();
    Vec3::new(ring * theta.cos(), ring * theta.sin(), z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angular_velocities_are_deterministic_and_bounded() {
        let config = PlateKinematicsConfig::new(17);
        let first = generate_plate_kinematics(12, config).unwrap();
        assert_eq!(first, generate_plate_kinematics(12, config).unwrap());
        assert_ne!(
            first,
            generate_plate_kinematics(12, PlateKinematicsConfig::new(18)).unwrap()
        );
        assert!(first.angular_velocities.iter().all(|velocity| {
            (config.minimum_angular_speed..=config.maximum_angular_speed)
                .contains(&velocity.length())
        }));
    }

    #[test]
    fn local_velocity_is_tangent_and_scales_with_radius() {
        let kinematics = PlateKinematics {
            angular_velocities: vec![Vec3::new(0.0, 2.0, 0.0)],
        };
        let unit_position = Vec3::new(1.0, 0.0, 0.0);
        let unit_velocity = kinematics.velocity_at(0, unit_position);
        let double_velocity = kinematics.velocity_at(0, unit_position * 2.0);

        assert_eq!(unit_velocity, Vec3::new(0.0, 0.0, -2.0));
        assert_eq!(double_velocity, unit_velocity * 2.0);
        assert_eq!(unit_velocity.dot(unit_position), 0.0);
    }

    #[test]
    fn maximum_convergence_accounts_for_both_sides() {
        let config = PlateKinematicsConfig {
            maximum_angular_speed: 2.0,
            ..PlateKinematicsConfig::new(17)
        };

        assert_eq!(config.maximum_convergence(3.0), 12.0);
    }

    #[test]
    fn rejects_invalid_speed_ranges() {
        for (minimum, maximum) in [(1.0, 0.5), (-0.1, 1.0), (0.0, f32::NAN)] {
            assert_eq!(
                generate_plate_kinematics(
                    1,
                    PlateKinematicsConfig {
                        seed: 0,
                        minimum_angular_speed: minimum,
                        maximum_angular_speed: maximum,
                    },
                ),
                Err(PlateKinematicsError::InvalidAngularSpeedRange)
            );
        }
    }
}
