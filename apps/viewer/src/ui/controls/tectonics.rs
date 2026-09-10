use super::super::{drag_value, section, slider};
use crate::model::{TectonicsSettings, WORLD_RADIUS};
use bevy_egui::egui;
use procgen_sphere::FibonacciConfig;
use procgen_tectonics::{
    BaseElevationConfig, BoundaryDeformationConfig, BoundaryEffect, CoarseElevationConfig,
    ContinentalRiftProfile, CrustBirthPriorConfig, CrustClassificationConfig,
    DEFAULT_STEP_DURATION, MAX_GROWTH_ROUGHNESS, PlateEvolutionConfig, PlateKinematicsConfig,
    PlatePartitionConfig,
};

// The mesh has no ceiling of its own; this bounds the CPU pipeline's run time.
const CELL_COUNT_RANGE: std::ops::RangeInclusive<usize> = 4..=262_144;
const ARC_COUNT_RANGE: std::ops::RangeInclusive<usize> = 1..=256;
const CURVATURE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=8.0;
// A minor plate cannot be smaller than a cell nor larger than the face it splits.
const PIECE_FRACTION_RANGE: std::ops::RangeInclusive<f32> = 0.0005..=0.1;
const ANGULAR_SPEED_RANGE: std::ops::RangeInclusive<f32> = 0.0..=10.0;
const ANGULAR_SPEED_STEP: f64 = 0.01;
// A flow cell spans the sphere at low frequency and a single plate near the top
// of this range, past which the fit averages the field away.
const FLOW_FREQUENCY_RANGE: std::ops::RangeInclusive<f32> = 0.1..=8.0;
// Crust factors multiply the hashed base speed before it is clamped.
const CRUST_SPEED_FACTOR_RANGE: std::ops::RangeInclusive<f32> = 0.1..=4.0;
const EVOLUTION_STEP_RANGE: std::ops::RangeInclusive<usize> = 0..=256;
// Zero freezes the world; the top of the range moves the fastest plates about
// ten cells per step on the default mesh, past which a step skips terrain it
// should have crossed.
const STEP_DURATION_RANGE: std::ops::RangeInclusive<f32> = 0.0..=DEFAULT_STEP_DURATION * 10.0;
const CRUST_BIRTH_PRIOR_RANGE: std::ops::RangeInclusive<usize> = 0..=256;
const DEFORMATION_DEPTH_RANGE: std::ops::RangeInclusive<usize> = 0..=32;
// A boundary reaches its full profile in one default step at the bottom and
// over the longest run the step range allows at the top.
const FULL_DEFORMATION_TIME_RANGE: std::ops::RangeInclusive<f32> =
    DEFAULT_STEP_DURATION..=DEFAULT_STEP_DURATION * *EVOLUTION_STEP_RANGE.end() as f32;
// A single profile offset is bounded to one, and tectonic elevation clamps to
// the unit range, so a clamp above one could never bite.
const DEFORMATION_MAGNITUDE_RANGE: std::ops::RangeInclusive<f32> = 0.01..=1.0;
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
    section(ui, "Seafloor age", |ui| {
        birth_prior_controls(ui, &mut settings.birth_prior)
    });
    section(ui, "Plate evolution", |ui| {
        evolution_controls(ui, &mut settings.evolution, settings.kinematics)
    });
    section(ui, "Boundary deformation", |ui| {
        deformation_controls(ui, &mut settings.evolution.deformation, settings.kinematics)
    });
    section(ui, "Base elevation", |ui| {
        base_elevation_controls(ui, &mut settings.base_elevation)
    });
    section(ui, "Tectonic elevation", |ui| {
        elevation_controls(ui, &mut settings.elevation)
    });
}

fn sampling_controls(ui: &mut egui::Ui, config: &mut FibonacciConfig) {
    drag_value(ui, "Cells", &mut config.count, CELL_COUNT_RANGE, 16.0);
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
    drag_value(
        ui,
        "Crack arcs",
        &mut config.arc_count,
        ARC_COUNT_RANGE,
        1.0,
    );
    slider(ui, "Curvature", &mut config.curvature, CURVATURE_RANGE);
    slider(
        ui,
        "Subdivided faces",
        &mut config.subdivided_fraction,
        0.0..=1.0,
    );
    slider(
        ui,
        "Minor plate area",
        &mut config.piece_fraction,
        PIECE_FRACTION_RANGE,
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
    slider(ui, "Coherence", &mut config.coherence, 0.0..=1.0);
    slider(
        ui,
        "Flow frequency",
        &mut config.flow_frequency,
        FLOW_FREQUENCY_RANGE,
    );
    slider(
        ui,
        "Oceanic speed",
        &mut config.oceanic_speed_factor,
        CRUST_SPEED_FACTOR_RANGE,
    );
    slider(
        ui,
        "Continental speed",
        &mut config.continental_speed_factor,
        CRUST_SPEED_FACTOR_RANGE,
    );
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
        "Step duration",
        &mut config.step_duration,
        STEP_DURATION_RANGE,
    );
    slider(
        ui,
        "Minimum convergence",
        &mut config.migration.minimum_convergence,
        0.0..=kinematics.maximum_convergence(WORLD_RADIUS),
    );
}

fn birth_prior_controls(ui: &mut egui::Ui, config: &mut CrustBirthPriorConfig) {
    drag_value(
        ui,
        "Ridge-less age",
        &mut config.ridge_less_age,
        CRUST_BIRTH_PRIOR_RANGE,
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
    slider(
        ui,
        "Full deformation time",
        &mut config.full_deformation_time,
        FULL_DEFORMATION_TIME_RANGE,
    );
    slider(
        ui,
        "Maximum magnitude",
        &mut config.maximum_magnitude,
        DEFORMATION_MAGNITUDE_RANGE,
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
