use super::{drag_value, section, slider};
use crate::model::{GenerationSettings, RegenerateWorld, WORLD_RADIUS};
use bevy::prelude::MessageWriter;
use bevy_egui::egui;
use procgen_climate::{ANNUAL_SAMPLE_RANGE, SolarForcingConfig};
use procgen_geology::{
    CratonFieldConfig, GeologicalElevationConfig, HotspotFieldConfig, IsostaticAdjustmentConfig,
    OceanicPeakFieldConfig, SedimentaryBasinFieldConfig, VolcanicArcFieldConfig,
};
use procgen_planet::{
    ATMOSPHERIC_SPECIFIC_GAS_CONSTANT_RANGE, MAXIMUM_LAND_ELEVATION_METERS_RANGE,
    PLANET_RADIUS_METERS_RANGE, Planet, SIDEREAL_ROTATION_PERIOD_SECONDS_RANGE,
};
use procgen_sphere::FibonacciConfig;
use procgen_tectonics::{
    BaseElevationConfig, BoundaryDeformationConfig, BoundaryEffect, CoarseElevationConfig,
    ContinentalRiftProfile, CrustClassificationConfig, PlateEvolutionConfig, PlateKinematicsConfig,
    PlatePartitionConfig, SEA_LEVEL, SeafloorAgeConfig,
};

const ANGULAR_SPEED_RANGE: std::ops::RangeInclusive<f32> = 0.0..=10.0;
const ANGULAR_SPEED_STEP: f64 = 0.01;
const EVOLUTION_STEP_RANGE: std::ops::RangeInclusive<usize> = 0..=256;
const SEAFLOOR_AGE_RANGE: std::ops::RangeInclusive<usize> = 0..=256;
const DEFORMATION_DEPTH_RANGE: std::ops::RangeInclusive<usize> = 0..=32;
const SMOOTHING_PASS_RANGE: std::ops::RangeInclusive<usize> = 0..=32;
const HOTSPOT_COUNT_RANGE: std::ops::RangeInclusive<usize> = 0..=256;
const HOTSPOT_TRAIL_RANGE: std::ops::RangeInclusive<usize> = 1..=64;
const OCEANIC_PEAK_AGE_RANGE: std::ops::RangeInclusive<usize> = 1..=64;
const ARC_SEGMENT_EDGE_RANGE: std::ops::RangeInclusive<usize> = 1..=64;
const ARC_INLAND_OFFSET_RANGE: std::ops::RangeInclusive<usize> = 1..=32;
const ARC_PEAK_DENSITY_DIVISOR_RANGE: std::ops::RangeInclusive<usize> = 1..=32;
const BOUNDARY_DISTANCE_RANGE: std::ops::RangeInclusive<usize> = 0..=64;
const BASIN_CELL_COUNT_RANGE: std::ops::RangeInclusive<usize> = 1..=256;

mod climate;

pub(super) fn generation_controls(
    ui: &mut egui::Ui,
    generation: &mut GenerationSettings,
    regenerate: &mut MessageWriter<RegenerateWorld>,
) {
    section(ui, "Sampling", |ui| {
        sampling_controls(ui, &mut generation.fibonacci)
    });
    section(ui, "Tectonic plates", |ui| {
        plate_controls(ui, &mut generation.plates)
    });
    section(ui, "Static crust", |ui| {
        crust_controls(ui, &mut generation.crust)
    });
    section(ui, "Plate kinematics", |ui| {
        kinematics_controls(ui, &mut generation.kinematics)
    });
    section(ui, "Plate evolution", |ui| {
        evolution_controls(ui, &mut generation.evolution, generation.kinematics)
    });
    section(ui, "Seafloor age", |ui| {
        seafloor_age_controls(ui, &mut generation.seafloor_age)
    });
    section(ui, "Base elevation", |ui| {
        base_elevation_controls(ui, &mut generation.base_elevation)
    });
    section(ui, "Boundary deformation", |ui| {
        deformation_controls(ui, &mut generation.deformation, generation.kinematics)
    });
    section(ui, "Tectonic elevation", |ui| {
        elevation_controls(ui, &mut generation.elevation)
    });
    section(ui, "Mantle hotspots", |ui| {
        hotspot_controls(ui, &mut generation.hotspots)
    });
    section(ui, "Seamounts and abyssal hills", |ui| {
        oceanic_peak_controls(ui, &mut generation.oceanic_peaks)
    });
    section(ui, "Volcanic arcs", |ui| {
        volcanic_arc_controls(ui, &mut generation.volcanic_arcs, generation.kinematics)
    });
    section(ui, "Cratons", |ui| {
        craton_controls(ui, &mut generation.cratons)
    });
    section(ui, "Sedimentary basins", |ui| {
        basin_controls(ui, &mut generation.basins)
    });
    section(ui, "Geological elevation", |ui| {
        geological_elevation_controls(ui, &mut generation.geological_elevation)
    });
    section(ui, "Isostatic adjustment", |ui| {
        isostatic_controls(ui, &mut generation.isostasy)
    });
    section(ui, "Planet", |ui| {
        planet_controls(ui, &mut generation.planet)
    });
    section(ui, "Solar forcing", |ui| {
        solar_forcing_controls(ui, &mut generation.solar_forcing)
    });
    climate::generation_controls(ui, generation);
    if ui.button("Regenerate").clicked() {
        regenerate.write_default();
    }
}

fn planet_controls(ui: &mut egui::Ui, planet: &mut Planet) {
    drag_value(
        ui,
        "Planet radius meters",
        &mut planet.radius_meters,
        PLANET_RADIUS_METERS_RANGE,
        10_000.0,
    );
    drag_value(
        ui,
        "Rotation period seconds",
        &mut planet.sidereal_rotation_period_seconds,
        SIDEREAL_ROTATION_PERIOD_SECONDS_RANGE,
        1_000.0,
    );
    drag_value(
        ui,
        "Atmospheric gas constant",
        &mut planet.atmospheric_specific_gas_constant_joules_per_kilogram_kelvin,
        ATMOSPHERIC_SPECIFIC_GAS_CONSTANT_RANGE,
        1.0,
    );
    drag_value(
        ui,
        "Maximum land elevation meters",
        &mut planet.maximum_land_elevation_meters,
        MAXIMUM_LAND_ELEVATION_METERS_RANGE,
        100.0,
    );
}

fn solar_forcing_controls(ui: &mut egui::Ui, config: &mut SolarForcingConfig) {
    slider(ui, "Orbital phase", &mut config.orbital_phase, 0.0..=1.0);
    drag_value(
        ui,
        "Annual samples",
        &mut config.annual_sample_count,
        ANNUAL_SAMPLE_RANGE,
        1.0,
    );
}

fn seafloor_age_controls(ui: &mut egui::Ui, config: &mut SeafloorAgeConfig) {
    drag_value(
        ui,
        "Ridge-less age",
        &mut config.ridge_less_age,
        SEAFLOOR_AGE_RANGE,
        1.0,
    );
}

fn base_elevation_controls(ui: &mut egui::Ui, config: &mut BaseElevationConfig) {
    slider(
        ui,
        "Continental base",
        &mut config.continental_base,
        0.0..=1.0,
    );
    slider(
        ui,
        "Ridge elevation",
        &mut config.ridge_elevation,
        0.0..=1.0,
    );
    slider(
        ui,
        "Deep ocean",
        &mut config.deep_ocean_elevation,
        0.0..=1.0,
    );
    drag_value(ui, "Cooling age", &mut config.cooling_age, 1..=256, 1.0);
}

fn sampling_controls(ui: &mut egui::Ui, config: &mut FibonacciConfig) {
    drag_value(ui, "Cells", &mut config.count, 4..=65_536, 16.0);
    slider(ui, "Jitter", &mut config.jitter, 0.0..=1.0);
    drag_value(
        ui,
        "Sampling seed",
        &mut config.seed,
        u64::MIN..=u64::MAX,
        1.0,
    );
}

fn plate_controls(ui: &mut egui::Ui, config: &mut PlatePartitionConfig) {
    drag_value(ui, "Major", &mut config.major_plate_count, 1..=128, 1.0);
    drag_value(ui, "Minor", &mut config.minor_plate_count, 0..=256, 1.0);
    drag_value(
        ui,
        "Major head start",
        &mut config.major_head_start_rounds,
        0..=64,
        1.0,
    );
    drag_value(ui, "Plate seed", &mut config.seed, u64::MIN..=u64::MAX, 1.0);
}

fn crust_controls(ui: &mut egui::Ui, config: &mut CrustClassificationConfig) {
    slider(
        ui,
        "Target ocean",
        &mut config.target_ocean_fraction,
        0.0..=1.0,
    );
    drag_value(ui, "Crust seed", &mut config.seed, u64::MIN..=u64::MAX, 1.0);
}

fn kinematics_controls(ui: &mut egui::Ui, config: &mut PlateKinematicsConfig) {
    drag_value(
        ui,
        "Motion seed",
        &mut config.seed,
        u64::MIN..=u64::MAX,
        1.0,
    );
    ui.horizontal(|ui| {
        ui.label("Angular speed");
        ui.add(
            egui::DragValue::new(&mut config.minimum_angular_speed)
                .range(ANGULAR_SPEED_RANGE)
                .speed(ANGULAR_SPEED_STEP),
        );
        ui.label("to");
        ui.add(
            egui::DragValue::new(&mut config.maximum_angular_speed)
                .range(ANGULAR_SPEED_RANGE)
                .speed(ANGULAR_SPEED_STEP),
        );
    });
}

fn evolution_controls(
    ui: &mut egui::Ui,
    config: &mut PlateEvolutionConfig,
    kinematics: PlateKinematicsConfig,
) {
    drag_value(
        ui,
        "Steps",
        &mut config.step_count,
        EVOLUTION_STEP_RANGE,
        1.0,
    );
    slider(
        ui,
        "Minimum convergence",
        &mut config.migration.minimum_convergence,
        0.0..=kinematics.maximum_convergence(WORLD_RADIUS),
    );
}

fn deformation_controls(
    ui: &mut egui::Ui,
    config: &mut BoundaryDeformationConfig,
    kinematics: PlateKinematicsConfig,
) {
    boundary_effect_controls(ui, "Convergent", &mut config.convergent);
    continental_rift_controls(ui, &mut config.rift);
    boundary_effect_controls(ui, "Transform", &mut config.transform);
    boundary_effect_controls(ui, "Collision", &mut config.collision);
    boundary_effect_controls(ui, "Trench", &mut config.trench);
    let maximum_strength = kinematics.maximum_convergence(WORLD_RADIUS).max(0.01);
    slider(
        ui,
        "Saturation speed",
        &mut config.saturation_speed,
        0.01..=maximum_strength,
    );
}

fn continental_rift_controls(ui: &mut egui::Ui, profile: &mut ContinentalRiftProfile) {
    slider(
        ui,
        "Rift center offset",
        &mut profile.center_offset,
        -1.0..=0.0,
    );
    slider(
        ui,
        "Rift flank offset",
        &mut profile.flank_offset,
        -1.0..=0.0,
    );
    drag_value(
        ui,
        "Rift decay depth",
        &mut profile.decay_depth,
        ContinentalRiftProfile::MIN_DECAY_DEPTH..=*DEFORMATION_DEPTH_RANGE.end(),
        1.0,
    );
}

fn elevation_controls(ui: &mut egui::Ui, config: &mut CoarseElevationConfig) {
    drag_value(
        ui,
        "Smoothing passes",
        &mut config.smoothing_passes,
        SMOOTHING_PASS_RANGE,
        1.0,
    );
    slider(
        ui,
        "Smoothing weight",
        &mut config.smoothing_weight,
        0.0..=1.0,
    );
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
    drag_value(
        ui,
        "Hotspot seed",
        &mut config.seed,
        u64::MIN..=u64::MAX,
        1.0,
    );
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

fn boundary_effect_controls(ui: &mut egui::Ui, label: &str, effect: &mut BoundaryEffect) {
    slider(
        ui,
        &format!("{label} offset"),
        &mut effect.offset,
        -1.0..=1.0,
    );
    drag_value(
        ui,
        &format!("{label} depth"),
        &mut effect.depth,
        DEFORMATION_DEPTH_RANGE,
        1.0,
    );
}
