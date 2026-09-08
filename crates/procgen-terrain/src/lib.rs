//! Terrain-detail control composition and, in later slices, height evaluation.

mod controls;
mod field;

pub use controls::{
    TerrainControlConfig, TerrainControlError, TerrainControlInputs, compose_terrain_controls,
};
pub use field::{TerrainCellControls, TerrainControls, TerrainStampInput, TerrainStampKind};
