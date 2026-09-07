//! Terrain-detail control composition and, in later slices, height evaluation.

mod controls;

pub use controls::{
    TERRAIN_CONTROL_CHANNELS, TerrainCellControls, TerrainControlConfig, TerrainControlError,
    TerrainControlInputs, TerrainControls, TerrainControlsError, TerrainStampInput,
    TerrainStampKind, compose_terrain_controls,
};
