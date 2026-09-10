use super::super::{drag_value, section, slider};
use crate::model::{GeologySettings, WORLD_RADIUS};
use bevy_egui::egui;
use procgen_geology::{
    CratonFieldConfig, GeologicalElevationConfig, HotspotFieldConfig, IsostaticAdjustmentConfig,
    OceanicPeakFieldConfig, SedimentaryBasinFieldConfig, VolcanicArcFieldConfig,
};
use procgen_tectonics::{PlateKinematicsConfig, SEA_LEVEL};

const HOTSPOT_COUNT_RANGE: std::ops::RangeInclusive<usize> = 0..=256;
const HOTSPOT_TRAIL_RANGE: std::ops::RangeInclusive<usize> = 1..=64;
const HOTSPOT_PROVINCE_FRACTION_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.0;
const HOTSPOT_PROVINCE_RADIUS_RANGE: std::ops::RangeInclusive<usize> = 0..=32;
const OCEANIC_PEAK_AGE_RANGE: std::ops::RangeInclusive<usize> = 1..=64;
const ARC_SEGMENT_EDGE_RANGE: std::ops::RangeInclusive<usize> = 1..=64;
const ARC_INLAND_OFFSET_RANGE: std::ops::RangeInclusive<usize> = 1..=32;
const ARC_PEAK_DENSITY_DIVISOR_RANGE: std::ops::RangeInclusive<usize> = 1..=32;
const BOUNDARY_DISTANCE_RANGE: std::ops::RangeInclusive<usize> = 0..=64;
const BASIN_CELL_COUNT_RANGE: std::ops::RangeInclusive<usize> = 1..=256;

/// Plate kinematics come from the tectonics phase, which bounds the strengths
/// arc segments can reach.
pub(super) fn controls(
    ui: &mut egui::Ui,
    settings: &mut GeologySettings,
    kinematics: PlateKinematicsConfig,
) {
    section(ui, "Mantle hotspots", |ui| {
        hotspot_controls(ui, &mut settings.hotspots)
    });
    section(ui, "Seamounts and abyssal hills", |ui| {
        oceanic_peak_controls(ui, &mut settings.oceanic_peaks)
    });
    section(ui, "Volcanic arcs", |ui| {
        volcanic_arc_controls(ui, &mut settings.volcanic_arcs, kinematics)
    });
    section(ui, "Cratons", |ui| {
        craton_controls(ui, &mut settings.cratons)
    });
    section(ui, "Sedimentary basins", |ui| {
        basin_controls(ui, &mut settings.basins)
    });
    section(ui, "Geological elevation", |ui| {
        geological_elevation_controls(ui, &mut settings.geological_elevation)
    });
    section(ui, "Isostatic adjustment", |ui| {
        isostatic_controls(ui, &mut settings.isostasy)
    });
}

fn hotspot_controls(ui: &mut egui::Ui, config: &mut HotspotFieldConfig) {
    drag_value(
        ui,
        "Hotspots",
        &mut config.hotspot_count,
        HOTSPOT_COUNT_RANGE,
        1.0,
    );
    drag_value(
        ui,
        "Maximum trail cells",
        &mut config.maximum_trail_cells,
        HOTSPOT_TRAIL_RANGE,
        1.0,
    );
    slider(
        ui,
        "Province fraction",
        &mut config.province_fraction,
        HOTSPOT_PROVINCE_FRACTION_RANGE,
    );
    drag_value(
        ui,
        "Province radius hops",
        &mut config.province_radius_hops,
        HOTSPOT_PROVINCE_RADIUS_RANGE,
        1.0,
    );
    // The rim slopes back down inside the radius, so it cannot outrun it.
    drag_value(
        ui,
        "Province rim hops",
        &mut config.province_rim_hops,
        0..=config.province_radius_hops,
        1.0,
    );
    drag_value(
        ui,
        "Hotspot seed",
        &mut config.seed,
        u64::MIN..=u64::MAX,
        1.0,
    );
}

fn oceanic_peak_controls(ui: &mut egui::Ui, config: &mut OceanicPeakFieldConfig) {
    drag_value(
        ui,
        "Maximum young age",
        &mut config.maximum_young_age,
        OCEANIC_PEAK_AGE_RANGE,
        1.0,
    );
    slider(
        ui,
        "Seamount density",
        &mut config.seamount_density_scale,
        0.0..=1.0,
    );
    slider(
        ui,
        "Abyssal-hill density",
        &mut config.abyssal_hill_density_scale,
        0.0..=1.0,
    );
    slider(
        ui,
        "Position offset",
        &mut config.maximum_position_offset,
        0.0..=1.0,
    );
    slider(
        ui,
        "Seamount height",
        &mut config.maximum_seamount_height,
        0.0..=2.0,
    );
    slider(
        ui,
        "Abyssal-hill height",
        &mut config.maximum_abyssal_hill_height,
        0.0..=2.0,
    );
    drag_value(ui, "Peak seed", &mut config.seed, u64::MIN..=u64::MAX, 1.0);
}

fn volcanic_arc_controls(
    ui: &mut egui::Ui,
    config: &mut VolcanicArcFieldConfig,
    kinematics: PlateKinematicsConfig,
) {
    drag_value(
        ui,
        "Minimum boundary edges",
        &mut config.minimum_boundary_edges,
        ARC_SEGMENT_EDGE_RANGE,
        1.0,
    );
    drag_value(
        ui,
        "Inland offset",
        &mut config.inland_offset_cells,
        ARC_INLAND_OFFSET_RANGE,
        1.0,
    );
    drag_value(
        ui,
        "Peak density divisor",
        &mut config.peak_density_divisor,
        ARC_PEAK_DENSITY_DIVISOR_RANGE,
        1.0,
    );
    slider(
        ui,
        "Strength saturation",
        &mut config.strength_saturation,
        0.01..=kinematics.maximum_convergence(WORLD_RADIUS).max(0.01),
    );
}

fn craton_controls(ui: &mut egui::Ui, config: &mut CratonFieldConfig) {
    drag_value(
        ui,
        "Minimum boundary distance",
        &mut config.minimum_boundary_distance,
        BOUNDARY_DISTANCE_RANGE,
        1.0,
    );
    drag_value(
        ui,
        "Ramp width",
        &mut config.ramp_width,
        BOUNDARY_DISTANCE_RANGE,
        1.0,
    );
}

fn basin_controls(ui: &mut egui::Ui, config: &mut SedimentaryBasinFieldConfig) {
    slider(
        ui,
        "Maximum elevation",
        &mut config.maximum_elevation,
        SEA_LEVEL..=1.0,
    );
    drag_value(
        ui,
        "Minimum cells",
        &mut config.minimum_cell_count,
        BASIN_CELL_COUNT_RANGE,
        1.0,
    );
    slider(
        ui,
        "Maximum ocean perimeter",
        &mut config.maximum_ocean_perimeter_fraction,
        0.0..=1.0,
    );
}

fn geological_elevation_controls(ui: &mut egui::Ui, config: &mut GeologicalElevationConfig) {
    slider(ui, "Hotspot uplift", &mut config.hotspot_uplift, 0.0..=1.0);
    slider(ui, "Plateau uplift", &mut config.plateau_uplift, 0.0..=1.0);
    slider(
        ui,
        "Volcanic-arc uplift",
        &mut config.volcanic_arc_uplift,
        0.0..=1.0,
    );
    slider(
        ui,
        "Craton flattening",
        &mut config.craton_flattening,
        0.0..=1.0,
    );
    slider(
        ui,
        "Basin flattening",
        &mut config.basin_flattening,
        0.0..=1.0,
    );
}

fn isostatic_controls(ui: &mut egui::Ui, config: &mut IsostaticAdjustmentConfig) {
    slider(
        ui,
        "Adjustment strength",
        &mut config.adjustment_strength,
        0.0..=1.0,
    );
    slider(
        ui,
        "Continental support",
        &mut config.continental_support,
        0.0..=1.0,
    );
    slider(
        ui,
        "Convergent bonus",
        &mut config.convergent_support_bonus,
        0.0..=1.0,
    );
    slider(
        ui,
        "Divergent penalty",
        &mut config.divergent_support_penalty,
        0.0..=1.0,
    );
    slider(
        ui,
        "Craton bonus",
        &mut config.craton_support_bonus,
        0.0..=1.0,
    );
    drag_value(
        ui,
        "Boundary distance",
        &mut config.maximum_boundary_distance,
        BOUNDARY_DISTANCE_RANGE,
        1.0,
    );
}
