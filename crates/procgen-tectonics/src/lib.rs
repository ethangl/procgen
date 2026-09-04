//! Deterministic tectonic state derived from spherical mesh topology.
//!
//! Plate partitioning, rigid spherical plate motion, and static boundary
//! classification live here. Geological effects and time evolution remain
//! separate later stages.

mod boundaries;
mod motion;
mod partition;

#[cfg(test)]
mod test_support;

pub use boundaries::{
    BoundaryClass, BoundaryClassification, BoundaryClassificationError, classify_boundaries,
};
pub use motion::{
    PlateKinematics, PlateKinematicsConfig, PlateKinematicsError, generate_plate_kinematics,
};
pub use partition::{PlatePartition, PlatePartitionConfig, PlatePartitionError, partition_plates};
