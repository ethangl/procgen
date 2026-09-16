//! Physical broad terrain for the planet-scale pilot. No density or LOD storage.
use std::{error::Error, fmt};

use procgen_core::{Vec3, hash_u32};
use procgen_noise::fold_seed_u64_to_u32;
use serde::{Deserialize, Serialize};

use crate::{
    FieldError, HeightDistribution, NoiseConfig,
    field::validate_range,
    height_distribution::sample_heights,
    noise::{normalized_noise, shape},
};

pub const MAX_DESIGN_OCTAVES: usize = 24;

/// Amplitude is in meters before feedback damping and the final height bound.
/// Zero amplitude or disabled means no contribution, including feedback.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OctaveConfig {
    pub enabled: bool,
    pub wavelength_m: f32,
    pub amplitude_m: f32,
    pub sharpness: f32,
    pub perturbation: f32,
    pub slope_erosion: f32,
    pub altitude_erosion: f32,
    pub ridge_erosion: f32,
}
impl OctaveConfig {
    pub const WAVELENGTH_RANGE: std::ops::RangeInclusive<f32> = 8.0..=16_000_000.0;
    pub const AMPLITUDE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=20_000.0;
}

/// One bounded 3D detail term added to the voxel potential, so the near field
/// can carry cliff faces, undercuts, and pinching a radial height cannot
/// express. It is faded out with height above the surface, which is what keeps
/// it from making floating blobs in the air (McKendrick 37:19-38:12). Height
/// tiles carry a first-order projection of it through `surface_height`, so the
/// far field draws the surface the band extracts; the fade itself belongs to
/// the density path, which is the only one that has a `d` to fade over.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeConfig {
    pub enabled: bool,
    pub wavelength_m: f32,
    /// A true bound on the term, not a one-sigma scale: the shaped noise passes
    /// through the design's `x / sqrt(1 + x*x)` before this scales it, so the
    /// term stays strictly inside +/- this many meters. The render bias in
    /// `local_draw_layout` depends on that.
    pub amplitude_m: f32,
    pub sharpness: f32,
    /// Height above the surface over which the term smoothly reaches zero.
    pub fade_m: f32,
}
impl Default for VolumeConfig {
    /// Disabled, but already carrying values that show something in the band
    /// the moment the panel checkbox is ticked. These are starting values, not
    /// a measured preset.
    fn default() -> Self {
        Self {
            enabled: false,
            wavelength_m: 24.0,
            amplitude_m: 6.0,
            sharpness: 0.5,
            fade_m: 12.0,
        }
    }
}
impl VolumeConfig {
    pub const WAVELENGTH_RANGE: std::ops::RangeInclusive<f32> = 4.0..=4_096.0;
    pub const AMPLITUDE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=64.0;
    pub const FADE_RANGE: std::ops::RangeInclusive<f32> = 1.0..=256.0;
    /// `p_m / wavelength_m` must stay under this, so the finest noise cell is
    /// still resolved to at least a sixteenth of its width in f32: consecutive
    /// f32 values below 2^18 are at most a thirty-secondth apart.
    pub const COORDINATE_LIMIT: f32 = 262_144.0;

    /// Zero unless enabled, so the one number callers outside the field need -
    /// how far the voxel surface can sit below the height surface - is read
    /// from one place.
    pub fn active_amplitude_m(&self) -> f32 {
        if self.enabled { self.amplitude_m } else { 0.0 }
    }
}

/// Serialized generation contract. The reference sphere is the zero-elevation
/// datum. Preview/camera preferences are not generation parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetDesignConfig {
    pub seed: u64,
    pub radius_m: f32,
    pub height_limit_m: f32,
    /// Strictly descending wavelengths; octave zero is the broadest band.
    pub octaves: Vec<OctaveConfig>,
    /// Generation data, unlike `ocean`: it is part of the field, and its
    /// default leaves every design written before it loading unchanged.
    #[serde(default)]
    pub volume: VolumeConfig,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DesignError {
    Field(FieldError),
    OctaveCount,
    WavelengthOrder,
    VolumeWavelength,
    Direction,
    Spacing,
}
impl From<FieldError> for DesignError {
    fn from(value: FieldError) -> Self {
        Self::Field(value)
    }
}
impl fmt::Display for DesignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field(e) => e.fmt(f),
            Self::OctaveCount => write!(f, "use 1 to {MAX_DESIGN_OCTAVES} octaves"),
            Self::WavelengthOrder => write!(f, "octave wavelengths must strictly decrease"),
            Self::VolumeWavelength => write!(
                f,
                "volume wavelength (m) must be at least radius / {}",
                VolumeConfig::COORDINATE_LIMIT
            ),
            Self::Direction => write!(f, "direction must be finite and nonzero"),
            Self::Spacing => write!(f, "sample spacing must be finite and nonnegative"),
        }
    }
}
impl Error for DesignError {}

impl PlanetDesignConfig {
    pub const RADIUS_RANGE: std::ops::RangeInclusive<f32> = 100_000.0..=8_000_000.0;
    pub const HEIGHT_LIMIT_RANGE: std::ops::RangeInclusive<f32> = 1.0..=20_000.0;

    /// Provisional editing preset, not a reconstruction of the source game's world.
    pub fn starter(seed: u64) -> Self {
        Self {
            seed,
            radius_m: 2_000_000.0,
            height_limit_m: 12_000.0,
            octaves: (0..18)
                .map(|i| OctaveConfig {
                    enabled: true,
                    wavelength_m: 1_048_576.0 / (1_u32 << i) as f32,
                    amplitude_m: 4_000.0 * 0.55_f32.powi(i),
                    sharpness: if i < 2 { 0.0 } else { -0.5 },
                    perturbation: 0.15,
                    slope_erosion: 0.15,
                    altitude_erosion: 0.0,
                    ridge_erosion: 0.1,
                })
                .collect(),
            volume: VolumeConfig::default(),
        }
    }

    pub fn validate(&self) -> Result<PlanetDesignField, DesignError> {
        validate_range("radius (m)", self.radius_m, Self::RADIUS_RANGE)?;
        validate_range(
            "height limit (m)",
            self.height_limit_m,
            Self::HEIGHT_LIMIT_RANGE,
        )?;
        if self.octaves.is_empty() || self.octaves.len() > MAX_DESIGN_OCTAVES {
            return Err(DesignError::OctaveCount);
        }
        for o in &self.octaves {
            validate_range(
                "wavelength (m)",
                o.wavelength_m,
                OctaveConfig::WAVELENGTH_RANGE,
            )?;
            validate_range(
                "amplitude (m)",
                o.amplitude_m,
                OctaveConfig::AMPLITUDE_RANGE,
            )?;
            validate_range("sharpness", o.sharpness, NoiseConfig::SHARPNESS_RANGE)?;
            for (name, value) in [
                ("warp", o.perturbation),
                ("slope damping", o.slope_erosion),
                ("altitude damping", o.altitude_erosion),
                ("ridge damping", o.ridge_erosion),
            ] {
                validate_range(name, value, NoiseConfig::SHAPING_RANGE)?;
            }
        }
        if self
            .octaves
            .windows(2)
            .any(|p| p[0].wavelength_m <= p[1].wavelength_m)
        {
            return Err(DesignError::WavelengthOrder);
        }
        validate_range(
            "volume wavelength (m)",
            self.volume.wavelength_m,
            VolumeConfig::WAVELENGTH_RANGE,
        )?;
        validate_range(
            "volume amplitude (m)",
            self.volume.amplitude_m,
            VolumeConfig::AMPLITUDE_RANGE,
        )?;
        validate_range(
            "volume sharpness",
            self.volume.sharpness,
            NoiseConfig::SHARPNESS_RANGE,
        )?;
        validate_range(
            "volume fade (m)",
            self.volume.fade_m,
            VolumeConfig::FADE_RANGE,
        )?;
        // Keep the noise argument inside the f32 range the term is resolved in.
        // A wavelength finer than radius / 2^18 would push `p_m / wavelength_m`
        // past 2^18 at the surface, where f32 can no longer place a sample
        // inside its noise cell to a sixteenth of the cell width. This one rule
        // is conditional, unlike the ranges above: an inert default wavelength
        // must not invalidate a large-radius design that never evaluates it.
        if self.volume.enabled
            && self.volume.wavelength_m < self.radius_m / VolumeConfig::COORDINATE_LIMIT
        {
            return Err(DesignError::VolumeWavelength);
        }
        let seed = fold_seed_u64_to_u32(self.seed);
        Ok(PlanetDesignField {
            config: self.clone(),
            key: hash_u32(seed, 0x5355_5246, 0, 0),
            volume_key: hash_u32(seed, 0x564F_4C55, 0, 0),
        })
    }
}

#[derive(Clone, Debug)]
pub struct PlanetDesignField {
    config: PlanetDesignConfig,
    key: u32,
    /// Separate from the height key, so the detail term is not a rescaled copy
    /// of an octave and enabling it cannot move the height surface.
    volume_key: u32,
}
impl PlanetDesignField {
    pub(crate) fn noise_key(&self) -> u32 {
        self.key
    }
    pub(crate) fn volume_key(&self) -> u32 {
        self.volume_key
    }

    pub fn config(&self) -> &PlanetDesignConfig {
        &self.config
    }

    /// Zero spacing evaluates every enabled octave. Nonzero spacing smoothly
    /// removes wavelengths from four down to two samples per wavelength.
    pub fn elevation_m(&self, direction: Vec3, spacing_m: f32) -> Result<f32, DesignError> {
        Ok(self.height(checked_direction(direction, spacing_m)?, spacing_m))
    }

    /// The surface the near-field band actually extracts, projected back into a
    /// single radial height so the far field can draw it.
    ///
    /// On the band's own surface `d` is zero, so the volume term's fade is one
    /// and the displaced height is `h + w * volume_shape(dir * (radius + h))`.
    /// `w` is the same footprint smoothstep the height octaves use, so a tile
    /// that cannot resolve the term's wavelength does not carry it and coarse
    /// tiles stitch through their shared footprints unchanged. Filtering is
    /// correct here and forbidden in the density path, where coincident samples
    /// at different levels must stay bit-identical.
    ///
    /// This is first order only. The true zero crossing along the radial ray
    /// solves `h - a + volume(P(a), h - a) = 0`; evaluating at `a = h` ignores
    /// the term's own variation over that displacement. The residual is
    /// measured, not argued: see `the_projected_far_field_surface_tracks_the_band_crossing`
    /// in voxel_density.rs.
    pub fn surface_m(&self, direction: Vec3, spacing_m: f32) -> Result<f32, DesignError> {
        let direction = checked_direction(direction, spacing_m)?;
        Ok(self.surface_height(direction, spacing_m))
    }

    pub fn height_distribution(&self) -> HeightDistribution {
        sample_heights(|direction| self.height(direction, 0.0))
    }

    /// See `surface_m` for the projection and its first-order caveat.
    pub(crate) fn surface_height(&self, direction: Vec3, footprint_m: f32) -> f32 {
        let height = self.height(direction, footprint_m);
        let volume = &self.config.volume;
        let weight = octave_weight(volume.wavelength_m, footprint_m);
        // Both early outs return `height` itself rather than adding a zero, so
        // a disabled or unresolvable term leaves the far field bit-identical.
        if volume.active_amplitude_m() == 0.0 || weight == 0.0 {
            return height;
        }
        let p = direction * (self.config.radius_m + height);
        height + weight * crate::voxel_density::volume_shape_at(self, p)
    }

    pub(crate) fn height(&self, direction: Vec3, spacing_m: f32) -> f32 {
        let mut sum = 0.0;
        let mut slope = Vec3::ZERO;
        let mut ridge = Vec3::ZERO;
        let mut warp = Vec3::ZERO;
        for o in &self.config.octaves {
            if !o.enabled || o.amplitude_m == 0.0 {
                continue;
            }
            let weight = octave_weight(o.wavelength_m, spacing_m);
            // Filter feedback as well as height. Unresolved octaves cannot alter
            // later bands or re-enter through domain warp.
            if weight == 0.0 {
                continue;
            }
            let p = direction * (self.config.radius_m / o.wavelength_m) + warp;
            let sample = normalized_noise(self.key, p);
            let d = sample.derivative * weight;
            slope = slope + d * o.slope_erosion;
            ridge = ridge + d * o.ridge_erosion;
            let altitude = (sum / self.config.height_limit_m).clamp(0.0, 1.0);
            let smooth = altitude * altitude * (3.0 - 2.0 * altitude);
            let damping = (1.0 + (smooth - 1.0) * o.altitude_erosion)
                * (1.0 - o.ridge_erosion / (1.0 + ridge.length_squared()))
                / (1.0 + slope.length_squared());
            sum += weight * o.amplitude_m * shape(sample, o.sharpness).value * damping;
            warp = warp + d * o.perturbation;
        }
        let relative = sum / self.config.height_limit_m;
        sum / (1.0 + relative * relative).sqrt()
    }
}

fn checked_direction(direction: Vec3, spacing_m: f32) -> Result<Vec3, DesignError> {
    if !direction.is_finite()
        || !direction.length_squared().is_finite()
        || direction.length_squared() == 0.0
    {
        return Err(DesignError::Direction);
    }
    if !spacing_m.is_finite() || spacing_m < 0.0 {
        return Err(DesignError::Spacing);
    }
    Ok(direction.normalized())
}

pub(crate) fn octave_weight(wavelength_m: f32, spacing_m: f32) -> f32 {
    if spacing_m == 0.0 {
        return 1.0;
    }
    let t = ((wavelength_m / spacing_m - 2.0) * 0.5).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::positions;

    #[test]
    fn physical_scale_and_serialization_preserve_the_field() {
        let config = PlanetDesignConfig::starter(4_294_967_338);
        let encoded = serde_json::to_string(&config).unwrap();
        let restored: PlanetDesignConfig = serde_json::from_str(&encoded).unwrap();
        assert_eq!(config, restored);
        let field = config.validate().unwrap();
        let mut doubled = restored;
        doubled.radius_m *= 2.0;
        doubled.height_limit_m *= 1.5;
        for o in &mut doubled.octaves {
            o.wavelength_m *= 2.0;
            o.amplitude_m *= 1.5;
        }
        let larger = doubled.validate().unwrap();
        for p in positions().take(100) {
            let h = field.elevation_m(p, 100.0).unwrap();
            // Centimeter margin for f32 summation at kilometer elevation scales.
            assert!((larger.elevation_m(p, 200.0).unwrap() - h * 1.5).abs() < 0.01);
        }
    }

    #[test]
    fn filtered_and_disabled_bands_leave_no_feedback() {
        let mut config = PlanetDesignConfig::starter(42);
        let field = config.validate().unwrap();
        config.octaves[1..]
            .iter_mut()
            .for_each(|o| o.enabled = false);
        let coarse = config.validate().unwrap();
        // First band has four samples per wavelength, second has two.
        let spacing = config.octaves[0].wavelength_m / 4.0;
        for p in positions().take(100) {
            assert_eq!(
                field.elevation_m(p, spacing).unwrap(),
                coarse.elevation_m(p, 0.0).unwrap()
            );
        }
        config.octaves[0].enabled = false;
        assert_eq!(
            config
                .validate()
                .unwrap()
                .elevation_m(Vec3::X, 0.0)
                .unwrap(),
            0.0
        );
    }

    #[test]
    fn validation_rejects_bad_order_lengths_and_nonfinite_queries() {
        let mut config = PlanetDesignConfig::starter(42);
        config.octaves.swap(0, 1);
        assert!(matches!(
            config.validate(),
            Err(DesignError::WavelengthOrder)
        ));
        config.octaves.swap(0, 1);
        config.radius_m = f32::NAN;
        assert!(config.validate().is_err());
        let field = PlanetDesignConfig::starter(42).validate().unwrap();
        assert!(field.elevation_m(Vec3::ZERO, 0.0).is_err());
        assert!(field.elevation_m(Vec3::X, -1.0).is_err());
    }

    #[test]
    fn per_band_controls_change_the_field_and_extremes_remain_bounded() {
        let config = PlanetDesignConfig::starter(42);
        let base = config.validate().unwrap();
        let mut variants = Vec::new();
        for control in 0..6 {
            let mut variant = config.clone();
            let band = &mut variant.octaves[2];
            match control {
                0 => band.amplitude_m *= 2.0,
                1 => band.sharpness = 1.0,
                2 => band.perturbation = 1.0,
                3 => band.slope_erosion = 1.0,
                4 => band.altitude_erosion = 1.0,
                5 => band.ridge_erosion = 1.0,
                _ => unreachable!(),
            }
            variants.push(variant.validate().unwrap());
        }
        for variant in variants {
            let difference: f32 = positions()
                .take(100)
                .map(|p| {
                    (base.elevation_m(p, 0.0).unwrap() - variant.elevation_m(p, 0.0).unwrap()).abs()
                })
                .sum();
            assert!(difference > 1.0);
        }
        let mut extreme = config;
        extreme.radius_m = *PlanetDesignConfig::RADIUS_RANGE.end();
        for band in &mut extreme.octaves {
            band.amplitude_m = *OctaveConfig::AMPLITUDE_RANGE.end();
            band.perturbation = 1.0;
            band.slope_erosion = 1.0;
            band.altitude_erosion = 1.0;
            band.ridge_erosion = 1.0;
            band.sharpness = 1.0;
        }
        let field = extreme.validate().unwrap();
        for p in positions().take(100) {
            let height = field.elevation_m(p, 0.0).unwrap();
            assert!(height.is_finite() && height.abs() < extreme.height_limit_m);
        }
    }

    #[test]
    fn a_design_without_a_volume_block_loads_disabled_and_round_trips_with_one() {
        let config = PlanetDesignConfig::starter(42);
        assert_eq!(config.volume, VolumeConfig::default());
        assert!(!config.volume.enabled);
        assert_eq!(config.volume.active_amplitude_m(), 0.0);
        let mut value: serde_json::Value = serde_json::to_value(&config).unwrap();
        assert!(value.as_object_mut().unwrap().remove("volume").is_some());
        let restored: PlanetDesignConfig = serde_json::from_value(value).unwrap();
        assert_eq!(restored, config);

        let mut enabled = config.clone();
        enabled.volume = VolumeConfig {
            enabled: true,
            wavelength_m: 31.5,
            amplitude_m: 12.0,
            sharpness: -0.25,
            fade_m: 40.0,
        };
        assert_eq!(enabled.volume.active_amplitude_m(), 12.0);
        let encoded = serde_json::to_string(&enabled).unwrap();
        assert_eq!(
            serde_json::from_str::<PlanetDesignConfig>(&encoded).unwrap(),
            enabled
        );
        // The term is generation data, so it must reach the field's key too.
        let field = enabled.validate().unwrap();
        assert_eq!(field.config().volume, enabled.volume);
        assert_ne!(field.volume_key(), field.noise_key());
        // An unknown member is still refused on both structs.
        let mut value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        value["volume"]["octaves"] = 3.into();
        assert!(serde_json::from_value::<PlanetDesignConfig>(value).is_err());
    }

    #[test]
    fn every_volume_range_and_the_radius_wavelength_rule_reject_one_value() {
        let base = PlanetDesignConfig {
            volume: VolumeConfig {
                enabled: true,
                ..Default::default()
            },
            ..PlanetDesignConfig::starter(42)
        };
        base.validate().unwrap();
        for case in 0..6 {
            let mut config = base.clone();
            let v = &mut config.volume;
            let name = match case {
                0 => {
                    v.wavelength_m = *VolumeConfig::WAVELENGTH_RANGE.end() + 1.0;
                    "volume wavelength (m)"
                }
                1 => {
                    v.wavelength_m = *VolumeConfig::WAVELENGTH_RANGE.start() - 0.5;
                    "volume wavelength (m)"
                }
                2 => {
                    v.amplitude_m = *VolumeConfig::AMPLITUDE_RANGE.end() + 1.0;
                    "volume amplitude (m)"
                }
                3 => {
                    v.sharpness = 1.5;
                    "volume sharpness"
                }
                4 => {
                    v.fade_m = *VolumeConfig::FADE_RANGE.start() - 0.5;
                    "volume fade (m)"
                }
                5 => {
                    v.fade_m = f32::NAN;
                    "volume fade (m)"
                }
                _ => unreachable!(),
            };
            let error = config.validate().unwrap_err().to_string();
            assert!(error.contains(name), "case {case}: {error}");
        }
        // A wavelength inside its own range can still be too fine for the
        // planet: 100 km / 2^18 is 0.38 m, 8,000 km / 2^18 is 30.5 m.
        let mut config = base.clone();
        config.radius_m = 8_000_000.0;
        assert_eq!(config.volume.wavelength_m, 24.0);
        // Disabled, the same wavelength is inert and must not invalidate it.
        config.volume.enabled = false;
        config.validate().unwrap();
        config.volume.enabled = true;
        assert!(matches!(
            config.validate(),
            Err(DesignError::VolumeWavelength)
        ));
        config.volume.wavelength_m = config.radius_m / VolumeConfig::COORDINATE_LIMIT;
        config.validate().unwrap();
        config.radius_m = *PlanetDesignConfig::RADIUS_RANGE.start();
        config.volume.wavelength_m = *VolumeConfig::WAVELENGTH_RANGE.start();
        config.validate().unwrap();
    }

    #[test]
    fn the_far_field_projection_adds_only_what_a_tile_can_resolve() {
        let disabled = PlanetDesignConfig::starter(42);
        let mut config = disabled.clone();
        config.volume.enabled = true;
        let volume = config.volume.clone();
        let with = config.validate().unwrap();
        let without = disabled.validate().unwrap();
        // Two samples per wavelength is where octave_weight reaches zero, so a
        // tile this coarse must carry exactly the bare height.
        let coarse = volume.wavelength_m / 2.0;
        let mut sampled = 0;
        let mut moved = 0;
        for p in positions() {
            sampled += 1;
            let p = p.normalized();
            assert_eq!(
                without.surface_height(p, 0.0),
                without.height(p, 0.0),
                "a disabled term must leave the far field untouched"
            );
            assert_eq!(without.surface_height(p, 0.25), without.height(p, 0.25));
            assert_eq!(
                with.surface_height(p, coarse),
                with.height(p, coarse),
                "a tile that cannot resolve the wavelength must not carry it"
            );
            let offset = with.surface_height(p, 0.0) - with.height(p, 0.0);
            assert!(
                offset.abs() <= volume.amplitude_m,
                "{offset} m exceeds the term's own bound"
            );
            moved += usize::from(offset != 0.0);
        }
        assert_eq!(
            moved, sampled,
            "the enabled term must move every unfiltered tile"
        );
        // The checked query agrees with the internal one and rejects the same
        // arguments elevation_m does.
        let p = Vec3::new(0.3, -1.0, 0.45);
        assert_eq!(
            with.surface_m(p, 0.0).unwrap(),
            with.surface_height(p.normalized(), 0.0)
        );
        assert!(with.surface_m(Vec3::ZERO, 0.0).is_err());
        assert!(with.surface_m(Vec3::X, -1.0).is_err());
    }

    #[test]
    fn bounded_seeded_and_schedule_independent() {
        let config = PlanetDesignConfig::starter(42);
        let a = config.validate().unwrap();
        let b = PlanetDesignConfig {
            seed: 4_294_967_338,
            ..config.clone()
        }
        .validate()
        .unwrap();
        let mut difference = 0.0;
        for p in positions().take(100) {
            let h = a.elevation_m(p, 0.0).unwrap();
            assert!(h.is_finite() && h.abs() < config.height_limit_m);
            difference += (h - b.elevation_m(p, 0.0).unwrap()).abs();
        }
        assert!(difference > 100.0);
        let run = |n| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .unwrap()
                .install(|| a.height_distribution())
        };
        assert_eq!(run(1), run(4));
        // The far-field projection has to be schedule-independent too.
        let projected = PlanetDesignConfig {
            volume: VolumeConfig {
                enabled: true,
                ..VolumeConfig::default()
            },
            ..config
        }
        .validate()
        .unwrap();
        let run = |n| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .unwrap()
                .install(|| {
                    positions()
                        .map(|p| projected.surface_height(p.normalized(), 0.0).to_bits())
                        .collect::<Vec<_>>()
                })
        };
        assert_eq!(run(1), run(4));
    }
}
