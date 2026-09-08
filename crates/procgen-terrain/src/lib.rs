//! Terrain-detail control composition and backend-neutral height evaluation.

mod bake;
mod coast;
mod controls;
mod field;
mod height;
mod stamp;

pub use bake::{TerrainControlBake, bake_terrain_controls};
pub use coast::{TerrainCoastConfig, TerrainCoastError};
pub use controls::{TerrainControlConfig, TerrainControlInputs, compose_terrain_controls};
pub use field::{
    TerrainCellControls, TerrainControlError, TerrainControls, TerrainStampInput, TerrainStampKind,
};
pub use height::{
    TerrainAbyssalConfig, TerrainDetailConfig, TerrainHeightConfig, TerrainHeightError,
    TerrainHeightInputs, TerrainNoiseSeeds, ValidatedTerrainHeightConfig, terrain_height,
};
pub use stamp::{StampCap, TerrainStampError, TerrainStampProfile, TerrainStampProfiles};
