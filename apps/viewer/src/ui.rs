use crate::model::{
    GeneratedWorld, GenerationSettings, GenerationStatus, RegenerateWorld, WORLD_RADIUS,
};
use crate::render::{DiagnosticLayer, LayerSettings};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use procgen_geology::HotspotFieldConfig;
use procgen_sphere::FibonacciConfig;
use procgen_tectonics::{
    BaseElevationConfig, BoundaryClass, BoundaryDeformationConfig, BoundaryEffect,
    CoarseElevationConfig, ContinentalRiftProfile, CrustClass, CrustClassificationConfig,
    FieldSummary, PlateEvolutionConfig, PlateKinematicsConfig, PlatePartitionConfig,
    SeafloorAgeConfig,
};

const ANGULAR_SPEED_RANGE: std::ops::RangeInclusive<f32> = 0.0..=10.0;
const ANGULAR_SPEED_STEP: f64 = 0.01;
const EVOLUTION_STEP_RANGE: std::ops::RangeInclusive<usize> = 0..=256;
const SEAFLOOR_AGE_RANGE: std::ops::RangeInclusive<usize> = 0..=256;
const DEFORMATION_DEPTH_RANGE: std::ops::RangeInclusive<usize> = 0..=32;
const SMOOTHING_PASS_RANGE: std::ops::RangeInclusive<usize> = 0..=32;
const HOTSPOT_COUNT_RANGE: std::ops::RangeInclusive<usize> = 0..=256;
const HOTSPOT_TRAIL_RANGE: std::ops::RangeInclusive<usize> = 1..=64;
const SECTION_SPACING: f32 = 6.0;

pub struct ViewerUiPlugin;

impl Plugin for ViewerUiPlugin {
    fn build(&self, app: &mut App) {
        debug_assert!(app.is_plugin_added::<EguiPlugin>());
        app.add_systems(EguiPrimaryContextPass, viewer_ui);
    }
}

fn viewer_ui(
    mut contexts: EguiContexts,
    mut generation: ResMut<GenerationSettings>,
    mut layers: ResMut<LayerSettings>,
    status: Res<GenerationStatus>,
    world: Res<GeneratedWorld>,
    mut regenerate: MessageWriter<RegenerateWorld>,
) -> Result {
    egui::SidePanel::left("controls")
        .default_width(250.0)
        .resizable(false)
        .show(contexts.ctx_mut()?, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Sphere topology");
                ui.add_space(6.0);
                generation_controls(ui, &mut generation, &mut regenerate);

                if let Some(error) = &status.last_error {
                    ui.colored_label(egui::Color32::from_rgb(255, 110, 110), error);
                }

                ui.separator();
                layer_controls(ui, &mut layers);

                ui.separator();
                world_summary(ui, &world);

                ui.separator();
                ui.label("Drag the viewport to orbit.");
                ui.label("Scroll to zoom.");
                ui.label("Axes: X red, Y green, Z blue.");
            });
        });
    Ok(())
}

fn generation_controls(
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
    section(ui, "Coarse elevation", |ui| {
        elevation_controls(ui, &mut generation.elevation)
    });
    section(ui, "Mantle hotspots", |ui| {
        hotspot_controls(ui, &mut generation.hotspots)
    });
    if ui.button("Regenerate").clicked() {
        regenerate.write_default();
    }
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

fn layer_controls(ui: &mut egui::Ui, layers: &mut LayerSettings) {
    ui.label("Layers");
    for layer in DiagnosticLayer::ALL {
        // Only mutably access the resource when egui reports a real change.
        let mut visible = layers.is_visible(layer);
        if ui.checkbox(&mut visible, layer.label()).changed() {
            layers.set_visible(layer, visible);
        }
    }
}

fn world_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    stat_grid(ui, "Active world", "stats", |ui| {
        stat(ui, "Cells", world.voronoi.cell_count());
        stat(ui, "Vertices", world.voronoi.vertex_count());
        stat(ui, "Edges", world.voronoi.edge_count());
        stat(ui, "Plates", world.plates.plate_count);
        stat(ui, "Sampling seed", world.config.fibonacci.seed);
        stat(ui, "Plate seed", world.config.plates.seed);
        stat(ui, "Crust seed", world.config.crust.seed);
        stat(ui, "Motion seed", world.config.kinematics.seed);
        stat(ui, "Hotspot seed", world.config.hotspots.seed);
        stat(
            ui,
            "Jitter",
            format!("{:.2}", world.config.fibonacci.jitter),
        );
    });

    stat_grid(ui, "Static crust", "crust", |ui| {
        stat(
            ui,
            "Target ocean area",
            format!("{:.2}%", world.config.crust.target_ocean_fraction * 100.0),
        );
        stat(
            ui,
            "Achieved ocean area",
            format!(
                "{:.2}%",
                world.crust.ocean_fraction(&world.voronoi, &world.plates) * 100.0
            ),
        );
        stat(
            ui,
            "Oceanic plates",
            world.crust.plate_count(CrustClass::Oceanic),
        );
        stat(
            ui,
            "Continental plates",
            world.crust.plate_count(CrustClass::Continental),
        );
    });

    stat_grid(ui, "Plate evolution", "evolution", |ui| {
        stat(ui, "Active steps", world.evolution.active_step_count);
        stat(ui, "Proposals", world.evolution.proposal_count);
        stat(
            ui,
            "Contested cell events",
            world.evolution.contested_cell_count,
        );
        stat(ui, "Migration events", world.evolution.migrated_cell_count);
        stat(
            ui,
            "Strongest migration",
            format!("{:.3}", world.evolution.maximum_convergence),
        );
    });

    stat_grid(ui, "Static boundaries", "boundaries", |ui| {
        stat(
            ui,
            "Convergent",
            world.boundaries.count(BoundaryClass::Convergent),
        );
        stat(
            ui,
            "Divergent",
            world.boundaries.count(BoundaryClass::Divergent),
        );
        stat(
            ui,
            "Transform",
            world.boundaries.count(BoundaryClass::Transform),
        );
    });

    stat_grid(ui, "Seafloor age", "seafloor_age", |ui| {
        field_summary_stats(ui, &world.seafloor_age.diagnostics.summary);
        stat(
            ui,
            "Oceanic cells",
            world.seafloor_age.diagnostics.oceanic_cell_count,
        );
        stat(
            ui,
            "Ridge cells",
            world.seafloor_age.diagnostics.ridge_cell_count,
        );
        stat(
            ui,
            "Ridge plates",
            world.seafloor_age.diagnostics.ridge_plate_count,
        );
        stat(
            ui,
            "Ridge-less plates",
            world.seafloor_age.diagnostics.ridge_less_plate_count,
        );
        stat(
            ui,
            "Fallback cells",
            world.seafloor_age.diagnostics.fallback_cell_count,
        );
    });

    stat_grid(ui, "Boundary deformation", "deformation", |ui| {
        field_summary_stats(ui, &world.deformation.diagnostics.summary);
        stat(
            ui,
            "Sources",
            world.deformation.diagnostics.source_cell_count,
        );
        stat(
            ui,
            "Affected",
            world.deformation.diagnostics.affected_cell_count(),
        );
        stat(
            ui,
            "Uplifted",
            world.deformation.diagnostics.uplifted_cell_count,
        );
        stat(
            ui,
            "Subsided",
            world.deformation.diagnostics.subsided_cell_count,
        );
    });
    stat_grid(ui, "Base elevation", "base_elevation", |ui| {
        field_summary_stats(ui, &world.base_elevation.diagnostics.summary);
        stat(
            ui,
            "Oceanic range",
            format_field_range(&world.base_elevation.diagnostics.oceanic),
        );
        stat(
            ui,
            "Oceanic cells",
            world.base_elevation.diagnostics.oceanic_cell_count,
        );
        stat(
            ui,
            "Continental cells",
            world.base_elevation.diagnostics.continental_cell_count,
        );
    });
    stat_grid(ui, "Coarse elevation", "elevation", |ui| {
        field_summary_stats(ui, &world.elevation.diagnostics);
    });
    stat_grid(ui, "Mantle hotspots", "hotspots", |ui| {
        stat(ui, "Hotspots", world.hotspots.diagnostics.hotspot_count);
        stat(
            ui,
            "Trail cells",
            world.hotspots.diagnostics.trail_cell_count,
        );
        stat(
            ui,
            "Affected cells",
            world.hotspots.diagnostics.affected_cell_count,
        );
        stat(
            ui,
            "Overlap cells",
            world.hotspots.diagnostics.overlap_cell_count,
        );
        stat(
            ui,
            "Stationary sources",
            world.hotspots.diagnostics.stationary_source_count,
        );
        stat(
            ui,
            "Trail length range",
            format!(
                "{} - {}",
                world.hotspots.diagnostics.shortest_trail_cells,
                world.hotspots.diagnostics.longest_trail_cells
            ),
        );
    });
    stat_grid(ui, "Timings", "timings", |ui| {
        for stage in world.timings.stages() {
            stat(ui, stage.label, millis(stage.duration));
        }
        stat(ui, "Total", millis(world.timings.total()));
    });
}

fn section(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(SECTION_SPACING);
    ui.label(title);
    content(ui);
}

fn stat_grid(ui: &mut egui::Ui, title: &str, id: &str, content: impl FnOnce(&mut egui::Ui)) {
    section(ui, title, |ui| {
        egui::Grid::new(id).num_columns(2).show(ui, content);
    });
}

fn stat(ui: &mut egui::Ui, label: &str, value: impl std::fmt::Display) {
    ui.label(label);
    ui.monospace(value.to_string());
    ui.end_row();
}

fn field_summary_stats(ui: &mut egui::Ui, summary: &FieldSummary) {
    stat(
        ui,
        "Range",
        format!("{:.3} - {:.3}", summary.minimum, summary.maximum),
    );
    stat(ui, "Mean", format!("{:.3}", summary.mean));
}

fn format_field_range(summary: &FieldSummary) -> String {
    format!("{:.3} - {:.3}", summary.minimum, summary.maximum)
}

fn millis(duration: std::time::Duration) -> String {
    format!("{:.2} ms", duration.as_secs_f64() * 1_000.0)
}

fn drag_value<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    speed: f64,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(value).range(range).speed(speed));
    });
}

fn slider(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::Slider::new(value, range));
    });
}
