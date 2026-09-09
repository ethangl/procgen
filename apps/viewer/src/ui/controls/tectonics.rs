use super::super::{drag_value, section, slider};
use crate::model::{TectonicsSettings, WORLD_RADIUS};
use bevy_egui::egui;
use procgen_sphere::FibonacciConfig;
use procgen_tectonics::{
    BaseElevationConfig, BoundaryDeformationConfig, BoundaryEffect, CoarseElevationConfig,
    ContinentalRiftProfile, CrustClassificationConfig, MAX_GROWTH_ROUGHNESS, PlateEvolutionConfig,
    PlateKinematicsConfig, PlatePartitionConfig, SeafloorAgeConfig,
};

const ANGULAR_SPEED_RANGE: std::ops::RangeInclusive<f32> = 0.0..=10.0;
const ANGULAR_SPEED_STEP: f64 = 0.01;
const EVOLUTION_STEP_RANGE: std::ops::RangeInclusive<usize> = 0..=256;
const SEAFLOOR_AGE_RANGE: std::ops::RangeInclusive<usize> = 0..=256;
const DEFORMATION_DEPTH_RANGE: std::ops::RangeInclusive<usize> = 0..=32;
const SMOOTHING_PASS_RANGE: std::ops::RangeInclusive<usize> = 0..=32;

pub(super) fn controls(ui: &mut egui::Ui, settings: &mut TectonicsSettings) {
    section(ui, "Sampling", |ui| {
        sampling_controls(ui, &mut settings.fibonacci)
    });
    section(ui, "Tectonic plates", |ui| {
        plate_controls(ui, &mut settings.plates)
    });
    section(ui, "Static crust", |ui| {
        crust_controls(ui, &mut settings.crust)
    });
    section(ui, "Plate kinematics", |ui| {
        kinematics_controls(ui, &mut settings.kinematics)
    });
    section(ui, "Plate evolution", |ui| {
        evolution_controls(ui, &mut settings.evolution, settings.kinematics)
    });
    section(ui, "Seafloor age", |ui| {
        seafloor_age_controls(ui, &mut settings.seafloor_age)
    });
    section(ui, "Base elevation", |ui| {
        base_elevation_controls(ui, &mut settings.base_elevation)
    });
    section(ui, "Boundary deformation", |ui| {
        deformation_controls(ui, &mut settings.deformation, settings.kinematics)
    });
    section(ui, "Tectonic elevation", |ui| {
        elevation_controls(ui, &mut settings.elevation)
    });
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
    drag_value(
        ui,
        "Growth roughness %",
        &mut config.growth_roughness,
        0..=MAX_GROWTH_ROUGHNESS,
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
