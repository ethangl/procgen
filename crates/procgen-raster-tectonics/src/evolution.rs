//! Data contracts for the raster plate evolution.
//!
//! Motion, boundary classification, and migration keep the mesh pipeline's
//! definitions, so their configuration types are reused rather than restated.
//! Crust no longer can: the mesh path classifies crust per cell, and the
//! pilot's kernel still flips whole plates until a target ocean area is met,
//! so the pilot owns [`RasterCrustClassificationConfig`] the way it already
//! owns its partition config, until its own crust slice. What this module owns
//! is the configuration itself and the per-plate inputs the host computes for
//! the kernels; how their results are packed is [`crate::field`]'s.

use crate::device::RasterTectonicsError;
use crate::field::{PLATE_ID_COUNT, RasterPlate};
use procgen_core::{RandomStream, random_streams::CRUST_PLATE_ORDER};
use procgen_cubesphere::NO_RASTER_CELL;
use procgen_tectonics::{
    PlateKinematicsConfig, PlateMigrationConfig, generate_random_plate_kinematics,
};

/// Steps per unit an angular-velocity component is quantized to before upload.
///
/// The host computes plate motion with the platform's math library, so the
/// components are snapped to a power-of-two grid that `f32` represents exactly
/// and no ulp difference between platforms reaches a kernel.
pub const ANGULAR_VELOCITY_STEPS_PER_UNIT: f32 = (1 << 20) as f32;

/// Configuration of the pilot's per-plate crust classification.
///
/// Plates are visited in a seeded order and flipped to oceanic while that
/// moves the achieved ocean fraction closer to the target. The mesh path grew
/// past this in the per-cell crust slice; the pilot keeps it until its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterCrustClassificationConfig {
    /// Desired fraction of the sphere's surface covered by oceanic crust.
    pub target_ocean_fraction: f32,
    pub seed: u64,
}

impl RasterCrustClassificationConfig {
    pub const fn new(seed: u64) -> Self {
        Self {
            target_ocean_fraction: 0.7,
            seed,
        }
    }
}

/// Configuration of the raster plate evolution.
///
/// Every field carries the mesh pipeline's meaning unchanged, so the shared
/// configuration types are reused as data contracts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterEvolutionConfig {
    pub crust: RasterCrustClassificationConfig,
    pub kinematics: PlateKinematicsConfig,
    pub migration: PlateMigrationConfig,
    /// Complete boundary-classification and migration transitions.
    pub step_count: u32,
}

impl Default for RasterEvolutionConfig {
    fn default() -> Self {
        Self {
            crust: RasterCrustClassificationConfig {
                target_ocean_fraction: 0.75,
                ..RasterCrustClassificationConfig::new(0)
            },
            kinematics: PlateKinematicsConfig::new(0),
            migration: PlateMigrationConfig::default(),
            step_count: 9,
        }
    }
}

impl RasterEvolutionConfig {
    /// Builds the plate buffer's host-owned fields: the seeded order the crust
    /// classification visits plates in, and each plate's quantized motion.
    ///
    /// The kernels fill the rest, so the whole buffer is uploaded once per run
    /// and no stage reads a stale seed cell, area, or crust class.
    pub fn plate_records(
        &self,
        plate_count: u32,
    ) -> Result<Vec<RasterPlate>, RasterTectonicsError> {
        let kinematics = generate_random_plate_kinematics(plate_count as usize, self.kinematics)
            .map_err(|_| RasterTectonicsError::InvalidAngularSpeedRange)?;
        let random = RandomStream::new(self.crust.seed, CRUST_PLATE_ORDER);
        let mut order: Vec<u32> = (0..plate_count).collect();
        order.sort_unstable_by_key(|&plate| (random.sample_u64(u64::from(plate), 0), plate));

        let mut records = vec![
            RasterPlate {
                seed_cell: NO_RASTER_CELL,
                ..RasterPlate::default()
            };
            PLATE_ID_COUNT as usize
        ];
        for (record, (&order, velocity)) in records
            .iter_mut()
            .zip(order.iter().zip(&kinematics.angular_velocities))
        {
            record.order = order;
            record.angular_velocity = [velocity.x, velocity.y, velocity.z].map(quantize);
        }
        Ok(records)
    }

    pub(crate) fn validate(&self) -> Result<(), RasterTectonicsError> {
        if !self.crust.target_ocean_fraction.is_finite()
            || !(0.0..=1.0).contains(&self.crust.target_ocean_fraction)
        {
            return Err(RasterTectonicsError::InvalidOceanFraction);
        }
        if !self.migration.minimum_convergence.is_finite()
            || self.migration.minimum_convergence < 0.0
        {
            return Err(RasterTectonicsError::InvalidMinimumConvergence);
        }
        Ok(())
    }
}

/// Snaps one angular-velocity component to the upload grid. Both the scale and
/// the rounded value are exact in `f32`, so the snap itself adds no error the
/// next platform could round differently.
fn quantize(component: f32) -> f32 {
    (component * ANGULAR_VELOCITY_STEPS_PER_UNIT).round() / ANGULAR_VELOCITY_STEPS_PER_UNIT
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_core::Vec3;

    #[test]
    fn plate_records_carry_a_seeded_order_and_quantized_motion() {
        let config = RasterEvolutionConfig::default();
        let records = config.plate_records(12).unwrap();

        assert_eq!(records.len(), PLATE_ID_COUNT as usize);
        let mut order: Vec<u32> = records[..12].iter().map(|record| record.order).collect();
        assert_ne!(
            order,
            (0..12).collect::<Vec<_>>(),
            "the order must be seeded"
        );
        order.sort_unstable();
        assert_eq!(order, (0..12).collect::<Vec<_>>(), "every plate once");

        let kinematics = generate_random_plate_kinematics(12, config.kinematics).unwrap();
        for (record, velocity) in records.iter().zip(&kinematics.angular_velocities) {
            let quantized = Vec3::new(
                record.angular_velocity[0],
                record.angular_velocity[1],
                record.angular_velocity[2],
            );
            assert!((quantized - *velocity).length() <= 3.0 / ANGULAR_VELOCITY_STEPS_PER_UNIT);
            for component in record.angular_velocity {
                let steps = component * ANGULAR_VELOCITY_STEPS_PER_UNIT;
                assert_eq!(steps, steps.round(), "{component} is off the upload grid");
            }
        }
        assert!(
            records[12..]
                .iter()
                .all(|record| record.angular_velocity == [0.0; 3]),
            "unaddressed plate ids stay still"
        );
    }

    #[test]
    fn rejects_configurations_the_kernels_cannot_represent() {
        let config = RasterEvolutionConfig::default();
        for target in [-0.1, 1.1, f32::NAN] {
            assert_eq!(
                RasterEvolutionConfig {
                    crust: RasterCrustClassificationConfig {
                        target_ocean_fraction: target,
                        ..config.crust
                    },
                    ..config
                }
                .validate(),
                Err(RasterTectonicsError::InvalidOceanFraction)
            );
        }
        assert_eq!(
            RasterEvolutionConfig {
                migration: PlateMigrationConfig {
                    minimum_convergence: -1.0,
                },
                ..config
            }
            .validate(),
            Err(RasterTectonicsError::InvalidMinimumConvergence)
        );
        assert_eq!(
            RasterEvolutionConfig {
                kinematics: PlateKinematicsConfig {
                    minimum_angular_speed: 2.0,
                    maximum_angular_speed: 1.0,
                    ..config.kinematics
                },
                ..config
            }
            .plate_records(4),
            Err(RasterTectonicsError::InvalidAngularSpeedRange)
        );
    }
}
