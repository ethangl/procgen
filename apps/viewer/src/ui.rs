use crate::model::{
    GeneratedWorld, GenerationSettings, GenerationStatus, RegenerateWorld, WORLD_RADIUS,
};
use crate::render::{DiagnosticLayer, LayerSettings};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use procgen_sphere::FibonacciConfig;
use procgen_tectonics::{
    BoundaryClass, BoundaryEffect, CoarseElevationConfig, CrustClass, CrustClassificationConfig,
    PlateEvolutionConfig, PlateKinematicsConfig, PlatePartitionConfig,
};

const ANGULAR_SPEED_RANGE: std::ops::RangeInclusive<f32> = 0.0..=10.0;
const ANGULAR_SPEED_STEP: f64 = 0.01;
const EVOLUTION_STEP_RANGE: std::ops::RangeInclusive<usize> = 0..=256;
const ELEVATION_DEPTH_RANGE: std::ops::RangeInclusive<usize> = 0..=32;
const SMOOTHING_PASS_RANGE: std::ops::RangeInclusive<usize> = 0..=32;

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
    section(ui, "Coarse elevation", |ui| {
        elevation_controls(ui, &mut generation.elevation)
    });
    if ui.button("Regenerate").clicked() {
        regenerate.write_default();
    }
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

fn elevation_controls(ui: &mut egui::Ui, config: &mut CoarseElevationConfig) {
    slider(ui, "Oceanic base", &mut config.oceanic_base, 0.0..=1.0);
    slider(
        ui,
        "Continental base",
        &mut config.continental_base,
        0.0..=1.0,
    );
    boundary_effect_controls(ui, "Convergent", &mut config.convergent);
    boundary_effect_controls(ui, "Divergent", &mut config.divergent);
    boundary_effect_controls(ui, "Transform", &mut config.transform);
    boundary_effect_controls(ui, "Collision", &mut config.collision);
    boundary_effect_controls(ui, "Trench", &mut config.trench);
    slider(
        ui,
        "Saturation speed",
        &mut config.saturation_speed,
        0.01..=20.0,
    );
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
        ELEVATION_DEPTH_RANGE,
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

    stat_grid(ui, "Coarse elevation", "elevation", |ui| {
        stat(
            ui,
            "Range",
            format!(
                "{:.3} - {:.3}",
                world.elevation.diagnostics.minimum, world.elevation.diagnostics.maximum
            ),
        );
        stat(
            ui,
            "Mean",
            format!("{:.3}", world.elevation.diagnostics.mean),
        );
        stat(
            ui,
            "Boundary sources",
            world.elevation.diagnostics.boundary_source_cell_count,
        );
        stat(
            ui,
            "Boundary affected",
            world.elevation.diagnostics.boundary_affected_cell_count,
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
    ui.add_space(4.0);
    ui.label(title);
    content(ui);
}

fn stat_grid(ui: &mut egui::Ui, title: &str, id: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(6.0);
    ui.label(title);
    egui::Grid::new(id).num_columns(2).show(ui, content);
}

fn stat(ui: &mut egui::Ui, label: &str, value: impl std::fmt::Display) {
    ui.label(label);
    ui.monospace(value.to_string());
    ui.end_row();
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
