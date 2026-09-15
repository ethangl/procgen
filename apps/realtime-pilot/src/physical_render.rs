//! Viewer-owned coloring and the vector conversions between engine and pilot types.
use bevy::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Coloring {
    Neutral,
    /// Slope and altitude choose soil, rock, snow, or shore sand.
    #[default]
    Material,
    Height,
    Lod,
    Normals,
}
pub fn vector(p: procgen_core::Vec3) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}
pub fn core(p: Vec3) -> procgen_core::Vec3 {
    procgen_core::Vec3::new(p.x, p.y, p.z)
}
