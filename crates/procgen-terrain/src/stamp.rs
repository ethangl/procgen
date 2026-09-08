//! Compact-support terrain-stamp policy and evaluation.

use procgen_core::Vec3;
use procgen_noise::NoiseSample3;

use crate::{TerrainStampInput, TerrainStampKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StampCap {
    Quadratic,
    Cubic,
}

impl StampCap {
    fn apply(self, support: NoiseSample3) -> NoiseSample3 {
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
}

pub(crate) fn stamp_contribution(
    direction: Vec3,
    stamp: TerrainStampInput,
    profile: TerrainStampProfile,
) -> NoiseSample3 {
    let displacement = direction - stamp.position;
    let radius_squared = profile.radius * profile.radius;
    let normalized_squared = displacement.length_squared() / radius_squared;
    if normalized_squared >= 1.0 {
        return NoiseSample3::default();
    }
    let distance = NoiseSample3 {
        value: normalized_squared,
        derivative: displacement * (2.0 / radius_squared),
    };
    profile.cap.apply(NoiseSample3::constant(1.0) - distance) * (profile.amplitude * stamp.strength)
}
