//! Terrain-detail control composition and backend-neutral height evaluation.

mod bake;
mod coast;
mod controls;
mod field;
mod height;
mod stamp;
mod tile;

#[cfg(test)]
mod test_support;

pub use bake::{TerrainControlBake, bake_terrain_controls};
pub use coast::{TerrainCoastConfig, TerrainCoastError};
pub use controls::{TerrainControlConfig, TerrainControlInputs, compose_terrain_controls};
pub use field::{
    TerrainCellControls, TerrainControlError, TerrainControls, TerrainStampInput, TerrainStampKind,
};
pub use height::{
    TerrainAbyssalConfig, TerrainDetailConfig, TerrainHeightConfig, TerrainHeightError,
    TerrainHeightInputs, TerrainNoiseKeys, ValidatedTerrainHeightConfig, terrain_height,
};
pub use stamp::{StampCap, TerrainStampError, TerrainStampProfile, TerrainStampProfiles};
pub use tile::{
    TERRAIN_TILE_SAMPLE_COUNT, TerrainTile, TerrainTileError, TerrainTileInputs,
    generate_terrain_tile,
};
