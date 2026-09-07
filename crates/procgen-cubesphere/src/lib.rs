//! Equi-angular cube-sphere mapping and power-of-two tile addressing.

mod mapping;
mod tile;

pub use mapping::{
    CubeFace, FaceCoordinates, FaceFrame, MappingError, direction_to_face, face_to_direction,
};
pub use tile::{
    FaceGridVertex, MAX_TILE_LEVEL, TILE_QUADS, TILE_VERTICES, TileAddress, TileError, TileQuadrant,
};
