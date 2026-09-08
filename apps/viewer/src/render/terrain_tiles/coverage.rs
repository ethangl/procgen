use super::{MAX_VISIBLE_TILES, SURFACE_RADIUS, TERRAIN_TILE_LEVEL};
use bevy::prelude::*;
use procgen_core::Vec3 as ProcgenVec3;
use procgen_cubesphere::{CubeFace, TILE_QUADS, TileAddress};

pub(super) fn visible_fixed_level_tiles(camera_position: Vec3) -> Vec<TileAddress> {
    let camera_distance = camera_position.length();
    let camera_direction = camera_position / camera_distance;
    let horizon = SURFACE_RADIUS / camera_distance;
    let tiles_per_axis = 1_u32 << TERRAIN_TILE_LEVEL;
    let addresses = CubeFace::ALL
        .into_iter()
        .flat_map(|face| {
            (0..tiles_per_axis).flat_map(move |y| {
                (0..tiles_per_axis).filter_map(move |x| {
                    let address = TileAddress::new(face, TERRAIN_TILE_LEVEL, x, y).unwrap();
                    tile_may_be_visible(address, camera_direction, horizon).then_some(address)
                })
            })
        })
        .collect::<Vec<_>>();
    // A 3,000-direction sweep at both detailed-mode distance extrema is pinned by the caller.
    assert!(addresses.len() <= MAX_VISIBLE_TILES);
    addresses
}

fn tile_may_be_visible(address: TileAddress, camera_direction: Vec3, horizon: f32) -> bool {
    let center = tile_direction(address, TILE_QUADS / 2, TILE_QUADS / 2);
    let extent = [
        tile_direction(address, 0, 0),
        tile_direction(address, TILE_QUADS, 0),
        tile_direction(address, 0, TILE_QUADS),
        tile_direction(address, TILE_QUADS, TILE_QUADS),
    ]
    .into_iter()
    .map(|corner| center.distance(corner))
    .fold(0.0, f32::max);
    center.dot(camera_direction) + extent >= horizon
}

pub(super) fn tile_direction(address: TileAddress, x: u32, y: u32) -> Vec3 {
    let ProcgenVec3 { x, y, z } = address.grid_vertex(x, y).unwrap().direction();
    Vec3::new(x, y, z)
}
