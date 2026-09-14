//! Conservative draw bounds. These do not change coverage or generated geometry.
use bevy::{
    camera::primitives::{Aabb, Frustum},
    math::{Affine3A, DVec3, Mat3A, Vec3A},
    prelude::Vec3,
};
use procgen_cubesphere::{TILE_QUADS, TileAddress};
use procgen_realtime_pilot::PlanetDesignField;

/// A radial box enclosing the tile's full permitted height range and all its
/// triangles. Store it with the immutable GPU buffer, not with camera state.
pub struct HeightBounds {
    center: DVec3,
    axes: Mat3A,
    half_extents: Vec3A,
}
impl HeightBounds {
    pub fn new(address: TileAddress, field: &PlanetDesignField) -> Self {
        let direction = |x, y| {
            let d = address.grid_vertex(x, y).unwrap().direction();
            DVec3::new(d.x as f64, d.y as f64, d.z as f64).normalize()
        };
        let radial = direction(TILE_QUADS / 2, TILE_QUADS / 2);
        // The projection of a rectangular cube patch is contained in the cone
        // from its center through its farthest corner. Along each edge the dot
        // with the center has no interior minimum (all directions share a face).
        // Cross products retain small angles without subtracting nearly equal dots.
        let sine = [
            [0, 0],
            [TILE_QUADS, 0],
            [0, TILE_QUADS],
            [TILE_QUADS, TILE_QUADS],
        ]
        .into_iter()
        .map(|[x, y]| radial.cross(direction(x, y)).length())
        .fold(0.0_f64, f64::max);
        let cosine = (1.0 - sine * sine).sqrt();
        let radius = f64::from(field.config().radius_m);
        let height = f64::from(field.config().height_limit_m);
        let outer = radius + height;
        let lower = (radius - height) * cosine;
        let center = radial * ((outer + lower) * 0.5);
        let frame_u = address.face().frame().u_axis;
        let u = DVec3::new(frame_u.x as f64, frame_u.y as f64, frame_u.z as f64);
        let tangent = (u - radial * u.dot(radial)).normalize();
        let axes = Mat3A::from_cols(
            tangent.as_vec3().into(),
            radial.cross(tangent).as_vec3().into(),
            radial.as_vec3().into(),
        );
        // Allow 16 f32 epsilons at the outer radius for CPU/WGSL direction,
        // anchor/offset reconstruction, and the camera-relative frustum test.
        let padding = (outer * 16.0 * f64::from(f32::EPSILON)) as f32;
        let half_extents = Vec3A::new(
            (outer * sine) as f32,
            (outer * sine) as f32,
            ((outer - lower) * 0.5) as f32,
        ) + Vec3A::splat(padding);
        Self {
            center,
            axes,
            half_extents,
        }
    }

    fn relative_transform(&self, anchor: [i32; 4]) -> Affine3A {
        Affine3A {
            matrix3: self.axes,
            translation: (self.center
                - DVec3::new(anchor[0] as f64, anchor[1] as f64, anchor[2] as f64))
            .as_vec3()
            .into(),
        }
    }

    /// Frustum rejection and front-to-back priority use the same conservative box.
    /// Eye and frustum are relative to the viewer's integer anchor.
    pub fn visible_distance(&self, frustum: &Frustum, anchor: [i32; 4], eye: Vec3) -> Option<f32> {
        let transform = self.relative_transform(anchor);
        let aabb = Aabb {
            center: Vec3A::ZERO,
            half_extents: self.half_extents,
        };
        if !frustum.intersects_obb(&aabb, &transform, true, true) {
            return None;
        }
        let local_eye = self.axes.transpose() * (Vec3A::from(eye) - transform.translation);
        let outside = (local_eye.abs() - self.half_extents).max(Vec3A::ZERO);
        Some(outside.length_squared())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::{Mat4, Transform};
    use procgen_cubesphere::{CubeFace, MAX_TILE_LEVEL};
    use procgen_realtime_pilot::{HeightTile, PlanetDesignConfig, height_tile_vertices};

    fn contains(bounds: &HeightBounds, position: DVec3) -> bool {
        let p = bounds.axes.transpose() * (position - bounds.center).as_vec3();
        p.abs().cmple(bounds.half_extents.into()).all()
    }

    #[test]
    fn radial_boxes_contain_grid_and_height_extremes_on_all_faces_and_levels() {
        let mut config = PlanetDesignConfig::starter(42);
        config.height_limit_m = *PlanetDesignConfig::HEIGHT_LIMIT_RANGE.end();
        for radius in [
            *PlanetDesignConfig::RADIUS_RANGE.start(),
            *PlanetDesignConfig::RADIUS_RANGE.end(),
        ] {
            config.radius_m = radius;
            let field = config.validate().unwrap();
            for face in CubeFace::ALL {
                for level in 0..=MAX_TILE_LEVEL {
                    let n = 1 << level;
                    for [x, y] in [[0, 0], [n / 2, n / 2], [n - 1, 0], [n - 1, n - 1]] {
                        let tile = TileAddress::new(face, level, x, y).unwrap();
                        let bounds = HeightBounds::new(tile, &field);
                        for y in 0..=TILE_QUADS {
                            for x in 0..=TILE_QUADS {
                                let d = tile.grid_vertex(x, y).unwrap().direction();
                                for h in [-config.height_limit_m, config.height_limit_m] {
                                    // Test the full height envelope, independent of noise.
                                    let point = DVec3::new(d.x as f64, d.y as f64, d.z as f64)
                                        * (radius + h) as f64;
                                    assert!(
                                        contains(&bounds, point),
                                        "{tile:?} [{x},{y}] at {radius}, {h}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn bounds_contain_generated_stitched_mesh_and_triangle_interiors() {
        let field = PlanetDesignConfig::starter(42).validate().unwrap();
        for face in CubeFace::ALL {
            let tile = HeightTile::new(
                TileAddress::new(face, 8, 91, 130).unwrap(),
                procgen_cubesphere::FaceEdge::ALL,
            );
            let bounds = HeightBounds::new(tile.address(), &field);
            let vertices = height_tile_vertices(&field, tile);
            let positions: Vec<_> = vertices
                .iter()
                .map(|v| {
                    DVec3::new(
                        v.anchor[0] as f64 + v.offset[0] as f64,
                        v.anchor[1] as f64 + v.offset[1] as f64,
                        v.anchor[2] as f64 + v.offset[2] as f64,
                    )
                })
                .collect();
            assert!(positions.iter().all(|&p| contains(&bounds, p)));
            for triangle in procgen_realtime_pilot::height_indices().chunks_exact(3) {
                let p = (positions[triangle[0] as usize]
                    + positions[triangle[1] as usize]
                    + positions[triangle[2] as usize])
                    / 3.0;
                assert!(contains(&bounds, p));
            }
        }
    }

    #[test]
    fn visibility_rejects_sideways_tiles_but_keeps_visible_extreme_heights() {
        let mut config = PlanetDesignConfig::starter(42);
        config.radius_m = 300_000.;
        let field = config.validate().unwrap();
        let tile = TileAddress::new(CubeFace::PositiveX, 12, 2300, 2048).unwrap();
        let bounds = HeightBounds::new(tile, &field);
        let anchor = [300_000, 0, 0, 0];
        let eye = Vec3::new(5., 0., 0.);
        let projection = Mat4::perspective_infinite_reverse_rh(1.0, 1.5, 0.1);
        let camera = Transform::from_translation(eye).looking_to(-Vec3::X, Vec3::Y);
        let frustum = Frustum::from_clip_from_world(&(projection * camera.to_matrix().inverse()));
        assert!(bounds.visible_distance(&frustum, anchor, eye).is_none());
        for height in [-config.height_limit_m, config.height_limit_m] {
            let d = tile
                .grid_vertex(TILE_QUADS / 2, TILE_QUADS / 2)
                .unwrap()
                .direction();
            let target =
                Vec3::new(d.x, d.y, d.z) * (config.radius_m + height) - Vec3::new(300_000., 0., 0.);
            let camera = Transform::from_translation(eye).looking_at(target, Vec3::Y);
            let frustum =
                Frustum::from_clip_from_world(&(projection * camera.to_matrix().inverse()));
            assert!(bounds.visible_distance(&frustum, anchor, eye).is_some());
        }
    }
}
