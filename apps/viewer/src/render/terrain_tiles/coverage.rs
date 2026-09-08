use std::collections::HashSet;

use super::{SURFACE_RADIUS, TERRAIN_MAX_TILE_LEVEL};
use bevy::prelude::*;
use procgen_core::Vec3 as ProcgenVec3;
use procgen_cubesphere::{CubeFace, TILE_QUADS, TileAddress, TileQuadrant};

pub(super) const TILE_SPLIT_PROJECTED_PIXELS: f32 = 180.0;
pub(super) const TILE_MERGE_PROJECTED_PIXELS: f32 = 140.0;

const QUADRANTS: [TileQuadrant; 4] = [
    TileQuadrant::LowerLeft,
    TileQuadrant::LowerRight,
    TileQuadrant::UpperLeft,
    TileQuadrant::UpperRight,
];

#[derive(Clone, Copy, Debug)]
pub(super) struct CoverageView {
    pub position: Vec3,
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub vertical_fov: f32,
    pub aspect_ratio: f32,
    pub viewport_height: f32,
}

#[derive(Resource, Default)]
pub(super) struct QuadtreeSelection {
    split: HashSet<TileAddress>,
}

impl QuadtreeSelection {
    pub(super) fn select(&mut self, view: CoverageView) -> Vec<TileAddress> {
        let previous = std::mem::take(&mut self.split);
        let mut selected = Vec::new();
        for face in CubeFace::ALL {
            let root = TileAddress::new(face, 0, 0, 0).unwrap();
            self.visit(root, view, &previous, &mut selected);
        }
        selected
    }

    pub(super) fn clear(&mut self) {
        self.split.clear();
    }

    fn visit(
        &mut self,
        address: TileAddress,
        view: CoverageView,
        previous: &HashSet<TileAddress>,
        selected: &mut Vec<TileAddress>,
    ) {
        let bounds = TileBounds::new(address);
        if !bounds.may_be_visible(view) {
            return;
        }
        let threshold = if previous.contains(&address) {
            TILE_MERGE_PROJECTED_PIXELS
        } else {
            TILE_SPLIT_PROJECTED_PIXELS
        };
        if address.level() < TERRAIN_MAX_TILE_LEVEL && bounds.projected_diameter(view) >= threshold
        {
            self.split.insert(address);
            for quadrant in QUADRANTS {
                self.visit(address.child(quadrant).unwrap(), view, previous, selected);
            }
        } else {
            selected.push(address);
        }
    }
}

#[derive(Clone, Copy)]
struct TileBounds {
    center: Vec3,
    radius: f32,
}

impl TileBounds {
    fn new(address: TileAddress) -> Self {
        let center = tile_direction(address, TILE_QUADS / 2, TILE_QUADS / 2);
        let radius = [
            tile_direction(address, 0, 0),
            tile_direction(address, TILE_QUADS, 0),
            tile_direction(address, 0, TILE_QUADS),
            tile_direction(address, TILE_QUADS, TILE_QUADS),
        ]
        .into_iter()
        .map(|corner| center.distance(corner))
        .fold(0.0, f32::max);
        Self { center, radius }
    }

    fn may_be_visible(self, view: CoverageView) -> bool {
        let camera_distance = view.position.length();
        let camera_direction = view.position / camera_distance;
        let horizon = SURFACE_RADIUS / camera_distance;
        if self.center.dot(camera_direction) + self.radius < horizon {
            return false;
        }

        let offset = self.center * SURFACE_RADIUS - view.position;
        let depth = offset.dot(view.forward);
        if depth + self.radius <= 0.0 {
            return false;
        }
        let tan_y = (view.vertical_fov * 0.5).tan();
        let tan_x = tan_y * view.aspect_ratio;
        let vertical_margin = self.radius * (1.0 + tan_y * tan_y).sqrt();
        let horizontal_margin = self.radius * (1.0 + tan_x * tan_x).sqrt();
        offset.dot(view.up).abs() <= depth * tan_y + vertical_margin
            && offset.dot(view.right).abs() <= depth * tan_x + horizontal_margin
    }

    fn projected_diameter(self, view: CoverageView) -> f32 {
        let depth = (self.center * SURFACE_RADIUS - view.position).dot(view.forward);
        if depth <= self.radius {
            return f32::INFINITY;
        }
        view.viewport_height * self.radius / (depth * (view.vertical_fov * 0.5).tan())
    }
}

pub(super) fn tile_direction(address: TileAddress, x: u32, y: u32) -> Vec3 {
    let ProcgenVec3 { x, y, z } = address.grid_vertex(x, y).unwrap().direction();
    Vec3::new(x, y, z)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(position: Vec3, viewport_height: f32) -> CoverageView {
        let forward = -position.normalize();
        let right = forward.cross(Vec3::Y).normalize();
        CoverageView {
            position,
            forward,
            right,
            up: right.cross(forward),
            vertical_fov: std::f32::consts::FRAC_PI_4,
            aspect_ratio: 1.6,
            viewport_height,
        }
    }

    #[test]
    fn selection_is_deterministic_and_never_exceeds_the_slice_maximum_level() {
        let view = view(Vec3::new(0.7, 0.3, 1.0).normalize() * 1.4, 800.0);
        let first = QuadtreeSelection::default().select(view);
        let second = QuadtreeSelection::default().select(view);
        assert_eq!(first, second);
        assert!(
            first
                .iter()
                .all(|tile| tile.level() <= TERRAIN_MAX_TILE_LEVEL)
        );
        assert!(
            first
                .iter()
                .any(|tile| tile.level() == TERRAIN_MAX_TILE_LEVEL)
        );
    }

    #[test]
    fn projected_size_drives_splits() {
        let position = Vec3::Z * 1.7;
        let small = QuadtreeSelection::default().select(view(position, 200.0));
        let large = QuadtreeSelection::default().select(view(position, 1_600.0));
        assert!(large.len() > small.len());
        assert!(
            large.iter().map(|tile| tile.level()).max()
                > small.iter().map(|tile| tile.level()).max()
        );
    }

    #[test]
    fn split_and_merge_thresholds_form_a_hysteresis_band() {
        let position = Vec3::Z * 1.7;
        let tile = TileAddress::new(CubeFace::PositiveZ, 2, 1, 1).unwrap();
        let pixels_per_height = TileBounds::new(tile).projected_diameter(view(position, 1.0));
        let band_height =
            (TILE_SPLIT_PROJECTED_PIXELS + TILE_MERGE_PROJECTED_PIXELS) * 0.5 / pixels_per_height;
        let view = view(position, band_height);
        let mut previous = HashSet::new();
        previous.insert(tile);
        let mut held = Vec::new();
        QuadtreeSelection::default().visit(tile, view, &previous, &mut held);
        let mut fresh = Vec::new();
        QuadtreeSelection::default().visit(tile, view, &HashSet::new(), &mut fresh);
        assert!(held.iter().all(|child| child.parent() == Some(tile)));
        assert_eq!(fresh, vec![tile]);
    }

    #[test]
    fn conservative_culling_keeps_every_sampled_visible_point_covered() {
        let view = view(Vec3::new(1.0, 0.4, -0.7).normalize() * 1.75, 800.0);
        let selected = QuadtreeSelection::default().select(view);
        let horizon = SURFACE_RADIUS / view.position.length();
        let camera_direction = view.position.normalize();
        for face in CubeFace::ALL {
            for y in 0..1_u32 << TERRAIN_MAX_TILE_LEVEL {
                for x in 0..1_u32 << TERRAIN_MAX_TILE_LEVEL {
                    let fine = TileAddress::new(face, TERRAIN_MAX_TILE_LEVEL, x, y).unwrap();
                    let visibly_sampled = (0..=TILE_QUADS)
                        .step_by((TILE_QUADS / 4) as usize)
                        .flat_map(|local_y| {
                            (0..=TILE_QUADS)
                                .step_by((TILE_QUADS / 4) as usize)
                                .map(move |local_x| tile_direction(fine, local_x, local_y))
                        })
                        .any(|direction| {
                            direction.dot(camera_direction) >= horizon
                                && point_in_frustum(direction, view)
                        });
                    if visibly_sampled {
                        assert!(selected.iter().any(|tile| is_ancestor(*tile, fine)));
                    }
                }
            }
        }
    }

    fn is_ancestor(ancestor: TileAddress, mut tile: TileAddress) -> bool {
        while tile.level() > ancestor.level() {
            tile = tile.parent().unwrap();
        }
        tile == ancestor
    }

    fn point_in_frustum(point: Vec3, view: CoverageView) -> bool {
        let offset = point - view.position;
        let depth = offset.dot(view.forward);
        let tan_y = (view.vertical_fov * 0.5).tan();
        let tan_x = tan_y * view.aspect_ratio;
        depth > 0.0
            && offset.dot(view.up).abs() <= depth * tan_y
            && offset.dot(view.right).abs() <= depth * tan_x
    }
}
