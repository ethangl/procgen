use super::super::{drag_value, hop_range, length_slider, section, slider};
use crate::model::{TectonicsSettings, WORLD_RADIUS};
use bevy_egui::egui;
use procgen_sphere::FibonacciConfig;
use procgen_sphere_mesh::{default_hop_length, mean_cell_width};
use procgen_tectonics::{
    BaseElevationConfig, BoundaryDeformationConfig, BoundaryEffect, CoarseElevationConfig,
    ContinentalRiftProfile, CrustBirthPriorConfig, CrustClassificationConfig,
    DEFAULT_STEP_DURATION, MAX_GAP_RADIUS, MAX_GROWTH_ROUGHNESS, PlateEvolutionConfig,
    PlateKinematicsConfig, PlateLifecycleConfig, PlatePartitionConfig, PoleDriftConfig,
    maximum_step_duration,
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
// One nucleus is a single continent; the top of the range is about the number
// of plates the default partition makes, past which a nucleus is a plate.
const NUCLEUS_COUNT_RANGE: std::ops::RangeInclusive<usize> = 1..=128;
// Crust factors multiply the hashed base speed before it is clamped.
const CRUST_SPEED_FACTOR_RANGE: std::ops::RangeInclusive<f32> = 0.1..=4.0;
// Both are shares, so both stop at one, where the slab factor disappears: at a
// trenchless factor of one every plate keeps its whole base speed, and at a
// saturation of one only a plate that is nothing but trench does. The bottoms
// sit where a plate without a trench barely moves and where the least trench a
// plate can have already saturates.
const TRENCHLESS_SPEED_FACTOR_RANGE: std::ops::RangeInclusive<f32> = 0.05..=1.0;
const SLAB_SATURATION_RANGE: std::ops::RangeInclusive<f32> = 0.05..=1.0;
const EVOLUTION_STEP_RANGE: std::ops::RangeInclusive<usize> = 0..=256;
// The top is the crate's own ceiling, which is how far the search reaches. At
// the bottom every cell a rigid rotation left empty makes floor it should not
// have.
const GAP_RADIUS_RANGE: std::ops::RangeInclusive<f32> = 0.5..=MAX_GAP_RADIUS;
// Drift rates are per unit root time, so both ranges are stated as the change
// one default step may make and divided back out by the root of that step. The
// tops sit where a boundary starts changing regime within a step or two, which
// blurs the accumulated fields instead of recording them.
const MAXIMUM_AXIS_DRIFT_PER_STEP: f32 = 0.5;
const MAXIMUM_SPEED_DRIFT_PER_STEP: f32 = 0.25;
// A rift rate is stated as the chance one default step draws, and divided
// back out. At the top a large plate breaks up almost every step.
const MAXIMUM_RIFT_CHANCE_PER_STEP: f32 = 1.0;
const RIFT_RATE_RANGE: std::ops::RangeInclusive<f32> =
    0.0..=MAXIMUM_RIFT_CHANCE_PER_STEP / DEFAULT_STEP_DURATION;
// A plate below a cell's worth of the default mesh cannot rift at all, and
// nothing above a fifth of the sphere is a plate.
const RIFT_AREA_FRACTION_RANGE: std::ops::RangeInclusive<f32> = 0.0..=0.2;
// Model times the user sets here all span one default step to the longest run
// the step range allows: a suture, a boundary reaching its full profile, and
// the ocean floor cooling to the deep floor all happen inside a run.
const MODEL_TIME_RANGE: std::ops::RangeInclusive<f32> =
    DEFAULT_STEP_DURATION..=DEFAULT_STEP_DURATION * *EVOLUTION_STEP_RANGE.end() as f32;
// Zero merges any pair of touching continents; the top is a collision front
// spanning a good fraction of a default-mesh plate's perimeter.
const SUTURE_SHARED_HOPS: std::ops::RangeInclusive<f32> = 0.0..=64.0;

// Lengths, in hops of the default mesh: the units the old hop counts were
// chosen in.
const DEFORMATION_DEPTH_HOPS: std::ops::RangeInclusive<f32> = 0.0..=32.0;
// A single profile offset is bounded to one, and tectonic elevation clamps to
// the unit range, so a clamp above one could never bite.
const DEFORMATION_MAGNITUDE_RANGE: std::ops::RangeInclusive<f32> = 0.01..=1.0;
const SMOOTHING_RADIUS_HOPS: std::ops::RangeInclusive<f32> = 0.0..=32.0;
// The datum must stay strictly inside the unit range the field is clamped to.
// The bottom leaves only the deep floor at 0.08 under water and the top drowns
// the continental base at 0.65, so the range spans an almost fully exposed
// world to one where only orogenic belts are left standing.
const SEA_LEVEL_RANGE: std::ops::RangeInclusive<f32> = 0.2..=0.8;
// Interior relief is broad and gentle on purpose. At the top of this range one
// term alone spans a third of the way from the continental base to sea level,
// which is where the two stop being relief on a plate and start deciding where
// its coast is.
const INTERIOR_RELIEF_AMPLITUDE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=0.1;
// A lattice feature spans about two cells, so the bottom is one basement
// feature across most of a great circle and the top puts the third octave at
// four or five cells of the default mesh, the shortest wavelength it carries.
const BASEMENT_FREQUENCY_RANGE: std::ops::RangeInclusive<f32> = 0.5..=8.0;
// Zero is the cliff a continent's edge was before the shelf existed; the top
// is a shelf as wide as the few cells a boundary deforms, past which the
// taper, not the crust mask, would decide where a continent is.
const MARGIN_WIDTH_HOPS: std::ops::RangeInclusive<f32> = 0.0..=8.0;
// The shelf edge spans the deep floor at 0.08 up to the continental base,
// which the stage rejects being above; the bottom lets a shelf drop to the
// ocean it meets and the top collapses the taper back to a cliff.
const MARGIN_EDGE_RANGE: std::ops::RangeInclusive<f32> = 0.08..=0.65;

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
        evolution_controls(
            ui,
            &mut settings.evolution,
            settings.kinematics,
            settings.fibonacci.count,
        )
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
        "Target continent",
        &mut config.continental_fraction,
        0.0..=1.0,
    );
    drag_value(
        ui,
        "Nuclei",
        &mut config.nucleus_count,
        NUCLEUS_COUNT_RANGE,
        1.0,
    );
    drag_value(
        ui,
        "Growth roughness %",
        &mut config.growth_roughness,
        0..=MAX_GROWTH_ROUGHNESS,
        1.0,
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
        ui.label("Angular speed").on_hover_text(
            "The range each plate's hashed base speed is drawn from. Crust and \
             slab pull scale that draw, so a plate's own speed can sit well \
             below the minimum; the maximum is also the ceiling every speed is \
             clamped to.",
        );
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
    slider(
        ui,
        "Trenchless speed",
        &mut config.trenchless_speed_factor,
        TRENCHLESS_SPEED_FACTOR_RANGE,
    );
    slider(
        ui,
        "Slab saturation",
        &mut config.slab_saturation_fraction,
        SLAB_SATURATION_RANGE,
    );
}

fn evolution_controls(
    ui: &mut egui::Ui,
    config: &mut PlateEvolutionConfig,
    kinematics: PlateKinematicsConfig,
    cell_count: usize,
) {
    slider(
        ui,
        "Run duration",
        &mut config.run_duration,
        0.0..=DEFAULT_STEP_DURATION * *EVOLUTION_STEP_RANGE.end() as f32,
    );
    // The run is what the user sets; the count is what this mesh and this step
    // make of it, and it is worth seeing because it is what the phase costs.
    ui.horizontal(|ui| {
        ui.label("Steps");
        ui.label(config.step_count().to_string())
            .on_hover_text("the run duration over the step duration");
    });
    // Zero freezes the world, and the top is as far as a step may carry
    // material before it outruns what transport can see. The fastest plate the
    // kinematics config could fit bounds it, because the plates are not fitted
    // until the phase is generated.
    let longest_step = maximum_step_duration(
        kinematics.maximum_angular_speed,
        WORLD_RADIUS,
        mean_cell_width(WORLD_RADIUS, cell_count),
        config,
    );
    slider(
        ui,
        "Step duration",
        &mut config.step_duration,
        0.0..=longest_step,
    );
    slider(
        ui,
        "Gap radius",
        &mut config.transport.gap_radius,
        GAP_RADIUS_RANGE,
    );
    pole_drift_controls(ui, &mut config.pole_drift);
    lifecycle_controls(ui, &mut config.lifecycle, kinematics);
    drag_value(
        ui,
        "Evolution seed",
        &mut config.seed,
        u64::MIN..=u64::MAX,
        1.0,
    );
}

fn pole_drift_controls(ui: &mut egui::Ui, config: &mut PoleDriftConfig) {
    slider(
        ui,
        "Axis drift",
        &mut config.axis_drift_rate,
        0.0..=MAXIMUM_AXIS_DRIFT_PER_STEP / DEFAULT_STEP_DURATION.sqrt(),
    );
    slider(
        ui,
        "Speed drift",
        &mut config.speed_drift_rate,
        0.0..=MAXIMUM_SPEED_DRIFT_PER_STEP / DEFAULT_STEP_DURATION.sqrt(),
    );
    // One is the whole of a plate's fitted speed: the band then reaches zero.
    slider(
        ui,
        "Speed drift band",
        &mut config.speed_drift_limit,
        0.0..=1.0,
    );
}

fn lifecycle_controls(
    ui: &mut egui::Ui,
    config: &mut PlateLifecycleConfig,
    kinematics: PlateKinematicsConfig,
) {
    slider(ui, "Rift rate", &mut config.rift_rate, RIFT_RATE_RANGE);
    slider(
        ui,
        "Rift minimum area",
        &mut config.rift_minimum_area_fraction,
        RIFT_AREA_FRACTION_RANGE,
    );
    slider(
        ui,
        "Rift curvature",
        &mut config.rift_curvature,
        CURVATURE_RANGE,
    );
    // The halves part at this speed on top of the parent's motion, so the
    // fastest plate the kinematics allows bounds it.
    slider(
        ui,
        "Rift opening speed",
        &mut config.rift_opening_speed,
        0.0..=kinematics.maximum_angular_speed,
    );
    slider(ui, "Suture time", &mut config.suture_time, MODEL_TIME_RANGE);
    length_slider(
        ui,
        "Suture front",
        &mut config.suture_minimum_shared_length,
        hop_range(SUTURE_SHARED_HOPS),
    );
}

fn birth_prior_controls(ui: &mut egui::Ui, config: &mut CrustBirthPriorConfig) {
    slider(
        ui,
        "Ridge-less age",
        &mut config.ridge_less_age,
        // Model time, like every other age. One hop of the default mesh at the
        // unit speed is what a hop of the prior's walk stands for, so the
        // range is the hop counts this held before it was a time.
        0.0..=256.0 * default_hop_length(),
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
    slider(ui, "Cooling age", &mut config.cooling_age, MODEL_TIME_RANGE);
    slider(
        ui,
        "Dynamic topography",
        &mut config.dynamic_topography_amplitude,
        INTERIOR_RELIEF_AMPLITUDE_RANGE,
    );
    slider(
        ui,
        "Basement relief",
        &mut config.basement_amplitude,
        INTERIOR_RELIEF_AMPLITUDE_RANGE,
    );
    slider(
        ui,
        "Basement frequency",
        &mut config.basement_frequency,
        BASEMENT_FREQUENCY_RANGE,
    );
    drag_value(
        ui,
        "Basement seed",
        &mut config.seed,
        u64::MIN..=u64::MAX,
        1.0,
    );
    length_slider(
        ui,
        "Margin width",
        &mut config.margin_width,
        hop_range(MARGIN_WIDTH_HOPS),
    );
    slider(
        ui,
        "Margin edge",
        &mut config.margin_edge_elevation,
        MARGIN_EDGE_RANGE,
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
    boundary_effect_controls(ui, "Island arc", &mut config.island_arc);
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
        MODEL_TIME_RANGE,
    );
    slider(
        ui,
        "Maximum magnitude",
        &mut config.maximum_magnitude,
        DEFORMATION_MAGNITUDE_RANGE,
    );
    slider(ui, "Erosion time", &mut config.erosion_time, MODEL_TIME_RANGE);
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
    length_slider(
        ui,
        "Rift decay depth",
        &mut profile.decay_depth,
        ContinentalRiftProfile::minimum_decay_depth()..=*hop_range(DEFORMATION_DEPTH_HOPS).end(),
    );
}

fn elevation_controls(ui: &mut egui::Ui, config: &mut CoarseElevationConfig) {
    slider(ui, "Sea level", &mut config.sea_level, SEA_LEVEL_RANGE);
    length_slider(
        ui,
        "Smoothing radius",
        &mut config.smoothing_radius,
        hop_range(SMOOTHING_RADIUS_HOPS),
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
    length_slider(
        ui,
        &format!("{label} depth"),
        &mut effect.depth,
        hop_range(DEFORMATION_DEPTH_HOPS),
    );
}
