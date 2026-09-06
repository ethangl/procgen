use bevy::prelude::{Color, Vec3};
use procgen_core::Vec3 as SphereVec3;
use procgen_tectonics::SEA_LEVEL;

pub(super) const DEFORMATION_COLOR_STOPS: &[(f32, Vec3)] = &[
    (-0.5, Vec3::new(0.08, 0.35, 0.95)),
    (0.0, Vec3::new(0.12, 0.12, 0.16)),
    (0.5, Vec3::new(1.0, 0.38, 0.08)),
];
pub(super) const ELEVATION_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.02, 0.08, 0.3)),
    (SEA_LEVEL, Vec3::new(0.08, 0.65, 0.85)),
    // Duplicate sea-level stop deliberately separates water from land.
    (SEA_LEVEL, Vec3::new(0.16, 0.55, 0.18)),
    (0.75, Vec3::new(0.55, 0.38, 0.16)),
    (1.0, Vec3::new(0.96, 0.96, 0.94)),
];
pub(super) const TEMPERATURE_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.015, 0.02, 0.08)),
    (180.0, Vec3::new(0.08, 0.16, 0.46)),
    (240.0, Vec3::new(0.12, 0.62, 0.86)),
    (273.15, Vec3::new(0.82, 0.95, 0.92)),
    (320.0, Vec3::new(1.0, 0.68, 0.12)),
    (400.0, Vec3::new(0.86, 0.08, 0.035)),
];
pub(super) const TEMPERATURE_AMPLITUDE_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.02, 0.035, 0.09)),
    (10.0, Vec3::new(0.08, 0.32, 0.62)),
    (30.0, Vec3::new(0.12, 0.72, 0.72)),
    (75.0, Vec3::new(1.0, 0.68, 0.1)),
    (150.0, Vec3::new(0.9, 0.08, 0.035)),
];
pub(super) const TEMPERATURE_GRADIENT_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.02, 0.035, 0.09)),
    (25.0, Vec3::new(0.08, 0.4, 0.72)),
    (75.0, Vec3::new(0.2, 0.82, 0.65)),
    (200.0, Vec3::new(1.0, 0.42, 0.08)),
];
pub(super) const PRESSURE_ACCELERATION_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.02, 0.035, 0.09)),
    (0.001, Vec3::new(0.12, 0.35, 0.8)),
    (0.004, Vec3::new(0.25, 0.82, 0.65)),
    (0.012, Vec3::new(1.0, 0.35, 0.08)),
];
pub(super) const CORIOLIS_COLOR_STOPS: &[(f32, Vec3)] = &[
    (-0.000_16, Vec3::new(0.15, 0.4, 1.0)),
    (0.0, Vec3::new(0.94, 0.94, 0.94)),
    (0.000_16, Vec3::new(1.0, 0.3, 0.15)),
];
pub(super) const FRACTION_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.03, 0.05, 0.1)),
    (0.5, Vec3::new(0.16, 0.68, 0.7)),
    (1.0, Vec3::new(1.0, 0.75, 0.15)),
];
pub(super) const ALBEDO_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.02, 0.035, 0.08)),
    (0.2, Vec3::new(0.12, 0.3, 0.55)),
    (0.6, Vec3::new(0.72, 0.82, 0.88)),
    (1.0, Vec3::new(1.0, 1.0, 1.0)),
];
pub(super) const WIND_SPEED_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.03, 0.05, 0.1)),
    (10.0, Vec3::new(0.08, 0.38, 0.72)),
    (30.0, Vec3::new(0.12, 0.75, 0.72)),
    (60.0, Vec3::new(1.0, 0.72, 0.12)),
    (100.0, Vec3::new(0.9, 0.1, 0.04)),
];
pub(super) const HUMIDITY_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.08, 0.045, 0.025)),
    (2.0, Vec3::new(0.55, 0.28, 0.08)),
    (10.0, Vec3::new(0.18, 0.58, 0.62)),
    (30.0, Vec3::new(0.12, 0.35, 0.85)),
    (75.0, Vec3::new(0.72, 0.88, 1.0)),
];
pub(super) const PRECIPITATION_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.12, 0.06, 0.025)),
    (0.25, Vec3::new(0.75, 0.38, 0.08)),
    (1.0, Vec3::new(0.28, 0.68, 0.42)),
    (4.0, Vec3::new(0.08, 0.48, 0.9)),
    (12.0, Vec3::new(0.72, 0.82, 1.0)),
];
pub(super) const SNOW_COVER_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.04, 0.055, 0.075)),
    (0.5, Vec3::new(0.58, 0.72, 0.82)),
    (1.0, Vec3::new(0.98, 0.99, 1.0)),
];
pub(super) const LAND_ICE_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.035, 0.05, 0.075)),
    (0.5, Vec3::new(0.35, 0.72, 0.9)),
    (1.0, Vec3::new(0.82, 0.96, 1.0)),
];
pub(super) const SEA_ICE_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.015, 0.04, 0.12)),
    (0.5, Vec3::new(0.25, 0.62, 0.82)),
    (1.0, Vec3::new(0.78, 0.94, 0.98)),
];
pub(super) const HOTSPOT_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.08, 0.06, 0.12)),
    (0.25, Vec3::new(0.55, 0.08, 0.3)),
    (0.65, Vec3::new(1.0, 0.25, 0.05)),
    (1.0, Vec3::new(1.0, 0.95, 0.25)),
];
pub(super) const OCEANIC_PEAK_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.02, 0.06, 0.12)),
    (0.25, Vec3::new(0.05, 0.35, 0.52)),
    (0.65, Vec3::new(0.18, 0.78, 0.72)),
    (1.0, Vec3::new(0.95, 0.9, 0.42)),
];
pub(super) const VOLCANIC_ARC_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.08, 0.055, 0.04)),
    (0.25, Vec3::new(0.55, 0.12, 0.02)),
    (0.65, Vec3::new(1.0, 0.42, 0.03)),
    (1.0, Vec3::new(1.0, 0.95, 0.28)),
];
pub(super) const CRATON_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.06, 0.08, 0.07)),
    (0.25, Vec3::new(0.18, 0.34, 0.22)),
    (0.65, Vec3::new(0.55, 0.68, 0.32)),
    (1.0, Vec3::new(0.92, 0.86, 0.5)),
];
pub(super) const SEAFLOOR_AGE_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.35, 0.95, 1.0)),
    (0.5, Vec3::new(0.08, 0.4, 0.8)),
    (1.0, Vec3::new(0.015, 0.05, 0.2)),
];
pub(super) const INSOLATION_COLOR_STOPS: &[(f32, Vec3)] = &[
    (0.0, Vec3::new(0.015, 0.02, 0.08)),
    (0.2, Vec3::new(0.08, 0.18, 0.5)),
    (0.45, Vec3::new(0.12, 0.65, 0.82)),
    (0.7, Vec3::new(1.0, 0.72, 0.12)),
    (1.0, Vec3::new(1.0, 0.98, 0.78)),
];

pub(super) fn piecewise_lerp(value: f32, stops: &[(f32, Vec3)]) -> Vec3 {
    let value = value.clamp(stops[0].0, stops[stops.len() - 1].0);
    for pair in stops.windows(2) {
        let (low_value, low) = pair[0];
        let (high_value, high) = pair[1];
        if value < high_value {
            let t = (value - low_value) / (high_value - low_value);
            return low.lerp(high, t);
        }
    }
    stops[stops.len() - 1].1
}

pub(super) fn id_color(id: usize) -> Color {
    let hue = (id as f32 * 137.508) % 360.0;
    Color::hsla(hue, 0.62, 0.62, 0.95)
}

pub(super) fn opaque_color(color: Vec3) -> Color {
    Color::srgb(color.x, color.y, color.z)
}

pub(super) fn to_bevy(point: SphereVec3) -> Vec3 {
    Vec3::new(point.x, point.y, point.z)
}
