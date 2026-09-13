//! Bounded CPU height previews. These are not voxel meshes or collision data.
use std::{error::Error, fmt};

use procgen_core::Vec3;
use rayon::prelude::*;

use crate::{
    DesignError, FieldError, HeightDistribution, PlanetDesignConfig, field::validate_range,
    planet_design::octave_weight, shell::face_grid,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PreviewArea {
    Planet,
    Patch {
        latitude_deg: f32,
        longitude_deg: f32,
        span_m: f32,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewBands {
    Combined,
    Only(usize),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignPreviewConfig {
    pub area: PreviewArea,
    pub bands: PreviewBands,
    pub quads: usize,
}
impl DesignPreviewConfig {
    pub const PATCH_SPAN_RANGE: std::ops::RangeInclusive<f32> = 128.0..=100_000.0;
}

#[derive(Debug)]
pub enum PreviewError {
    Design(DesignError),
    Resolution,
    Band,
    Mesh,
}
impl From<DesignError> for PreviewError {
    fn from(e: DesignError) -> Self {
        Self::Design(e)
    }
}
impl From<FieldError> for PreviewError {
    fn from(e: FieldError) -> Self {
        Self::Design(e.into())
    }
}
impl fmt::Display for PreviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Design(e) => e.fmt(f),
            Self::Resolution => write!(f, "preview quads must be a power of two in 16..=256"),
            Self::Band => write!(f, "selected octave does not exist"),
            Self::Mesh => write!(f, "invalid preview mesh"),
        }
    }
}
impl Error for PreviewError {}

pub struct DesignPreview {
    /// Planet-centered for the globe; tangent frame at patch center otherwise.
    /// Patch Y is radial up; the center surface has Y=0.
    positions_m: Vec<Vec3>,
    triangles: Vec<[u32; 3]>,
    heights_m: Vec<f32>,
    pub spacing_m: f32,
    pub octave_weights: Vec<f32>,
    /// Full-resolution, equal-area planetary distribution for the selected bands.
    pub distribution: HeightDistribution,
    pub extent_m: f32,
}
impl DesignPreview {
    pub fn positions_m(&self) -> &[Vec3] {
        &self.positions_m
    }
    pub fn triangles(&self) -> &[[u32; 3]] {
        &self.triangles
    }
    pub fn heights_m(&self) -> &[f32] {
        &self.heights_m
    }
    pub fn validate(&self) -> Result<(), PreviewError> {
        if self.positions_m.is_empty()
            || self.positions_m.len() != self.heights_m.len()
            || self.positions_m.iter().any(|p| !p.is_finite())
            || self.heights_m.iter().any(|h| !h.is_finite())
            || self
                .triangles
                .iter()
                .any(|t| t.iter().any(|&i| i as usize >= self.positions_m.len()))
        {
            return Err(PreviewError::Mesh);
        }
        Ok(())
    }
}
struct Vertex {
    position: Vec3,
    height: f32,
}

pub fn generate_design_preview(
    config: &PlanetDesignConfig,
    preview: DesignPreviewConfig,
) -> Result<DesignPreview, PreviewError> {
    // Validate the saved config even if solo mode would hide a bad band.
    let mut field = config.validate()?;
    if let PreviewBands::Only(index) = preview.bands {
        if index >= config.octaves.len() {
            return Err(PreviewError::Band);
        }
        let mut solo = config.clone();
        for (i, o) in solo.octaves.iter_mut().enumerate() {
            o.enabled = i == index;
        }
        field = solo.validate()?;
    }
    let n = preview.quads;
    if !n.is_power_of_two() || !(16..=256).contains(&n) {
        return Err(PreviewError::Resolution);
    }
    let radius = config.radius_m;
    let (vertices, triangles, spacing_m, extent_m) = match preview.area {
        PreviewArea::Planet => {
            let (directions, quads) = face_grid(n);
            // Conservative reference-sphere spacing, identical at shared edges.
            let spacing = 2.0 * radius / n as f32;
            let vertices: Vec<_> = directions
                .par_iter()
                .map(|&direction| {
                    let height = field.height(direction, spacing);
                    Vertex {
                        position: direction * (radius + height),
                        height,
                    }
                })
                .collect();
            let triangles = quads
                .into_iter()
                .flat_map(|q| [[0, 1, 3], [0, 3, 2]].map(|t| t.map(|i| q.columns[i] as u32)))
                .collect();
            (vertices, triangles, spacing, radius + config.height_limit_m)
        }
        PreviewArea::Patch {
            latitude_deg,
            longitude_deg,
            span_m,
        } => {
            validate_range("latitude", latitude_deg, -90.0..=90.0)?;
            validate_range("longitude", longitude_deg, -180.0..=180.0)?;
            validate_range(
                "patch span (m)",
                span_m,
                DesignPreviewConfig::PATCH_SPAN_RANGE,
            )?;
            let latitude = latitude_deg.to_radians();
            let longitude = longitude_deg.to_radians();
            let up = Vec3::new(
                latitude.cos() * longitude.cos(),
                latitude.sin(),
                latitude.cos() * longitude.sin(),
            );
            let east = Vec3::new(-longitude.sin(), 0.0, longitude.cos());
            let north = east.cross(up);
            let spacing = span_m / n as f32;
            let center_height = field.height(up, spacing);
            let vertices: Vec<_> = (0..(n + 1).pow(2))
                .into_par_iter()
                .map(|i| {
                    let x = (i % (n + 1)) as f32 / n as f32 * span_m - span_m * 0.5;
                    let z = (i / (n + 1)) as f32 / n as f32 * span_m - span_m * 0.5;
                    let distance = (radius * radius + x * x + z * z).sqrt();
                    let direction = (up * radius + east * x + north * z) * distance.recip();
                    let height = field.height(direction, spacing);
                    // Compute relative curvature without subtracting two planet radii.
                    let sag = -radius * (x * x + z * z) / (distance * (distance + radius));
                    Vertex {
                        position: Vec3::new(
                            x * ((radius + height) / distance),
                            sag + height * (radius / distance) - center_height,
                            z * ((radius + height) / distance),
                        ),
                        height,
                    }
                })
                .collect();
            let triangles = (0..n)
                .flat_map(|z| {
                    (0..n).flat_map(move |x| {
                        let a = (z * (n + 1) + x) as u32;
                        let b = a + 1;
                        let c = a + (n + 1) as u32;
                        [[a, c, b], [b, c, c + 1]]
                    })
                })
                .collect();
            let extent = vertices
                .iter()
                .map(|v| v.position.length())
                .fold(0.0, f32::max);
            (vertices, triangles, spacing, extent)
        }
    };
    let result = DesignPreview {
        positions_m: vertices.iter().map(|v| v.position).collect(),
        heights_m: vertices.into_iter().map(|v| v.height).collect(),
        triangles,
        spacing_m,
        extent_m,
        octave_weights: field
            .config()
            .octaves
            .iter()
            .map(|o| {
                if o.enabled && o.amplitude_m > 0.0 {
                    octave_weight(o.wavelength_m, spacing_m)
                } else {
                    0.0
                }
            })
            .collect(),
        distribution: field.height_distribution(),
    };
    result.validate()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn globe_is_closed_and_patch_has_curvature_without_large_origin_cancellation() {
        let mut config = PlanetDesignConfig::starter(42);
        let settings = DesignPreviewConfig {
            area: PreviewArea::Planet,
            bands: PreviewBands::Combined,
            quads: 16,
        };
        let globe = generate_design_preview(&config, settings).unwrap();
        let mut edges = std::collections::BTreeMap::new();
        for t in globe.triangles() {
            let [a, b, c] = t.map(|i| globe.positions_m()[i as usize]);
            assert!((b - a).cross(c - a).dot(a) > 0.0);
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let entry = edges.entry([a.min(b), a.max(b)]).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += if a < b { 1 } else { -1 };
            }
        }
        assert!(edges.values().all(|&v| v == (2, 0)));
        for o in &mut config.octaves {
            o.enabled = false;
        }
        let patch = generate_design_preview(
            &config,
            DesignPreviewConfig {
                area: PreviewArea::Patch {
                    latitude_deg: 35.0,
                    longitude_deg: 70.0,
                    span_m: 128.0,
                },
                ..settings
            },
        )
        .unwrap();
        assert!(patch.positions_m()[0].y < -0.001);
        assert_eq!(patch.positions_m()[8 * 17 + 8], Vec3::ZERO);
        for t in patch.triangles() {
            let [a, b, c] = t.map(|i| patch.positions_m()[i as usize]);
            assert!((b - a).cross(c - a).y > 0.0);
        }
    }
}
