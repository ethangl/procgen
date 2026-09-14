//! CPU reference and GPU field contracts for the independent real-time world pilot.
//! Rendering is confined to the binary; these modules use no engine types.

mod collision;
mod contour;
mod contour_cells;
mod design_preview;
mod detail;
mod evaluation;
mod field;
mod height_distribution;
mod mesh;
mod meter_position;
mod noise;
mod physical_audit;
mod physical_motion;
mod physical_terrain;
mod placement;
mod planet;
mod planet_design;
mod presets;
mod qef;
mod refinement;
mod routes;
mod scenario;
mod shell;
mod streaming;
mod terrain;
mod triangle_query;
mod usable;
mod volume;
mod voxel_address;
mod voxel_audit;
mod voxel_chunk_mesh;
mod voxel_collision;
mod voxel_collision_index;
mod voxel_density;
mod voxel_gpu;
mod voxel_gpu_residency;
mod voxel_leaf_index;
mod voxel_mesh_gpu;
mod voxel_mesh_topology;
mod voxel_neighborhood;
mod voxel_residency;
mod voxel_selection;
mod voxel_surface;
mod voxel_surface_audit;
mod voxel_surface_extract;
mod voxel_surface_faces;
mod voxel_surface_grid;
mod voxel_transition;
mod voxel_transition_gpu;
mod voxel_transition_plan;
mod voxel_travel_audit;
mod walking;

#[cfg(test)]
mod test_support;

pub use field::{FieldError, MAX_COORDINATE};
pub use height_distribution::{HEIGHT_DISTRIBUTION_SAMPLES, HeightDistribution};
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

pub use design_preview::{
    DesignPreview, DesignPreviewConfig, PreviewArea, PreviewBands, PreviewError,
    generate_design_preview,
};
pub use planet_design::{
    DesignError, MAX_DESIGN_OCTAVES, OctaveConfig, PlanetDesignConfig, PlanetDesignField,
};

pub use voxel_address::{
    ChunkIndex, VOXEL_CHUNK_CELLS, VOXEL_HALO, VOXEL_ROOT_LOD, VOXEL_SAMPLE_COUNT,
    VOXEL_SAMPLE_SIDE, VOXEL_WORLD_HALF_EXTENT_M, VoxelAddressError, VoxelChunkAddress,
    VoxelPosition, VoxelSampleIndex,
};
pub use voxel_density::{
    VOXEL_DENSITY_BYTES, VOXEL_DENSITY_LIMIT_M, VoxelVolume, VoxelVolumeError, sample_voxel_chunk,
};

pub use voxel_audit::{VoxelAuditConfig, VoxelAuditError, VoxelChunkAudit, audit_voxel_chunk};

pub use voxel_residency::{
    VoxelResidency, VoxelResidencyConfig, VoxelResidencyError, VoxelResidencyStats,
};

pub use voxel_travel_audit::{
    VoxelTravelAudit, VoxelTravelError, VoxelTravelStop, audit_voxel_travel,
};

pub use voxel_surface::{
    MAX_SURFACE_CHUNKS, VoxelSurface, VoxelSurfaceError, VoxelSurfaceTopology, VoxelTriangle,
};
pub use voxel_surface_extract::build_voxel_surface;

pub use voxel_collision::{
    VOXEL_CONTACT_SKIN_M, VoxelCollision, VoxelCollisionError, VoxelContact, VoxelSweep,
};

pub use voxel_surface_audit::{VoxelSurfaceAudit, VoxelSurfaceAuditError, audit_voxel_surfaces};

pub use physical_motion::{PLAYER_EYE_M, PLAYER_RADIUS_M, PLAYER_SPEED_MPS, PhysicalWalker};

pub use physical_terrain::{PhysicalTerrain, PhysicalTerrainError, PhysicalTerrainFrame};

pub use physical_audit::{
    PhysicalAudit, PhysicalAuditError, PhysicalStop, audit_physical_exploration,
};

pub use meter_position::MeterPosition;

pub use voxel_chunk_mesh::{
    VoxelChunkMesh, VoxelMeshError, VoxelMeshVertex, build_voxel_chunk_mesh,
};
pub use voxel_gpu::{VoxelGpuChunk, VoxelGpuParameters, voxel_density_shader};
pub use voxel_mesh_gpu::{
    VOXEL_MESH_SCAN_BLOCKS, VOXEL_MESH_WORKGROUP_SIZE, VoxelMeshConfig, VoxelMeshDraw,
    VoxelMeshGpuParameters, VoxelMeshStatus, voxel_mesh_shader,
};
pub use voxel_mesh_topology::{VOXEL_MESH_MAX_TRIANGLES, VOXEL_MESH_VERTEX_SLOTS};

pub use voxel_mesh_gpu::VoxelMeshGpuChunk;
pub use voxel_neighborhood::{
    MAX_VOXEL_COVERAGE_LEAVES, VoxelCoverage, VoxelCoverageError, VoxelMeshKey,
    select_voxel_coverage,
};
pub use voxel_transition::{
    VoxelTransitionSamples, build_voxel_regular_mesh, build_voxel_transition_mesh,
    sample_voxel_transition,
};
pub use voxel_transition_plan::{
    MAX_TRANSITION_TETRAHEDRA, VOXEL_CELL_MASK_WORDS, VoxelTransitionNode, VoxelTransitionPlan,
};

pub use voxel_transition_gpu::{
    VoxelTransitionConfig, VoxelTransitionGpuParameters, voxel_transition_shader,
};

pub use voxel_gpu_residency::{
    VoxelGpuOutcome, VoxelGpuRequest, VoxelGpuResidency, VoxelGpuTicket, VoxelPublication,
    VoxelStreamConfig, VoxelStreamError,
};

#[cfg(feature = "gpu")]
mod voxel_gpu_buffers;
#[cfg(feature = "gpu")]
mod voxel_gpu_compute;
#[cfg(feature = "gpu")]
pub use voxel_gpu_buffers::{VoxelGpuMeshBuffers, VoxelGpuMeshConfig, VoxelGpuSlot};
#[cfg(feature = "gpu")]
pub use voxel_gpu_compute::{VoxelGpuInput, VoxelGpuMesher, VoxelGpuWork};

#[cfg(feature = "gpu")]
mod voxel_gpu_world;
#[cfg(feature = "gpu")]
pub use voxel_gpu_world::{VoxelGpuError, VoxelGpuEvent, VoxelGpuWorld, VoxelGpuWorldConfig};
