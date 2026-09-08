//! Compact-support terrain-stamp policy and evaluation.

use std::{error::Error, fmt};

use procgen_core::{ScalarFieldSample3, Vec3};

use crate::{TerrainStampInput, TerrainStampKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StampCap {
    Quadratic,
    Cubic,
}

impl StampCap {
    fn apply(self, support: ScalarFieldSample3) -> ScalarFieldSample3 {
        let squared = support * support;
        match self {
            Self::Quadratic => squared,
            Self::Cubic => squared * support,
        }
    }
}

/// Radius and maximum normalized-height contribution for one stamp kind.
///
/// Radius is chord distance on the unit sphere and must be in `(0, 2]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainStampProfile {
    pub radius: f32,
    pub amplitude: f32,
    pub cap: StampCap,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainStampProfiles {
    pub hotspot: TerrainStampProfile,
    pub volcanic_arc: TerrainStampProfile,
    pub oceanic_seamount: TerrainStampProfile,
    pub oceanic_abyssal_hill: TerrainStampProfile,
}

impl TerrainStampProfiles {
    pub fn profile(self, kind: TerrainStampKind) -> TerrainStampProfile {
        match kind {
            TerrainStampKind::Hotspot => self.hotspot,
            TerrainStampKind::VolcanicArc => self.volcanic_arc,
            TerrainStampKind::OceanicSeamount => self.oceanic_seamount,
            TerrainStampKind::OceanicAbyssalHill => self.oceanic_abyssal_hill,
        }
    }

    pub(crate) fn validate(self) -> Result<(), TerrainStampError> {
        for kind in TerrainStampKind::ALL {
            let profile = self.profile(kind);
            let valid_radius =
                profile.radius.is_finite() && profile.radius > 0.0 && profile.radius <= 2.0;
            let valid_amplitude =
                profile.amplitude.is_finite() && (0.0..=1.0).contains(&profile.amplitude);
            if !valid_radius || !valid_amplitude {
                return Err(TerrainStampError::InvalidProfile(kind));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainStampError {
    InvalidProfile(TerrainStampKind),
}

impl fmt::Display for TerrainStampError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::InvalidProfile(kind) = self;
        write!(
            formatter,
            "terrain-height {kind:?} stamp profile is invalid"
        )
    }
}

impl Error for TerrainStampError {}

pub(crate) fn stamp_contribution(
    direction: Vec3,
    stamp: TerrainStampInput,
    profile: TerrainStampProfile,
) -> ScalarFieldSample3 {
    let displacement = direction - stamp.position;
    let radius_squared = profile.radius * profile.radius;
    let normalized_squared = displacement.length_squared() / radius_squared;
    if normalized_squared >= 1.0 {
        return ScalarFieldSample3::default();
    }
    let distance = ScalarFieldSample3 {
        value: normalized_squared,
        derivative: displacement * (2.0 / radius_squared),
    };
    profile
        .cap
        .apply(ScalarFieldSample3::constant(1.0) - distance)
        * (profile.amplitude * stamp.strength)
}
