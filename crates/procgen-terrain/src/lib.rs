//! Terrain-detail control composition and backend-neutral height evaluation.

mod bake;
mod controls;
mod field;
mod height;
mod stamp;
mod warp;

pub use bake::{TerrainControlBake, bake_terrain_controls};
pub use controls::{TerrainControlConfig, TerrainControlInputs, compose_terrain_controls};
pub use field::{
    TerrainCellControls, TerrainControlError, TerrainControls, TerrainStampInput, TerrainStampKind,
};
pub use height::{
    TerrainAbyssalConfig, TerrainDetailConfig, TerrainHeightConfig, TerrainHeightError,
    TerrainHeightInputs, TerrainHeightSample, ValidatedTerrainHeightConfig, terrain_height,
};
pub use stamp::{StampCap, TerrainStampProfile, TerrainStampProfiles};
pub use warp::TerrainCoastConfig;
