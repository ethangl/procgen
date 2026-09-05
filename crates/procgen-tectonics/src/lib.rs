//! Deterministic tectonic state derived from spherical mesh topology.
//!
//! Plate partitioning, rigid spherical plate motion, static boundary
//! classification, static crust classification, one-step plate migration,
//! deterministic multi-step ownership evolution, and post-evolution coarse
//! elevation live here. Geological effects remain separate later stages.

mod boundaries;
mod crust;
mod elevation;
mod evolution;
mod migration;
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
pub use elevation::{
    BoundaryEffect, CoarseElevation, CoarseElevationConfig, CoarseElevationDiagnostics,
    CoarseElevationError, derive_coarse_elevation,
};
pub use evolution::{
    PlateEvolution, PlateEvolutionConfig, PlateEvolutionDiagnostics, PlateEvolutionError,
    evolve_plate_ownership,
};
pub use migration::{
    CellMigration, PlateMigration, PlateMigrationConfig, PlateMigrationError, migrate_plates_once,
};
pub use motion::{
    PlateKinematics, PlateKinematicsConfig, PlateKinematicsError, generate_plate_kinematics,
};
pub use partition::{PlatePartition, PlatePartitionConfig, PlatePartitionError, partition_plates};
