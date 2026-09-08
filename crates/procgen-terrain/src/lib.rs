//! Terrain-detail control composition and backend-neutral height evaluation.

mod bake;
mod controls;
mod field;
mod height;

pub use bake::{TerrainControlBake, bake_terrain_controls};
pub use controls::{TerrainControlConfig, TerrainControlInputs, compose_terrain_controls};
pub use field::{
    TerrainCellControls, TerrainControlError, TerrainControls, TerrainStampInput, TerrainStampKind,
};
pub use height::{
    TerrainHeightConfig, TerrainHeightError, TerrainHeightInputs, TerrainHeightSample,
    TerrainStampProfile, ValidatedTerrainHeightConfig, terrain_height,
};
