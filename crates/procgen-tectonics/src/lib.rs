//! Deterministic tectonic state derived from spherical mesh topology.
//!
//! Plate partitioning, flow-field-fitted rigid plate motion, static boundary
//! classification, per-cell continental crust grown from nuclei to a target
//! area, deterministic multi-step evolution of ownership, of the plate set
//! itself as plates rift and suture, and of the material the plates carry —
//! particles of crust that rotate rigidly with their plate, holding the birth
//! time and the accumulated deformation each cell reads off them — seafloor
//! age as model time, oceanic bathymetric base elevation, and coarse
//! elevation composition live here. Geological effects remain separate later
//! stages.

mod base_elevation;
mod boundaries;
mod boundary_profiles;
mod cracks;
mod crust;
mod deformation;
mod elevation;
mod evolution;
mod evolution_error;
mod field;
mod interior_relief;
mod lifecycle;
mod motion;
mod partition;
mod reach;
mod rifting;
mod seafloor_age;
mod speed;
mod stage;
mod step;
mod transport;

#[cfg(test)]
mod test_support;

pub use base_elevation::{
    BaseElevation, BaseElevationConfig, BaseElevationDiagnostics, BaseElevationError,
    derive_base_elevation,
};
pub use boundaries::{
    BoundaryClass, BoundaryClassification, BoundaryClassificationError, classify_boundaries,
    subducting_fractions,
};
pub use boundary_profiles::{
    BoundaryDeformationConfig, BoundaryDeformationError, BoundaryEffect, ContinentalRiftProfile,
};
pub use crust::{
    CellCrust, CrustClass, CrustClassification, CrustClassificationConfig,
    CrustClassificationDiagnostics, CrustClassificationError, classify_crust, material_order,
};
pub use deformation::{BoundaryDeformation, BoundaryDeformationDiagnostics};
pub use elevation::{
    CoarseElevation, CoarseElevationConfig, CoarseElevationError, ElevationField,
    compose_coarse_elevation, is_land, land_elevation_meters,
};
pub use evolution::{
    PlateEvolution, PlateEvolutionConfig, PlateEvolutionDiagnostics, PlateEvolutionInputs,
    evolve_plate_ownership,
};
pub use evolution_error::PlateEvolutionError;
pub use field::{DEFAULT_STEP_DURATION, FieldSummary};
pub use lifecycle::PlateLifecycleConfig;
pub use motion::{
    FlowField, PlateKinematics, PlateKinematicsConfig, PlateKinematicsError,
    generate_plate_kinematics,
};
pub use partition::{
    MAX_GROWTH_ROUGHNESS, PlatePartition, PlatePartitionConfig, PlatePartitionError,
    partition_plates,
};
pub use reach::{
    MAX_GAP_RADIUS, MaterialTransportConfig, TRANSPORT_REACH_HOPS, maximum_step_duration,
};
pub use seafloor_age::{
    CrustBirthPrior, CrustBirthPriorConfig, CrustBirthPriorDiagnostics, CrustBirthPriorError,
    SeafloorAge, SeafloorAgeDiagnostics, derive_crust_birth_prior, derive_seafloor_age,
};
pub use speed::{PlateSpeedSummary, plate_speed};
pub use stage::StageInputError;
pub use step::PoleDriftConfig;
