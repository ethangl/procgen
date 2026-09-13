//! CPU field experiment for the independent real-time world pilot.
//! Rendering is confined to the binary; these modules use no engine types.

mod field;
mod noise;
mod presets;
mod terrain;
mod volume;

#[cfg(test)]
mod test_support;

pub use field::{FieldError, MAX_COORDINATE};
pub use noise::{NoiseConfig, OCTAVES};
pub use presets::{PRESETS, Preset};
pub use terrain::{TerrainConfig, TerrainField};
pub use volume::{Axis, INSPECTION_GRID, Volume, VolumeGrid, sample_volume};
