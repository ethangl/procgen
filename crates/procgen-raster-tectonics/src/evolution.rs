//! Data contracts for the raster plate evolution.
//!
//! Crust, motion, boundary classification, and migration keep the mesh
//! pipeline's definitions, so their configuration types are reused rather than
//! restated. What this module owns is how the raster packs their results: one
//! ownership word per cell carrying the current and pending plate, one
//! boundary word per cell carrying the class of each of its four borders, and
//! the integer area units the crust reduction sums.

use crate::field::{RasterPlate, RasterTectonicsError};
use crate::partition::PLATE_ID_COUNT;
use procgen_core::{RandomStream, random_streams::CRUST_PLATE_ORDER};
use procgen_cubesphere::{NO_RASTER_CELL, TexelLink};
use procgen_tectonics::{
    BoundaryClass, CrustClass, CrustClassificationConfig, PlateKinematicsConfig,
    PlateMigrationConfig, generate_plate_kinematics,
};

/// Area units one steradian is worth in the crust reduction.
///
/// Cell areas are summed with integer atomics, so the reduction is
/// order-independent by construction. The scale is the largest that keeps the
/// whole sphere inside a `u32` with room to spare, which leaves the coarsest
/// cell about sixty units at the pilot's default resolution.
pub const AREA_UNITS_PER_STERADIAN: u32 = 32_000_000;

const _: () = assert!(
    13 * AREA_UNITS_PER_STERADIAN as u64 <= u32::MAX as u64,
    "the whole sphere's area, under 13 steradians, must fit one u32 counter"
);

/// Bits of a cell's ownership word reserved for its current plate. The pending
/// plate a migration step proposes occupies the bits above them.
pub const CELL_PLATE_BITS: u32 = 16;
/// Mask of one plate field inside a cell's ownership word.
pub const CELL_PLATE_MASK: u32 = (1 << CELL_PLATE_BITS) - 1;

const _: () = assert!(PLATE_ID_COUNT <= CELL_PLATE_MASK + 1);

/// Bits one border's class occupies in a cell's packed boundary word.
pub const BOUNDARY_CLASS_BITS: u32 = 2;
/// Mask of one border's class inside a cell's packed boundary word.
pub const BOUNDARY_CLASS_MASK: u32 = (1 << BOUNDARY_CLASS_BITS) - 1;

const _: () = assert!(BoundaryClass::ALL.len() as u32 <= BOUNDARY_CLASS_MASK + 1);

/// Steps per unit an angular-velocity component is quantized to before upload.
///
/// The host computes plate motion with the platform's math library, so the
/// components are snapped to a power-of-two grid that `f32` represents exactly
/// and no ulp difference between platforms reaches a kernel.
pub const ANGULAR_VELOCITY_STEPS_PER_UNIT: f32 = (1 << 20) as f32;

/// Packs a cell's current and pending plate into one ownership word.
///
/// Migration is a simultaneous update, so a step reads every cell's current
/// plate while writing its own pending plate. Both live in one word because
/// the kernels are already at `wgpu`'s default storage-binding limit.
pub const fn plate_ownership(current: u32, pending: u32) -> u32 {
    (pending << CELL_PLATE_BITS) | current
}

/// Returns the plate a cell currently belongs to.
pub const fn current_plate(ownership: u32) -> u32 {
    ownership & CELL_PLATE_MASK
}

/// Returns the plate the last migration step proposed for a cell, which equals
/// its current plate when no proposal won.
pub const fn pending_plate(ownership: u32) -> u32 {
    ownership >> CELL_PLATE_BITS
}

/// Returns the code the boundary buffer stores for a class.
pub const fn boundary_class_code(class: BoundaryClass) -> u32 {
    class as u32
}

/// Returns the code the plate buffer stores for a crust class.
pub const fn crust_class_code(class: CrustClass) -> u32 {
    class as u32
}

/// Returns the class of one of a cell's borders from its packed boundary word.
///
/// Both cells of a border classify it independently from the same inputs and
/// reach the same class, so a cell's word answers for every border it touches
/// without resolving which cell owns the border edge.
pub fn boundary_class(classes: u32, link: TexelLink) -> BoundaryClass {
    assert!(link.is_border(), "only border links carry a boundary class");
    match (classes >> (BOUNDARY_CLASS_BITS * link.index())) & BOUNDARY_CLASS_MASK {
        0 => BoundaryClass::Interior,
        1 => BoundaryClass::Convergent,
        2 => BoundaryClass::Divergent,
        _ => BoundaryClass::Transform,
    }
}

/// Configuration of the raster plate evolution.
///
/// Every field carries the mesh pipeline's meaning unchanged, so the shared
/// configuration types are reused as data contracts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterEvolutionConfig {
    pub crust: CrustClassificationConfig,
    pub kinematics: PlateKinematicsConfig,
    pub migration: PlateMigrationConfig,
    /// Complete boundary-classification and migration transitions.
    pub step_count: u32,
}

impl Default for RasterEvolutionConfig {
    fn default() -> Self {
        Self {
            crust: CrustClassificationConfig {
                target_ocean_fraction: 0.75,
                ..CrustClassificationConfig::new(0)
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
        let kinematics = generate_plate_kinematics(plate_count as usize, self.kinematics)
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
    fn ownership_carries_the_current_and_pending_plate_independently() {
        let ownership = plate_ownership(7, 300);
        assert_eq!(current_plate(ownership), 7);
        assert_eq!(pending_plate(ownership), 300);
        assert_eq!(
            pending_plate(plate_ownership(
                crate::UNCLAIMED_PLATE,
                crate::UNCLAIMED_PLATE
            )),
            crate::UNCLAIMED_PLATE
        );
    }

    #[test]
    fn each_border_link_reads_back_its_own_class() {
        let mut classes = 0;
        for (index, link) in TexelLink::BORDERS.into_iter().enumerate() {
            classes |= boundary_class_code(BoundaryClass::ALL[index])
                << (BOUNDARY_CLASS_BITS * link.index());
        }
        for (index, link) in TexelLink::BORDERS.into_iter().enumerate() {
            assert_eq!(boundary_class(classes, link), BoundaryClass::ALL[index]);
        }
    }

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

        let kinematics = generate_plate_kinematics(12, config.kinematics).unwrap();
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
                    crust: CrustClassificationConfig {
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
