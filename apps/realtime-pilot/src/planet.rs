use std::{error::Error, fmt};

use procgen_core::{Vec3, hash_u32};

use crate::{
    FieldError, PRESETS, TerrainConfig, TerrainField,
    field::validate_range,
    terrain::{CAVE_DEPTH, CAVE_RADIUS, CAVE_SPACING, DENSITY_LIMIT},
};

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadialBand {
    /// Distance below the broad elevation, in model lengths.
    pub below: f32,
    /// Distance above broad elevation, at most one reference radius.
    pub above: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetConfig {
    pub radius: f32,
    pub terrain: TerrainConfig,
    pub band: RadialBand,
}

pub const PILOT_PLANET: PlanetConfig = PlanetConfig {
    radius: 4.0,
    terrain: TerrainConfig {
        height_scale: 0.5,
        detail_scale: 0.04,
        ..PRESETS[0].config
    },
    band: RadialBand {
        below: 0.4,
        above: 0.3,
    },
};

#[derive(Clone, Debug, PartialEq)]
pub enum PlanetError {
    Field(FieldError),
    Band,
    Direction,
    Resolution,
    Samples,
    Topology,
    Mesh,
    PilotRadius,
}
impl From<FieldError> for PlanetError {
    fn from(error: FieldError) -> Self {
        Self::Field(error)
    }
}
impl fmt::Display for PlanetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field(error) => error.fmt(f),
            Self::PilotRadius => write!(
                f,
                "streaming scenarios require the pilot radius {}",
                PILOT_PLANET.radius
            ),
            Self::Band => write!(
                f,
                "band must enclose detail and caves with strictly solid/empty ends, and stay outside the planet center"
            ),
            Self::Direction => write!(
                f,
                "direction must be finite and nonzero; altitude must be within the planet query bounds"
            ),
            Self::Resolution => write!(
                f,
                "face quads must be a power of two in 4..=128; radial cells in 2..=64"
            ),
            Self::Samples => write!(
                f,
                "shell samples must be finite with solid lower and empty upper ends"
            ),
            Self::Topology => write!(
                f,
                "each crossing edge must have a closed ring of three or four cells"
            ),
            Self::Mesh => write!(
                f,
                "mesh must have finite positions, valid triangles, and finite render origin"
            ),
        }
    }
}
impl Error for PlanetError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Field(error) => Some(error),
            _ => None,
        }
    }
}

impl PlanetConfig {
    pub fn validate(self, seed: u64) -> Result<PlanetField, PlanetError> {
        let terrain = self.terrain.validate(seed)?;
        validate_range("planet radius", self.radius, 1.0..=16.0)?;
        let lower = if self.terrain.cave_density > 0.0 {
            self.terrain.detail_scale.max(CAVE_DEPTH + CAVE_RADIUS)
        } else {
            self.terrain.detail_scale
        };
        if !self.band.below.is_finite()
            || !self.band.above.is_finite()
            || self.band.below <= lower
            || self.band.above <= self.terrain.detail_scale
            || self.radius - self.terrain.height_scale - self.band.below <= 0.0
            || self.band.above > self.radius
        {
            return Err(PlanetError::Band);
        }
        Ok(PlanetField {
            config: self,
            terrain,
        })
    }
}

/// Cube faces never enter this field. Positions and cave identities are planet-centered.
#[derive(Clone, Debug)]
pub struct PlanetField {
    config: PlanetConfig,
    terrain: TerrainField,
}
impl PlanetField {
    pub fn config(&self) -> PlanetConfig {
        self.config
    }

    pub fn surface_height(&self, direction: Vec3) -> Result<f32, PlanetError> {
        Ok(self.height(unit_direction(direction)?))
    }

    /// Positive-solid density at a direction and altitude above the reference sphere.
    /// Altitude must be at least -radius and at most height_scale + band.above.
    pub fn density(&self, direction: Vec3, altitude: f32) -> Result<f32, PlanetError> {
        let direction = unit_direction(direction)?;
        if !altitude.is_finite()
            || altitude < -self.config.radius
            || altitude > self.config.terrain.height_scale + self.config.band.above
        {
            return Err(PlanetError::Direction);
        }
        let column = self.column(direction);
        Ok(column.density(self, altitude - column.height))
    }

    pub(crate) fn height(&self, direction: Vec3) -> f32 {
        self.terrain.height_at(direction * self.config.radius)
    }

    pub(crate) fn column(&self, direction: Vec3) -> PlanetColumn {
        let reference = direction * self.config.radius;
        let cell =
            [reference.x, reference.y, reference.z].map(|v| (v / CAVE_SPACING).floor() as i32);
        let mut cave_distances = Vec::new();
        if self.config.terrain.cave_density > 0.0 {
            for z in -1..=1 {
                for y in -1..=1 {
                    for x in -1..=1 {
                        if let Some(candidate) =
                            self.cave_candidate([cell[0] + x, cell[1] + y, cell[2] + z])
                        {
                            cave_distances.push(reference.distance_squared(candidate));
                        }
                    }
                }
            }
        }
        PlanetColumn {
            direction,
            height: self.height(direction),
            cave_distances,
        }
    }

    fn cave_candidate(&self, cell: [i32; 3]) -> Option<Vec3> {
        let key = hash_u32(
            self.terrain.cave_key(),
            cell[0] as u32,
            cell[1] as u32,
            cell[2] as u32,
        );
        let unit = |tag| (hash_u32(key, tag, 0, 0) & 0xffff) as f32 / 65536.0;
        if unit(0) >= self.config.terrain.cave_density * CAVE_SPACING * CAVE_SPACING {
            return None;
        }
        let p = Vec3::new(
            cell[0] as f32 + unit(1),
            cell[1] as f32 + unit(2),
            cell[2] as f32 + unit(3),
        ) * CAVE_SPACING;
        let radius = p.length();
        if (radius - self.config.radius).abs() > CAVE_SPACING * 0.5 {
            return None;
        }
        Some(p * (self.config.radius / radius))
    }
}

fn unit_direction(direction: Vec3) -> Result<Vec3, PlanetError> {
    let length = direction.length();
    if !length.is_finite() || length <= 1e-9 {
        return Err(PlanetError::Direction);
    }
    Ok(direction * length.recip())
}

pub(crate) struct PlanetColumn {
    pub(crate) direction: Vec3,
    pub(crate) height: f32,
    cave_distances: Vec<f32>,
}
impl PlanetColumn {
    pub(crate) fn position(&self, field: &PlanetField, offset: f32) -> Vec3 {
        self.direction * (field.config.radius + self.height + offset)
    }
    pub(crate) fn density(&self, field: &PlanetField, offset: f32) -> f32 {
        let mut density = -offset + field.terrain.detail_at(self.position(field, offset));
        for &distance in &self.cave_distances {
            // Caves follow the broad elevation: chord distance horizontally, band offset vertically.
            let chamber = CAVE_RADIUS - (distance + (offset + CAVE_DEPTH).powi(2)).sqrt();
            let shaft_end = field.config.terrain.detail_scale + CAVE_RADIUS;
            let nearest = offset.clamp(-CAVE_DEPTH, shaft_end);
            let shaft = CAVE_RADIUS / 3.0 - (distance + (offset - nearest).powi(2)).sqrt();
            density = density.min(-chamber.max(shaft));
        }
        density.clamp(-DENSITY_LIMIT, DENSITY_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::positions;
    #[test]
    fn rejects_clipped_bands_and_preserves_solid_empty_ends() {
        for band in [
            RadialBand {
                below: 0.3,
                above: 0.3,
            },
            RadialBand {
                below: 0.4,
                above: 0.01,
            },
            RadialBand {
                below: f32::NAN,
                above: 0.3,
            },
        ] {
            assert_eq!(
                PlanetConfig {
                    band,
                    ..PILOT_PLANET
                }
                .validate(42)
                .unwrap_err(),
                PlanetError::Band
            );
        }
        let field = PILOT_PLANET.validate(42).unwrap();
        for direction in positions() {
            let column = field.column(direction.normalized());
            assert!(column.density(&field, -PILOT_PLANET.band.below) > 0.0);
            assert!(column.density(&field, PILOT_PLANET.band.above) < 0.0);
        }
    }
    #[test]
    fn cave_search_matches_wider_support() {
        let field = PILOT_PLANET.validate(42).unwrap();
        for direction in positions().map(Vec3::normalized) {
            let mut wide = field.column(direction);
            let reference = direction * PILOT_PLANET.radius;
            let cell =
                [reference.x, reference.y, reference.z].map(|v| (v / CAVE_SPACING).floor() as i32);
            wide.cave_distances.clear();
            for z in -3..=3 {
                for y in -3..=3 {
                    for x in -3..=3 {
                        if let Some(p) =
                            field.cave_candidate([cell[0] + x, cell[1] + y, cell[2] + z])
                        {
                            wide.cave_distances.push(reference.distance_squared(p));
                        }
                    }
                }
            }
            let local = field.column(direction);
            for offset in [-0.4, -0.2, 0.0, 0.2, 0.3] {
                assert_eq!(local.density(&field, offset), wide.density(&field, offset));
            }
        }
    }
}
