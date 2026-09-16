//! CPU reference and GPU field contracts for the independent real-time world pilot.
//! Rendering is confined to the binary; these modules use no engine types.

mod build_id;
mod camera_clearance;
mod cube_grid;
mod design_preview;
mod field;
mod height_coverage;
mod height_detail;
mod height_distribution;
mod height_mesh;
mod height_tile;
mod meter_position;
mod noise;
mod planet_design;
/// Kept as the seed for the planned dual-contour mesher; the current voxel
/// mesher is marching tetrahedra and does not solve a QEF yet.
#[allow(dead_code)]
mod qef;
mod voxel_address;
mod voxel_attachment;
mod voxel_chunk_mesh;
mod voxel_coverage_step;
mod voxel_density;
mod voxel_gpu;
mod voxel_gpu_residency;
mod voxel_leaf_index;
mod voxel_mesh_gpu;
mod voxel_mesh_topology;
mod voxel_neighborhood;
mod voxel_selection;
mod voxel_transition;
mod voxel_transition_gpu;
mod voxel_transition_plan;

#[cfg(test)]
mod test_support;

pub use build_id::{BUILD_ID, TOOLCHAIN};
pub use camera_clearance::{CAMERA_CLEARANCE_M, keep_camera_above_terrain};
pub use design_preview::{
    DesignPreview, DesignPreviewConfig, PreviewArea, PreviewBands, PreviewError,
    generate_design_preview,
};
pub use field::FieldError;
pub use height_coverage::{
    MAX_HEIGHT_TILES, VOXEL_BAND_MAX_SPACING_M, VOXEL_BAND_REQUESTED_LEAVES, cap_voxel_coverage,
    select_height_coverage, select_voxel_band,
};
pub use height_detail::{HEIGHT_DETAIL_RATIO, HEIGHT_FILTER_MIN_M};
pub use height_distribution::HeightDistribution;
pub use height_mesh::{HEIGHT_TILE_BYTES, HeightVertex, height_indices, height_tile_vertices};
pub use height_tile::{HEIGHT_QUADS, HEIGHT_SIDE, HEIGHT_VERTEX_COUNT, HeightTile};
pub use meter_position::MeterPosition;
pub use noise::NoiseConfig;
pub use planet_design::{
    DesignError, MAX_DESIGN_OCTAVES, OctaveConfig, PlanetDesignConfig, PlanetDesignField,
    VolumeConfig,
};

pub use voxel_address::{
    ChunkIndex, VOXEL_CHUNK_CELLS, VOXEL_HALO, VOXEL_ROOT_LOD, VOXEL_SAMPLE_COUNT,
    VOXEL_SAMPLE_SIDE, VoxelChunkAddress, VoxelPosition, VoxelSampleIndex,
};
pub use voxel_attachment::{ATTACHMENT_BLOCK_CHUNKS, DetachedReport, audit_detached_solids};
pub use voxel_chunk_mesh::{
    VoxelChunkMesh, VoxelMeshError, VoxelMeshVertex, build_voxel_chunk_mesh,
};
pub use voxel_density::{
    VOXEL_DENSITY_LIMIT_M, VoxelVolume, VoxelVolumeError, sample_voxel_chunk,
    sample_voxel_potential,
};
pub use voxel_gpu::{VoxelGpuChunk, VoxelGpuParameters, voxel_density_shader};
pub use voxel_gpu_residency::{
    VoxelGpuOutcome, VoxelGpuRequest, VoxelGpuResidency, VoxelGpuTicket, VoxelPublication,
    VoxelStreamConfig, VoxelStreamError,
};
pub use voxel_mesh_gpu::{
    VOXEL_MESH_SCAN_BLOCKS, VOXEL_MESH_WORKGROUP_SIZE, VoxelMeshConfig, VoxelMeshDraw,
    VoxelMeshGpuChunk, VoxelMeshGpuParameters, VoxelMeshStatus, voxel_mesh_shader,
};
pub use voxel_mesh_topology::{VOXEL_MESH_MAX_TRIANGLES, VOXEL_MESH_VERTEX_SLOTS};
pub use voxel_neighborhood::{
    MAX_VOXEL_COVERAGE_LEAVES, VoxelCoverage, VoxelCoverageError, VoxelMeshKey,
    select_voxel_coverage,
};
pub use voxel_transition::{
    VoxelTransitionSamples, build_voxel_regular_mesh, build_voxel_transition_mesh,
};
pub use voxel_transition_gpu::{
    VoxelTransitionConfig, VoxelTransitionGpuParameters, voxel_transition_shader,
};
pub use voxel_transition_plan::{
    MAX_TRANSITION_TETRAHEDRA, VOXEL_CELL_MASK_WORDS, VoxelTransitionPlan,
};

#[cfg(feature = "gpu")]
mod height_gpu;
#[cfg(feature = "gpu")]
mod voxel_gpu_buffers;
#[cfg(feature = "gpu")]
mod voxel_gpu_compute;
#[cfg(feature = "gpu")]
mod voxel_gpu_timing;
#[cfg(feature = "gpu")]
mod voxel_gpu_world;

#[cfg(feature = "gpu")]
pub use height_gpu::{
    HEIGHT_GPU_BATCH_TILES, HeightGpuMesher, HeightGpuPipeline, LOCAL_GPU_WORLD_CONFIG,
    height_shader,
};
#[cfg(feature = "gpu")]
pub use voxel_gpu_buffers::{VoxelGpuMeshBuffers, VoxelGpuMeshConfig, VoxelGpuSlot};
#[cfg(feature = "gpu")]
pub use voxel_gpu_compute::{VoxelGpuInput, VoxelGpuMesher, VoxelGpuWork};
#[cfg(feature = "gpu")]
pub use voxel_gpu_timing::VoxelGpuTimes;
#[cfg(feature = "gpu")]
pub use voxel_gpu_world::{
    VoxelGpuError, VoxelGpuEvent, VoxelGpuLease, VoxelGpuSubmission, VoxelGpuWorld,
    VoxelGpuWorldConfig,
};
