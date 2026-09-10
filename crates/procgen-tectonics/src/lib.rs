//! Deterministic tectonic state derived from spherical mesh topology.
//!
//! Plate partitioning, flow-field-fitted rigid plate motion, static boundary
//! classification, static per-plate crust classification, one-step plate
//! migration, deterministic multi-step evolution of ownership, of the plate
//! set itself as plates rift and suture, and of what the cells carry — crust
//! birth and the deformation the boundaries raise on them step by step —
//! seafloor age in evolution steps, oceanic bathymetric base
//! elevation, and coarse elevation composition live here. Geological effects
//! remain separate later stages.

mod base_elevation;
mod boundaries;
mod cracks;
mod crust;
mod deformation;
mod elevation;
mod evolution;
mod field;
mod lifecycle;
mod migration;
mod motion;
mod partition;
mod seafloor_age;
mod stage;
mod step;

#[cfg(test)]
mod test_support;

pub use base_elevation::{
    BaseElevation, BaseElevationConfig, BaseElevationDiagnostics, BaseElevationError,
    derive_base_elevation,
};
pub use boundaries::{
    BoundaryClass, BoundaryClassification, BoundaryClassificationError,
    CONVERGENCE_TO_SHEAR_THRESHOLD, classify_boundaries,
};
pub use crust::{
    CellCrust, CrustClass, CrustClassification, CrustClassificationConfig,
    CrustClassificationError, classify_crust,
};
pub use deformation::{
    BoundaryDeformation, BoundaryDeformationConfig, BoundaryDeformationDiagnostics,
    BoundaryDeformationError, BoundaryEffect, ContinentalRiftProfile,
};
pub use elevation::{
    CoarseElevation, CoarseElevationConfig, CoarseElevationError, ElevationField,
    compose_coarse_elevation, is_land, land_elevation_meters,
};
pub use evolution::{
    PlateEvolution, PlateEvolutionConfig, PlateEvolutionDiagnostics, PlateEvolutionError,
    PlateEvolutionInputs, evolve_plate_ownership,
};
pub use field::{DEFAULT_STEP_DURATION, FieldSummary};
pub use lifecycle::PlateLifecycleConfig;
pub use migration::{
    CellMigration, PlateMigration, PlateMigrationConfig, PlateMigrationError, migrate_plates_once,
};
pub use motion::{
    FlowField, PlateKinematics, PlateKinematicsConfig, PlateKinematicsError,
    generate_plate_kinematics, generate_random_plate_kinematics,
};
pub use partition::{
    MAX_GROWTH_ROUGHNESS, PlatePartition, PlatePartitionConfig, PlatePartitionError,
    partition_plates,
};
pub use seafloor_age::{
    CrustBirthPrior, CrustBirthPriorConfig, CrustBirthPriorDiagnostics, SeafloorAge,
    SeafloorAgeDiagnostics, derive_crust_birth_prior, derive_seafloor_age,
};
pub use stage::StageInputError;
pub use step::PoleDriftConfig;
