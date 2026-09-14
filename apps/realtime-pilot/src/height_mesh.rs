//! Canonical filtered height tiles. The CPU path serves reproducible audits.
use crate::PlanetDesignField;
use crate::height_tile::{HEIGHT_QUADS, HEIGHT_SIDE, HEIGHT_VERTEX_COUNT, HeightTile};
use bytemuck::{Pod, Zeroable};
use procgen_cubesphere::{TILE_QUADS, vertex_spacing};
use rayon::prelude::*;
pub const HEIGHT_TILE_BYTES: u64 = HEIGHT_VERTEX_COUNT as u64 * size_of::<HeightVertex>() as u64;
/// Central differences of the locally fixed-band field, down to one meter.
pub(crate) const HEIGHT_NORMAL_MIN_STEP_M: f32 = 1.0;
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct HeightVertex {
    /// Planet-centered integer base; w is the tile level.
    pub anchor: [i32; 4],
    /// Radial residual and height displacement; w is the sample spacing.
    pub offset: [f32; 4],
    /// Unit outward normal; w is the surface altitude in meters, before any draw-time bias.
    pub normal: [f32; 4],
}
pub fn height_tile_vertices(field: &PlanetDesignField, tile: HeightTile) -> Vec<HeightVertex> {
    let address = tile.address();
    let radius = field.config().radius_m;
    let spacing = vertex_spacing(address.level()) * radius * (TILE_QUADS / HEIGHT_QUADS) as f32;
    (0..HEIGHT_VERTEX_COUNT)
        .into_par_iter()
        .map(|i| {
            let [x, y] = tile.grid_coordinates(i);
            let d = address
                .grid_vertex(
                    x * (TILE_QUADS / HEIGHT_QUADS),
                    y * (TILE_QUADS / HEIGHT_QUADS),
                )
                .unwrap()
                .direction();
            let footprint = tile.footprint_m([x, y], radius);
            let surface_height = field.height(d, footprint);
            let mut result = HeightVertex {
                anchor: [0, 0, 0, address.level() as i32],
                offset: [0.0, 0.0, 0.0, spacing],
                normal: [0.0; 4],
            };
            let n = height_normal(field, d, footprint, surface_height);
            result.normal = [n.x, n.y, n.z, surface_height];
            for (axis, value) in [d.x, d.y, d.z].into_iter().enumerate() {
                result.anchor[axis] = (value * radius).floor() as i32;
                result.offset[axis] =
                    value.mul_add(radius, -(result.anchor[axis] as f32)) + value * surface_height;
            }
            result
        })
        .collect()
}

fn height_normal(
    field: &PlanetDesignField,
    direction: procgen_core::Vec3,
    footprint: f32,
    height: f32,
) -> procgen_core::Vec3 {
    use procgen_core::Vec3;
    let radius = field.config().radius_m;
    let step = footprint.max(HEIGHT_NORMAL_MIN_STEP_M);
    let sample = |d: Vec3| {
        let d = d.normalized();
        field.height(d, footprint)
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
/// Shared grid; stitched edge vertices collapse redundant triangles in place.
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
    result
}

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
                let footprint = 0.25;
                let n = height_normal(&field, d, footprint, field.height(d, footprint));
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
                        let r = f64::from(config.radius_m) + f64::from(field.height(q, footprint));
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
