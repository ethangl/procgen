//! Deterministic tectonic state derived from spherical mesh topology.
//!
//! Plate partitioning, rigid spherical plate motion, static boundary
//! classification, and static crust classification live here. Geological
//! effects and time evolution remain separate later stages.

mod boundaries;
mod crust;
mod motion;
mod partition;

mod random_streams {
    // Keep stream ids globally unique so equal user-facing seeds do not
    // correlate random draws between tectonic stages.
    pub const FIRST_MAJOR_SEED: u64 = 0;
    pub const ROTATION_AXIS: u64 = 1;
    pub const ANGULAR_SPEED: u64 = 2;
    pub const CRUST_PLATE_ORDER: u64 = 3;
}

#[cfg(test)]
mod test_support;

pub use boundaries::{
    BoundaryClass, BoundaryClassification, BoundaryClassificationError, classify_boundaries,
};
pub use crust::{
    CrustClass, CrustClassification, CrustClassificationConfig, CrustClassificationError,
    classify_crust,
};
pub use motion::{
    PlateKinematics, PlateKinematicsConfig, PlateKinematicsError, generate_plate_kinematics,
};
pub use partition::{PlatePartition, PlatePartitionConfig, PlatePartitionError, partition_plates};

fn count_of<T: PartialEq>(values: impl IntoIterator<Item = T>, target: T) -> usize {
    values
        .into_iter()
        .filter(|candidate| *candidate == target)
        .count()
}
