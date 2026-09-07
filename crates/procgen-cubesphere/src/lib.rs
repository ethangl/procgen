//! Equi-angular cube-sphere mapping, control faces, and tile addressing.

mod controls;
mod mapping;
mod tile;

pub use controls::{
    ControlBake, ControlBakeError, ControlFace, bake_control_faces, control_face_resolution,
};
pub use mapping::{
    CubeFace, FaceCoordinates, FaceFrame, MappingError, direction_to_face, face_to_direction,
};
pub use tile::{
    FaceGridVertex, MAX_TILE_LEVEL, TILE_QUADS, TILE_VERTICES, TileAddress, TileError, TileQuadrant,
};
