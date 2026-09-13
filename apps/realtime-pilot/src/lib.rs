//! CPU field experiment for the independent real-time world pilot.
//! Rendering is confined to the binary; these modules use no engine types.

mod collision;
mod contour;
mod contour_cells;
mod detail;
mod evaluation;
mod field;
mod mesh;
mod noise;
mod placement;
mod planet;
mod presets;
mod qef;
mod routes;
mod scenario;
mod shell;
mod streaming;
mod terrain;
mod usable;
mod volume;
mod walking;

#[cfg(test)]
mod test_support;

pub use field::{FieldError, MAX_COORDINATE};
pub use noise::{NoiseConfig, OCTAVES};
pub use presets::{PRESETS, Preset};
pub use terrain::{TerrainConfig, TerrainField};
pub use volume::{Axis, INSPECTION_GRID, Volume, VolumeGrid, sample_volume};

pub use contour::{OVERVIEW_FACE_QUADS, contour_shell, planet_overview};
pub use mesh::{MeshTopology, SurfaceMesh, SurfaceTriangle};
pub use planet::{
    DENSITY_NORMAL_STEP, PILOT_PLANET, PlanetConfig, PlanetError, PlanetField, RadialBand,
};
pub use shell::{PILOT_SHELL, RegionAddress, ShellConfig, ShellVolume, sample_shell};

pub use detail::{
    DetailLevel, DetailSource, RegionMesh, STREAM_SHELL, build_region, prepare_detail,
};

pub use streaming::{
    BYTES_PER_TRIANGLE, DetailFocus, INSTALL_MILLIS_PER_FRAME, MANAGED_MEMORY_LIMIT,
    MAX_ACTIVE_JOBS, REPLACEMENT_SECONDS, SOURCE_WORK_RESERVATION, StreamError, StreamEvent,
    StreamStats, StreamView, StreamingWorld, TRIANGLES_PER_PIECE, Ticket, UPLOAD_BYTES_PER_FRAME,
    UploadPiece,
};

pub use routes::{
    FAST_FLIGHT_SPEED, FLIGHT_SPEED, ROUTE_SECONDS, RouteSample, route_walker, streaming_route,
    walking_input,
};

pub use collision::{
    COLLISION_REACH, CONTACT_SKIN, CollisionPatch, ContactError, SurfaceContact, TerrainQueries,
    USABLE_MEMORY_RESERVATION,
};

pub use placement::{
    LANDMARK_SEARCH_REACH, MAX_POPULATION, POPULATION_REACH, Placement, PlacementId, PlacementKind,
    Population,
};
pub use walking::{EYE_HEIGHT, MAX_WALK_SECONDS, WALK_RADIUS, WALK_SPEED, WALKABLE_COSINE, Walker};

pub use usable::{UsableError, UsableTerrain};

pub use scenario::{BUILD_ID, PLANET_PRESETS, PlanetPreset, RouteKind, Scenario, TOOLCHAIN};

pub use evaluation::{
    Evaluation, EvaluationError, EvaluationFailure, PREPARATION_LIMIT_MS, WALK_STEP_LIMIT_MS,
    evaluate,
};
