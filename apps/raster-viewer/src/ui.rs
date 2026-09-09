//! Settings panel, stage-timing readout, and run diagnostics.

use crate::{
    render::{DisplaySettings, GRID_QUAD_RANGE, SurfaceLayer},
    tectonics::{
        EVOLUTION_STEP_RANGE, FACE_RESOLUTIONS, GROWTH_ROUGHNESS_RANGE, HEAD_START_ARC_RANGE,
        MAJOR_PLATE_RANGE, MINOR_PLATE_RANGE, OCEAN_FRACTION_RANGE, ResidentTectonics,
        TectonicsSettings,
    },
};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use procgen_raster_tectonics::PipelineStage;
use procgen_tectonics::BoundaryClass;

pub struct RasterViewerUiPlugin;

impl Plugin for RasterViewerUiPlugin {
    fn build(&self, app: &mut App) {
        debug_assert!(app.is_plugin_added::<EguiPlugin>());
        app.add_systems(EguiPrimaryContextPass, raster_viewer_ui);
    }
}

fn raster_viewer_ui(
    mut contexts: EguiContexts,
    mut settings: ResMut<TectonicsSettings>,
    mut display: ResMut<DisplaySettings>,
    tectonics: Option<Res<ResidentTectonics>>,
) -> Result {
    let mut next_settings = *settings;
    let mut next_display = *display;
    egui::SidePanel::left("raster controls")
        .default_width(300.0)
        .resizable(false)
        .show(contexts.ctx_mut()?, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Raster");
                resolution_control(ui, &mut next_settings);
                ui.add(
                    egui::Slider::new(&mut next_display.grid_quads, GRID_QUAD_RANGE)
                        .text("Grid quads per face"),
                );
                layer_control(ui, &mut next_display);

                ui.separator();
                ui.heading("Plates");
                partition_controls(ui, &mut next_settings);

                ui.separator();
                ui.heading("Evolution");
                evolution_controls(ui, &mut next_settings);

                ui.separator();
                ui.heading("Run");
                match &tectonics {
                    Some(tectonics) => run_summary(ui, tectonics),
                    None => {
                        ui.label("Generating the first world.");
                    }
                }

                ui.separator();
                ui.label("Drag the viewport to orbit.");
                ui.label("Scroll to zoom.");
            });
        });
    settings.set_if_neq(next_settings);
    display.set_if_neq(next_display);
    Ok(())
}

fn resolution_control(ui: &mut egui::Ui, settings: &mut TectonicsSettings) {
    ui.horizontal(|ui| {
        ui.label("Texels per face");
        for resolution in FACE_RESOLUTIONS {
            ui.selectable_value(&mut settings.resolution, resolution, resolution.to_string());
        }
    });
}

fn layer_control(ui: &mut egui::Ui, display: &mut DisplaySettings) {
    ui.horizontal(|ui| {
        ui.label("Layer");
        for layer in SurfaceLayer::ALL {
            ui.selectable_value(&mut display.layer, layer, layer.label());
        }
    });
}

fn partition_controls(ui: &mut egui::Ui, settings: &mut TectonicsSettings) {
    let partition = &mut settings.config.partition;
    ui.add(egui::Slider::new(&mut partition.major_plate_count, MAJOR_PLATE_RANGE).text("Major"));
    ui.add(egui::Slider::new(&mut partition.minor_plate_count, MINOR_PLATE_RANGE).text("Minor"));
    ui.add(
        egui::Slider::new(&mut partition.major_head_start_arc, HEAD_START_ARC_RANGE)
            .text("Head start (rad)"),
    );
    ui.add(
        egui::Slider::new(&mut partition.growth_roughness, GROWTH_ROUGHNESS_RANGE)
            .text("Growth roughness %"),
    );
    ui.add(egui::DragValue::new(&mut partition.seed).prefix("Seed "));
}

fn evolution_controls(ui: &mut egui::Ui, settings: &mut TectonicsSettings) {
    let maximum_convergence = settings.maximum_convergence();
    let evolution = &mut settings.config.evolution;
    ui.add(
        egui::Slider::new(
            &mut evolution.crust.target_ocean_fraction,
            OCEAN_FRACTION_RANGE,
        )
        .text("Target ocean fraction"),
    );
    ui.add(egui::DragValue::new(&mut evolution.crust.seed).prefix("Crust seed "));
    ui.add(egui::DragValue::new(&mut evolution.kinematics.seed).prefix("Motion seed "));
    ui.add(
        egui::Slider::new(
            &mut evolution.migration.minimum_convergence,
            0.0..=maximum_convergence,
        )
        .text("Minimum convergence"),
    );
    ui.add(egui::Slider::new(&mut evolution.step_count, EVOLUTION_STEP_RANGE).text("Steps"));
}

fn run_summary(ui: &mut egui::Ui, tectonics: &ResidentTectonics) {
    let pipeline = tectonics.pipeline();
    let run = tectonics.run();
    ui.label(format!(
        "{} cells at {} texels per face",
        pipeline.cell_count(),
        pipeline.resolution()
    ));
    for stage in PipelineStage::ALL {
        ui.label(format!(
            "{:<11} {:>8.2} ms",
            stage.label(),
            millis(run.timings.duration(stage))
        ));
    }
    ui.label(format!(
        "{:<11} {:>8.2} ms",
        "Total",
        millis(run.timings.total())
    ));

    ui.separator();
    ui.heading("Diagnostics");
    let diagnostics = &run.diagnostics;
    ui.label(format!(
        "Frontier passes {} of {}",
        diagnostics.longest_relaxation_passes, run.pass_budget
    ));
    ui.label(format!("Ocean fraction {:.3}", diagnostics.ocean_fraction));
    ui.label(format!(
        "Migrated cells {}",
        diagnostics.migrated_cell_count
    ));
    if diagnostics.empty_plate_count > 0 {
        ui.label(format!(
            "Plates left seedless {}",
            diagnostics.empty_plate_count
        ));
    }
    for class in BoundaryClass::ALL {
        ui.label(format!(
            "{:<11} {:>6.2}%  ({})",
            format!("{class:?}"),
            100.0 * diagnostics.boundary_fraction(class),
            diagnostics.count(class)
        ));
    }
    if !run.settled() {
        ui.colored_label(
            egui::Color32::LIGHT_RED,
            "The relaxation exhausted its pass budget and did not settle.",
        );
    }
}

fn millis(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
