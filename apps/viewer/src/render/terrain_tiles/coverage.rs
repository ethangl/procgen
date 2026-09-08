use std::collections::{BTreeSet, HashSet};

use super::{SURFACE_RADIUS, TERRAIN_MAX_TILE_LEVEL};
use bevy::{camera::Projection, prelude::*};
use procgen_core::Vec3 as ProcgenVec3;
use procgen_cubesphere::{CubeFace, TILE_QUADS, TileAddress, TileEdge};

const TILE_SPLIT_PROJECTED_PIXELS: f32 = 180.0;
const TILE_MERGE_PROJECTED_PIXELS: f32 = 140.0;

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

impl CoverageView {
    pub(super) fn from_camera(
        camera: &Camera,
        projection: &Projection,
        transform: &Transform,
    ) -> Option<Self> {
        let Projection::Perspective(projection) = projection else {
            return None;
        };
        Some(Self {
            position: transform.translation,
            forward: *transform.forward(),
            right: *transform.right(),
            up: *transform.up(),
            vertical_fov: projection.fov,
            aspect_ratio: projection.aspect_ratio,
            viewport_height: camera.logical_viewport_size()?.y,
        })
    }
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
            let root = TileAddress::root(face);
            self.visit(root, view, &previous, &mut selected);
        }
        self.balance_neighbor_levels(&mut selected);
        selected.sort();
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
            for child in address.children().unwrap() {
                self.visit(child, view, previous, selected);
            }
        } else {
            selected.push(address);
        }
    }

    fn balance_neighbor_levels(&mut self, selected: &mut Vec<TileAddress>) {
        loop {
            let leaves = selected.iter().copied().collect::<HashSet<_>>();
            let mut split = BTreeSet::new();
            for &tile in selected.iter() {
                for edge in TileEdge::ALL {
                    let same_level = tile.edge_neighbor(edge);
                    let Some(neighbor) =
                        std::iter::successors(Some(same_level), |address| address.parent())
                            .find(|address| leaves.contains(address))
                    else {
                        continue;
                    };
                    if neighbor.level() + 1 < tile.level() {
                        split.insert(neighbor);
                    }
                }
            }
            if split.is_empty() {
                return;
            }
            selected.retain(|tile| !split.contains(tile));
            for tile in split {
                self.split.insert(tile);
                selected.extend(tile.children().unwrap());
            }
        }
    }
}

pub(super) fn tile_morph_factor(address: TileAddress, view: CoverageView) -> f32 {
    let Some(parent) = address.parent() else {
        return 1.0;
    };
    ((TileBounds::new(parent).projected_diameter(view) - TILE_SPLIT_PROJECTED_PIXELS)
        / TILE_SPLIT_PROJECTED_PIXELS)
        .clamp(0.0, 1.0)
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
        let radius = self.radius * SURFACE_RADIUS;
        if depth + radius <= 0.0 {
            return false;
        }
        let tan_y = (view.vertical_fov * 0.5).tan();
        let tan_x = tan_y * view.aspect_ratio;
        let vertical_margin = radius * (1.0 + tan_y * tan_y).sqrt();
        let horizontal_margin = radius * (1.0 + tan_x * tan_x).sqrt();
        offset.dot(view.up).abs() <= depth * tan_y + vertical_margin
            && offset.dot(view.right).abs() <= depth * tan_x + horizontal_margin
    }

    fn projected_diameter(self, view: CoverageView) -> f32 {
        let depth = (self.center * SURFACE_RADIUS - view.position).dot(view.forward);
        let radius = self.radius * SURFACE_RADIUS;
        let nearest_depth = depth - radius;
        if nearest_depth <= 0.0 {
            return f32::INFINITY;
        }
        view.viewport_height * radius / (nearest_depth * (view.vertical_fov * 0.5).tan())
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
        assert!(first.iter().any(|tile| tile.level() > 0));
    }

    #[test]
    fn close_camera_reaches_level_twelve_without_exceeding_it() {
        let distance = SURFACE_RADIUS + crate::camera::MIN_CAMERA_ALTITUDE;
        let selected = QuadtreeSelection::default().select(view(Vec3::Z * distance, 800.0));
        assert_eq!(selected.iter().map(|tile| tile.level()).max(), Some(12));
        assert!(
            selected
                .iter()
                .all(|tile| tile.level() <= TERRAIN_MAX_TILE_LEVEL)
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
    fn every_visible_leaf_meets_the_projected_size_limit() {
        let view = view(Vec3::new(0.7, 0.3, 1.0).normalize() * 1.08, 900.0);
        let selected = QuadtreeSelection::default().select(view);
        for tile in selected {
            let bounds = TileBounds::new(tile);
            if bounds.may_be_visible(view) && tile.level() < TERRAIN_MAX_TILE_LEVEL {
                assert!(
                    bounds.projected_diameter(view) < TILE_SPLIT_PROJECTED_PIXELS,
                    "visible leaf {tile:?} exceeds the split threshold"
                );
            }
        }
    }

    #[test]
    fn selected_neighbors_differ_by_at_most_one_level_across_faces() {
        let view = view(Vec3::new(1.0, 0.8, 0.9).normalize() * 1.03, 900.0);
        let selected = QuadtreeSelection::default().select(view);
        let leaves = selected.iter().copied().collect::<HashSet<_>>();
        for tile in selected {
            for edge in TileEdge::ALL {
                let same_level = tile.edge_neighbor(edge);
                if let Some(neighbor) =
                    std::iter::successors(Some(same_level), |address| address.parent())
                        .find(|address| leaves.contains(address))
                {
                    assert!(tile.level().abs_diff(neighbor.level()) <= 1);
                }
            }
        }
    }

    #[test]
    fn child_morph_runs_from_split_threshold_to_twice_its_size() {
        let address = TileAddress::new(CubeFace::PositiveZ, 3, 4, 4).unwrap();
        let position = Vec3::Z * 1.4;
        let pixels_per_height =
            TileBounds::new(address.parent().unwrap()).projected_diameter(view(position, 1.0));
        let at_split = view(position, TILE_SPLIT_PROJECTED_PIXELS / pixels_per_height);
        let at_full = view(
            position,
            2.0 * TILE_SPLIT_PROJECTED_PIXELS / pixels_per_height,
        );
        assert_eq!(tile_morph_factor(address, at_split), 0.0);
        assert_eq!(tile_morph_factor(address, at_full), 1.0);
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
        const AUDIT_LEVEL: u8 = 5;
        for face in CubeFace::ALL {
            for y in 0..1_u32 << AUDIT_LEVEL {
                for x in 0..1_u32 << AUDIT_LEVEL {
                    let fine = TileAddress::new(face, AUDIT_LEVEL, x, y).unwrap();
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
                        assert!(selected.iter().any(|tile| tile.is_ancestor_of(fine)));
                    }
                }
            }
        }
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
