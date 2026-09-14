//! Canonical filtered height tiles. The CPU path serves reproducible audits.
use crate::PlanetDesignField;
use bytemuck::{Pod, Zeroable};
use procgen_cubesphere::{TILE_QUADS, TileAddress, vertex_spacing};
use rayon::prelude::*;
pub const HEIGHT_QUADS: u32 = TILE_QUADS / 2;
pub const HEIGHT_SIDE: u32 = HEIGHT_QUADS + 1;
pub const HEIGHT_VERTEX_COUNT: u32 = HEIGHT_SIDE * HEIGHT_SIDE + 4 * HEIGHT_SIDE;
pub const HEIGHT_TILE_BYTES: u64 = HEIGHT_VERTEX_COUNT as u64 * size_of::<HeightVertex>() as u64;
/// Central differences resolve the filtered field, down to the finest voxel.
pub(crate) const HEIGHT_NORMAL_MIN_STEP_M: f32 = 1.0;
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct HeightVertex {
    /// Planet-centered integer base; w is the tile level.
    pub anchor: [i32; 4],
    /// Radial residual and height displacement; w is the sample spacing.
    pub offset: [f32; 4],
    /// Unit outward normal; w is the surface altitude in meters, including on skirts.
    pub normal: [f32; 4],
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
            let surface_height = field.height(d, filter.spacing_m(d, radius));
            let h = if skirt {
                -field.config().height_limit_m - spacing * 2.0
            } else {
                surface_height
            };
            let mut result = HeightVertex {
                anchor: [0, 0, 0, tile.level() as i32],
                offset: [0.0, 0.0, 0.0, spacing],
                normal: [0.0; 4],
            };
            // Skirts inherit their top vertex's normal, avoiding dark curtains.
            let n = height_normal(field, d, filter, surface_height);
            result.normal = [n.x, n.y, n.z, surface_height];
            for (axis, value) in [d.x, d.y, d.z].into_iter().enumerate() {
                result.anchor[axis] = (value * radius).floor() as i32;
                result.offset[axis] =
                    value.mul_add(radius, -(result.anchor[axis] as f32)) + value * h;
            }
            result
        })
        .collect()
}

fn height_normal(
    field: &PlanetDesignField,
    direction: procgen_core::Vec3,
    filter: HeightFilter,
    height: f32,
) -> procgen_core::Vec3 {
    use procgen_core::Vec3;
    let radius = field.config().radius_m;
    let step = filter
        .spacing_m(direction, radius)
        .max(HEIGHT_NORMAL_MIN_STEP_M);
    let sample = |d: Vec3| {
        let d = d.normalized();
        field.height(d, filter.spacing_m(d, radius))
    };
    let derivative = |axis| {
        let delta = axis * (step / radius);
        (sample(direction + delta) - sample(direction - delta)) / (2.0 * step)
    };
    let gradient = Vec3::new(
        derivative(Vec3::X),
        derivative(Vec3::Y),
        derivative(Vec3::Z),
    );
    let tangent = gradient - direction * gradient.dot(direction);
    (direction - tangent * (radius / (radius + height))).normalized()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlanetDesignConfig;
    use procgen_core::Vec3;

    #[test]
    fn height_normals_follow_the_sphere_and_displaced_surface() {
        let mut config = PlanetDesignConfig::starter(42);
        config.radius_m = 300_000.0;
        config.octaves.truncate(1);
        config.octaves[0].wavelength_m = 65_536.0;
        config.octaves[0].perturbation = 0.0;
        config.octaves[0].slope_erosion = 0.0;
        config.octaves[0].ridge_erosion = 0.0;
        let directions = [
            Vec3::X,
            -Vec3::X,
            Vec3::Y,
            -Vec3::Y,
            Vec3::Z,
            -Vec3::Z,
            Vec3::new(1.0, 1.0, 1.0).normalized(),
            Vec3::new(-1.0, 1.0, 0.3).normalized(),
        ];
        for amplitude in [0.0, 4_000.0] {
            config.octaves[0].amplitude_m = amplitude;
            let field = config.validate().unwrap();
            for d in directions {
                let filter = HeightFilter {
                    surface_m: [
                        d.x * config.radius_m,
                        d.y * config.radius_m,
                        d.z * config.radius_m,
                    ],
                    clearance_m: 0.0,
                };
                let n = height_normal(
                    &field,
                    d,
                    filter,
                    field.height(d, filter.spacing_m(d, config.radius_m)),
                );
                assert!(n.is_finite() && (n.length() - 1.0).abs() < 0.00001);
                assert!(n.dot(d) > 0.0, "outward on every cube face");
                if amplitude == 0.0 {
                    assert!((n - d).length() < 0.00001, "sphere normal must be radial");
                } else {
                    // Independent surface secants in a tangent frame, assembled in
                    // f64 to avoid subtracting planetary positions in f32.
                    let axis = if d.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
                    let u = axis.cross(d).normalized();
                    let v = d.cross(u);
                    let surface = |t: Vec3| {
                        let q = (d + t * (8.0 / config.radius_m)).normalized();
                        let r = f64::from(config.radius_m)
                            + f64::from(field.height(q, filter.spacing_m(q, config.radius_m)));
                        [f64::from(q.x) * r, f64::from(q.y) * r, f64::from(q.z) * r]
                    };
                    for tangent in [u, v] {
                        let a = surface(tangent);
                        let b = surface(-tangent);
                        let edge: [f64; 3] = std::array::from_fn(|i| a[i] - b[i]);
                        let length = edge.iter().map(|v| v * v).sum::<f64>().sqrt();
                        let dot = f64::from(n.x) * edge[0]
                            + f64::from(n.y) * edge[1]
                            + f64::from(n.z) * edge[2];
                        assert!(
                            dot.abs() / length < 0.01,
                            "normal must be perpendicular to the smooth displaced surface"
                        );
                    }
                }
            }
        }
    }
}
