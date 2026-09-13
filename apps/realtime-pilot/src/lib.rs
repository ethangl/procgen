//! CPU field experiment for the independent real-time world pilot.
//! Rendering is confined to the binary; these modules use no engine types.

mod contour;
mod detail;
mod field;
mod noise;
mod planet;
mod presets;
mod qef;
mod routes;
mod shell;
mod streaming;
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

pub use detail::{
    DetailLevel, DetailSource, RegionMesh, STREAM_SHELL, build_region, prepare_detail,
};

pub use streaming::{
    BYTES_PER_TRIANGLE, INSTALL_MILLIS_PER_FRAME, MANAGED_MEMORY_LIMIT, MAX_ACTIVE_JOBS,
    REPLACEMENT_SECONDS, SOURCE_WORK_RESERVATION, StreamError, StreamEvent, StreamStats,
    StreamView, StreamingWorld, TRIANGLES_PER_PIECE, Ticket, UPLOAD_BYTES_PER_FRAME, UploadPiece,
};

pub use routes::{FAST_FLIGHT_SPEED, FLIGHT_SPEED, ROUTE_SECONDS, RouteSample, streaming_route};
