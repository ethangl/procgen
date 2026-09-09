//! Equi-angular cube-sphere mapping, control faces, and tile addressing.

mod field;
mod mapping;
mod raster;
mod tile;

pub use field::{
    BakeError, CubeField, CubeFieldSample, FaceField, bake_cube_field, control_face_resolution,
};
pub use mapping::{
    CubeFace, EQUIANGULAR_TANGENT_TEST_VECTORS, FaceCoordinates, FaceEdge, FaceFrame, MappingError,
    direction_to_face, equiangular_tangent, face_to_direction,
};
pub use raster::{
    AXIS_LINK_LENGTH, BORDER_LINKS_PER_CELL, DIAGONAL_LINK_LENGTH, FaceTexel,
    MAX_BORDER_RESOLUTION, MAX_RASTER_RESOLUTION, NO_RASTER_CELL, RasterError,
    TEXEL_SOLID_ANGLE_TOLERANCE, TexelLink,
};
pub use tile::{
    FaceGridVertex, MAX_TILE_LEVEL, TILE_QUADS, TILE_VERTICES, TileAddress, TileError,
    TileQuadrant, vertex_spacing,
};

/// WGSL mirror of canonical cube-sphere mapping and tile addressing.
pub const MAPPING_WGSL_SOURCE: &str = include_str!("../wgsl/mapping.wgsl");

/// WGSL mirror of canonical cube-sphere raster addressing and adjacency.
pub const RASTER_WGSL_SOURCE: &str = include_str!("../wgsl/raster.wgsl");

/// WGSL mirror of canonical cross-face field sampling.
pub const FIELD_WGSL_SOURCE: &str = include_str!("../wgsl/field.wgsl");
