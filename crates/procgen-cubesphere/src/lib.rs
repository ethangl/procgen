//! Equi-angular cube-sphere mapping, control faces, and tile addressing.

mod field;
mod mapping;
mod tile;

pub use field::{
    BakeError, CubeField, CubeFieldSample, FaceField, MAX_CUBE_FIELD_RESOLUTION, bake_cube_field,
    control_face_resolution,
};
pub use mapping::{
    CubeFace, FaceCoordinates, FaceFrame, MappingError, direction_to_face, face_to_direction,
};
pub use tile::{
    FaceGridVertex, MAX_TILE_LEVEL, TILE_QUADS, TILE_VERTICES, TileAddress, TileError, TileQuadrant,
};

/// WGSL mirror of canonical cube-sphere mapping and tile addressing.
pub const MAPPING_WGSL_SOURCE: &str = include_str!("../wgsl/mapping.wgsl");

/// WGSL mirror of canonical cross-face field sampling.
pub const FIELD_WGSL_SOURCE: &str = include_str!("../wgsl/field.wgsl");
