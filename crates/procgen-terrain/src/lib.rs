//! Terrain-detail control composition and, in later slices, height evaluation.

mod controls;

pub use controls::{
    TerrainCellControls, TerrainControlConfig, TerrainControlError, TerrainControlInputs,
    TerrainControls, TerrainStampInput, TerrainStampKind, compose_terrain_controls,
};
