use procgen_core::{Vec3, hash_u32};
use procgen_noise::fold_seed_u64_to_u32;

use crate::{
    field::{FieldError, validate_position, validate_range},
    noise::{NoiseConfig, normalized_noise, uber_noise},
};

const CAVE_SPACING: f32 = 0.5;
const CAVE_RADIUS: f32 = 0.11;
const CAVE_DEPTH: f32 = 0.2;
const DETAIL_WAVELENGTH: f32 = 0.16;
// Truncation makes the influence of distant cave candidates exactly finite.
const DENSITY_LIMIT: f32 = CAVE_SPACING * 0.25;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainConfig {
    pub noise: NoiseConfig,
    /// Maximum absolute base-surface elevation, in model lengths.
    pub height_scale: f32,
    /// Maximum additive density displacement, in model lengths.
    pub detail_scale: f32,
    /// Expected cave candidates per square model length.
    pub cave_density: f32,
}

impl TerrainConfig {
    pub const HEIGHT_RANGE: std::ops::RangeInclusive<f32> = 0.0..=4.0;
    pub const DETAIL_RANGE: std::ops::RangeInclusive<f32> = 0.0..=0.25;
    pub const CAVE_DENSITY_RANGE: std::ops::RangeInclusive<f32> =
        0.0..=1.0 / (CAVE_SPACING * CAVE_SPACING);
    pub fn validate(self, seed: u64) -> Result<TerrainField, FieldError> {
        self.noise.validate()?;
        validate_range("height scale", self.height_scale, Self::HEIGHT_RANGE)?;
        validate_range("detail scale", self.detail_scale, Self::DETAIL_RANGE)?;
        validate_range("cave density", self.cave_density, Self::CAVE_DENSITY_RANGE)?;
        let root = fold_seed_u64_to_u32(seed);
        Ok(TerrainField {
            config: self,
            surface_key: hash_u32(root, 0x5355_5246, 0, 0),
            detail_key: hash_u32(root, 0x4445_544c, 0, 0),
            cave_key: hash_u32(root, 0x4341_5645, 0, 0),
        })
    }
}

/// Immutable, validated local field. Queries never read a generated volume.
#[derive(Clone, Debug)]
pub struct TerrainField {
    config: TerrainConfig,
    surface_key: u32,
    detail_key: u32,
    cave_key: u32,
}

impl TerrainField {
    pub fn surface_height(&self, x: f32, z: f32) -> Result<f32, FieldError> {
        validate_position(Vec3::new(x, 0.0, z))?;
        Ok(self.height(x, z))
    }

    /// Positive is solid. Magnitudes are truncated away from the surface.
    /// This is a density, not a signed distance or gradient.
    pub fn density(&self, position: Vec3) -> Result<f32, FieldError> {
        validate_position(position)?;
        Ok(self
            .column(position.x, position.z)
            .density(self, position.y))
    }

    fn height(&self, x: f32, z: f32) -> f32 {
        uber_noise(self.surface_key, Vec3::new(x, 0.0, z), self.config.noise)
            * self.config.height_scale
    }

    pub(crate) fn column(&self, x: f32, z: f32) -> Column {
        let height = self.height(x, z);
        let mut caves = Vec::new();
        if self.config.cave_density > 0.0 {
            let cell_x = (x / CAVE_SPACING).floor() as i32;
            let cell_z = (z / CAVE_SPACING).floor() as i32;
            // Chamber + entrance + density truncation reach less than one cell
            // in either horizontal axis, so omitted candidates cannot affect a query.
            for dz in -1..=1 {
                for dx in -1..=1 {
                    if let Some(cave) = self.cave(cell_x + dx, cell_z + dz) {
                        caves.push(cave);
                    }
                }
            }
        }
        Column {
            x,
            z,
            height,
            caves,
        }
    }

    fn cave(&self, x: i32, z: i32) -> Option<Cave> {
        let bits = hash_u32(self.cave_key, x as u32, z as u32, 0);
        let occurrence = (bits & 0xffff) as f32 / 65536.0;
        if occurrence >= self.config.cave_density * CAVE_SPACING * CAVE_SPACING {
            return None;
        }
        let jitter = hash_u32(self.cave_key, x as u32, z as u32, 1);
        let cx = (x as f32 + 0.2 + 0.6 * (jitter & 0xffff) as f32 / 65536.0) * CAVE_SPACING;
        let cz = (z as f32 + 0.2 + 0.6 * (jitter >> 16) as f32 / 65536.0) * CAVE_SPACING;
        let center = Vec3::new(cx, self.height(cx, cz) - CAVE_DEPTH, cz);
        let direction = [Vec3::X, Vec3::Z, -Vec3::X, -Vec3::Z][(bits >> 30) as usize];
        let end = center + direction * CAVE_RADIUS;
        let entrance = Vec3::new(
            end.x,
            self.height(end.x, end.z) + self.config.detail_scale + CAVE_RADIUS,
            end.z,
        );
        Some(Cave { center, entrance })
    }
}

/// Per-column immutable data, shared by the direct and batched query paths.
pub(crate) struct Column {
    x: f32,
    z: f32,
    pub(crate) height: f32,
    caves: Vec<Cave>,
}

impl Column {
    pub(crate) fn density(&self, field: &TerrainField, y: f32) -> f32 {
        let position = Vec3::new(self.x, y, self.z);
        let detail = normalized_noise(field.detail_key, position * DETAIL_WAVELENGTH.recip()).value;
        let mut density = self.height - y + detail * field.config.detail_scale;
        for cave in &self.caves {
            density = density.min(-cave.density(position));
        }
        density.clamp(-DENSITY_LIMIT, DENSITY_LIMIT)
    }
}

struct Cave {
    center: Vec3,
    entrance: Vec3,
}

impl Cave {
    fn density(&self, position: Vec3) -> f32 {
        let chamber = CAVE_RADIUS - (position - self.center).length();
        let axis = self.entrance - self.center;
        // Projection onto the closed segment is part of the capsule definition.
        let t = ((position - self.center).dot(axis) / axis.length_squared()).clamp(0.0, 1.0);
        let shaft = CAVE_RADIUS / 3.0 - (position - (self.center + axis * t)).length();
        chamber.max(shaft)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PRESETS, test_support::positions};
    use procgen_core::quantized_fingerprint;

    #[test]
    fn queries_are_repeatable_in_shuffled_order_and_seeded_by_both_halves() {
        for preset in &PRESETS {
            let field = preset.config.validate(42).unwrap();
            let points: Vec<_> = positions().collect();
            let expected: Vec<_> = points.iter().map(|&p| field.density(p).unwrap()).collect();
            for i in (0..points.len()).map(|i| (i * 37) % points.len()) {
                assert_eq!(field.density(points[i]).unwrap(), expected[i]);
            }
            let changed = preset.config.validate(42 + (1 << 32)).unwrap();
            assert_ne!(
                quantized_fingerprint(expected),
                quantized_fingerprint(points.iter().map(|&p| changed.density(p).unwrap()))
            );
        }
    }

    #[test]
    fn field_has_bounded_solid_and_empty_envelopes() {
        for preset in &PRESETS {
            let field = preset.config.validate(u64::MAX).unwrap();
            for p in positions() {
                let h = field.surface_height(p.x, p.z).unwrap();
                assert!(h.abs() <= preset.config.height_scale);
                assert!(
                    field
                        .density(Vec3::new(p.x, preset.config.height_scale + 1.0, p.z))
                        .unwrap()
                        < 0.0
                );
                assert!(
                    field
                        .density(Vec3::new(p.x, -preset.config.height_scale - 1.0, p.z))
                        .unwrap()
                        > 0.0
                );
                assert!(field.density(p).unwrap().is_finite());
            }
        }
    }

    #[test]
    fn cave_chambers_and_entrances_are_carved_without_moving_base_surface() {
        let config = TerrainConfig {
            cave_density: 4.0,
            detail_scale: 0.0,
            ..PRESETS[0].config
        };
        let field = config.validate(42).unwrap();
        let uncarved = TerrainConfig {
            cave_density: 0.0,
            ..config
        }
        .validate(42)
        .unwrap();
        for z in -2..2 {
            for x in -2..2 {
                let cave = field.cave(x, z).unwrap();
                assert!(uncarved.density(cave.center).unwrap() > 0.0);
                assert!(field.density(cave.center).unwrap() < 0.0);
                for i in 0..=20 {
                    let p = cave.center + (cave.entrance - cave.center) * (i as f32 / 20.0);
                    assert!(field.density(p).unwrap() < 0.0, "blocked entrance: {p:?}");
                }
                assert_eq!(
                    field.height(cave.center.x, cave.center.z),
                    uncarved.height(cave.center.x, cave.center.z)
                );
            }
        }
    }

    #[test]
    fn nearby_candidates_agree_with_a_wider_search_at_grid_boundaries() {
        let field = PRESETS[1].config.validate(42).unwrap();
        for x in [-0.50001, -0.49999, -0.00001, 0.00001, 0.49999, 0.50001] {
            for y in [-1.0, -0.1, 0.4, 0.8] {
                let mut wide = field.column(x, 0.21);
                let field_ref = &field;
                wide.caves = (-4..=4)
                    .flat_map(|z| (-4..=4).filter_map(move |cx| field_ref.cave(cx, z)))
                    .collect();
                let local = field.density(Vec3::new(x, y, 0.21)).unwrap();
                assert_eq!(local, wide.density(&field, y));
            }
        }
    }

    #[test]
    fn config_and_query_errors_are_explicit() {
        for invalid in [f32::NAN, f32::INFINITY, -1.0, 1.1] {
            let config = TerrainConfig {
                detail_scale: invalid,
                ..PRESETS[0].config
            };
            assert!(config.validate(0).is_err());
        }
        let config = TerrainConfig {
            noise: NoiseConfig {
                wavelength: 0.0,
                ..PRESETS[0].config.noise
            },
            ..PRESETS[0].config
        };
        assert!(config.validate(0).is_err());
        let field = PRESETS[0].config.validate(0).unwrap();
        for p in [Vec3::new(f32::NAN, 0.0, 0.0), Vec3::X * 9.0] {
            assert_eq!(field.density(p), Err(FieldError::Position));
        }
    }
}
