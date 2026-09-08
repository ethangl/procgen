//! Terrain-detail control composition and, in later slices, height evaluation.

mod bake;
mod controls;
mod field;

pub use bake::{TerrainControlBake, bake_terrain_controls};
pub use controls::{TerrainControlConfig, TerrainControlInputs, compose_terrain_controls};
pub use field::{
    TerrainCellControls, TerrainControlError, TerrainControls, TerrainStampInput, TerrainStampKind,
};
