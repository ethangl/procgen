//! CPU field experiment for the independent real-time world pilot.
//! Rendering is confined to the binary; these modules use no engine types.

mod contour;
mod field;
mod noise;
mod planet;
mod presets;
mod qef;
mod shell;
mod terrain;
mod volume;

#[cfg(test)]
mod test_support;

pub use field::{FieldError, MAX_COORDINATE};
pub use noise::{NoiseConfig, OCTAVES};
pub use presets::{PRESETS, Preset};
pub use terrain::{TerrainConfig, TerrainField};
pub use volume::{Axis, INSPECTION_GRID, Volume, VolumeGrid, sample_volume};

pub use contour::{
    MeshTopology, OVERVIEW_FACE_QUADS, SurfaceMesh, SurfaceTriangle, contour_shell, planet_overview,
};
pub use planet::{PILOT_PLANET, PlanetConfig, PlanetError, PlanetField, RadialBand};
pub use shell::{PILOT_SHELL, RegionAddress, ShellConfig, ShellVolume, sample_shell};
