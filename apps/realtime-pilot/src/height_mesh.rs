//! Canonical filtered height tiles. The CPU path serves reproducible audits.
use crate::PlanetDesignField;
use bytemuck::{Pod, Zeroable};
use procgen_cubesphere::{TILE_QUADS, TileAddress, vertex_spacing};
use rayon::prelude::*;
pub const HEIGHT_QUADS: u32 = TILE_QUADS / 2;
pub const HEIGHT_SIDE: u32 = HEIGHT_QUADS + 1;
pub const HEIGHT_VERTEX_COUNT: u32 = HEIGHT_SIDE * HEIGHT_SIDE + 4 * HEIGHT_SIDE;
pub const HEIGHT_TILE_BYTES: u64 = HEIGHT_VERTEX_COUNT as u64 * size_of::<HeightVertex>() as u64;
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct HeightVertex {
    /// Planet-centered integer base; w is the tile level.
    pub anchor: [i32; 4],
    /// Radial residual and height displacement; w is the sample spacing.
    pub offset: [f32; 4],
}
pub fn height_tile_vertices(
    field: &PlanetDesignField,
    tile: TileAddress,
    filter: HeightFilter,
) -> Vec<HeightVertex> {
    let radius = field.config().radius_m;
    let spacing = vertex_spacing(tile.level()) * radius * (TILE_QUADS / HEIGHT_QUADS) as f32;
    (0..HEIGHT_VERTEX_COUNT)
        .into_par_iter()
        .map(|i| {
            let (x, y, skirt) = if i < HEIGHT_SIDE * HEIGHT_SIDE {
                (i % HEIGHT_SIDE, i / HEIGHT_SIDE, false)
            } else {
                let edge = (i - HEIGHT_SIDE * HEIGHT_SIDE) / HEIGHT_SIDE;
                let along = (i - HEIGHT_SIDE * HEIGHT_SIDE) % HEIGHT_SIDE;
                let (x, y) = match edge {
                    0 => (0, along),
                    1 => (HEIGHT_QUADS, along),
                    2 => (along, 0),
                    _ => (along, HEIGHT_QUADS),
                };
                (x, y, true)
            };
            let d = tile
                .grid_vertex(
                    x * (TILE_QUADS / HEIGHT_QUADS),
                    y * (TILE_QUADS / HEIGHT_QUADS),
                )
                .unwrap()
                .direction();
            let h = if skirt {
                -field.config().height_limit_m - spacing * 2.0
            } else {
                field.height(d, filter.spacing_m(d, radius))
            };
            let mut result = HeightVertex {
                anchor: [0, 0, 0, tile.level() as i32],
                offset: [0.0, 0.0, 0.0, spacing],
            };
            for (axis, value) in [d.x, d.y, d.z].into_iter().enumerate() {
                result.anchor[axis] = (value * radius).floor() as i32;
                result.offset[axis] =
                    value.mul_add(radius, -(result.anchor[axis] as f32)) + value * h;
            }
            result
        })
        .collect()
}
/// Fixed shared grid plus radial skirts below the validated height envelope.
pub fn height_indices() -> Vec<u32> {
    let mut result = Vec::new();
    for y in 0..HEIGHT_QUADS {
        for x in 0..HEIGHT_QUADS {
            let a = y * HEIGHT_SIDE + x;
            result.extend([
                a,
                a + 1,
                a + HEIGHT_SIDE + 1,
                a,
                a + HEIGHT_SIDE + 1,
                a + HEIGHT_SIDE,
            ]);
        }
    }
    for edge in 0..4 {
        for along in 0..HEIGHT_QUADS {
            let top = |i| match edge {
                0 => i * HEIGHT_SIDE,
                1 => i * HEIGHT_SIDE + HEIGHT_QUADS,
                2 => i,
                _ => HEIGHT_QUADS * HEIGHT_SIDE + i,
            };
            let a = HEIGHT_SIDE * HEIGHT_SIDE + edge * HEIGHT_SIDE + along;
            result.extend([top(along), top(along + 1), a + 1, top(along), a + 1, a]);
        }
    }
    result
}

/// One continuous footprint shared by every tile in a published surface.
/// Filtering depends on distance, not tile ownership, so adjacent tile levels
/// cannot disagree on the height at the same direction.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct HeightFilter {
    pub surface_m: [f32; 3],
    pub clearance_m: f32,
}
impl HeightFilter {
    pub fn new(field: &PlanetDesignField, eye: crate::VoxelPosition) -> Self {
        let p = procgen_core::Vec3::new(eye.x_m as f32, eye.y_m as f32, eye.z_m as f32);
        let d = if p.length_squared() == 0.0 {
            procgen_core::Vec3::X
        } else {
            p.normalized()
        };
        let surface = d * field.config().radius_m;
        Self {
            surface_m: [surface.x, surface.y, surface.z],
            clearance_m: (p.length() - field.config().radius_m - field.height(d, 0.0)).max(0.0),
        }
    }
    pub fn spacing_m(self, direction: procgen_core::Vec3, radius: f32) -> f32 {
        let center =
            procgen_core::Vec3::new(self.surface_m[0], self.surface_m[1], self.surface_m[2]);
        ((direction * radius - center)
            .length()
            .hypot(self.clearance_m)
            / HEIGHT_FILTER_DISTANCE_RATIO)
            .max(HEIGHT_FILTER_MIN_M)
    }
}
pub const HEIGHT_FILTER_DISTANCE_RATIO: f32 = 128.0;
pub const HEIGHT_FILTER_MIN_M: f32 = 0.25;
