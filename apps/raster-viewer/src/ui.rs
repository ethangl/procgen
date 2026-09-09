//! Settings panel and stage-timing readout.

use crate::{
    partition::{
        FACE_RESOLUTIONS, GROWTH_ROUGHNESS_RANGE, HEAD_START_ARC_RANGE, MAJOR_PLATE_RANGE,
        MINOR_PLATE_RANGE, PartitionSettings, ResidentPartition,
    },
    render::{DisplaySettings, GRID_QUAD_RANGE},
};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use procgen_raster_tectonics::PipelineStage;

pub struct RasterViewerUiPlugin;

impl Plugin for RasterViewerUiPlugin {
    fn build(&self, app: &mut App) {
        debug_assert!(app.is_plugin_added::<EguiPlugin>());
        app.add_systems(EguiPrimaryContextPass, raster_viewer_ui);
    }
}

fn raster_viewer_ui(
    mut contexts: EguiContexts,
    mut settings: ResMut<PartitionSettings>,
    mut display: ResMut<DisplaySettings>,
    partition: Option<Res<ResidentPartition>>,
) -> Result {
    let mut next_settings = *settings;
    let mut next_display = *display;
    egui::SidePanel::left("raster controls")
        .default_width(280.0)
        .resizable(false)
        .show(contexts.ctx_mut()?, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Raster");
                resolution_control(ui, &mut next_settings);
                ui.add(
                    egui::Slider::new(&mut next_display.grid_quads, GRID_QUAD_RANGE)
                        .text("Grid quads per face"),
                );

                ui.separator();
                ui.heading("Plates");
                partition_controls(ui, &mut next_settings);

                ui.separator();
                ui.heading("Run");
                match &partition {
                    Some(partition) => run_summary(ui, partition),
                    None => {
                        ui.label("Generating the first partition.");
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

fn resolution_control(ui: &mut egui::Ui, settings: &mut PartitionSettings) {
    ui.horizontal(|ui| {
        ui.label("Texels per face");
        for resolution in FACE_RESOLUTIONS {
            ui.selectable_value(&mut settings.resolution, resolution, resolution.to_string());
        }
    });
}

fn partition_controls(ui: &mut egui::Ui, settings: &mut PartitionSettings) {
    let config = &mut settings.config;
    ui.add(egui::Slider::new(&mut config.major_plate_count, MAJOR_PLATE_RANGE).text("Major"));
    ui.add(egui::Slider::new(&mut config.minor_plate_count, MINOR_PLATE_RANGE).text("Minor"));
    ui.add(
        egui::Slider::new(&mut config.major_head_start_arc, HEAD_START_ARC_RANGE)
            .text("Head start (rad)"),
    );
    ui.add(
        egui::Slider::new(&mut config.growth_roughness, GROWTH_ROUGHNESS_RANGE)
            .text("Growth roughness %"),
    );
    ui.add(egui::DragValue::new(&mut config.seed).prefix("Seed "));
}

fn run_summary(ui: &mut egui::Ui, partition: &ResidentPartition) {
    let pipeline = partition.pipeline();
    let run = partition.run();
    ui.label(format!(
        "{} cells at {} texels per face",
        pipeline.cell_count(),
        pipeline.resolution()
    ));
    for stage in PipelineStage::ALL {
        ui.label(format!(
            "{:<13} {:>8.2} ms",
            stage.label(),
            millis(run.timings.duration(stage))
        ));
    }
    ui.label(format!(
        "{:<13} {:>8.2} ms",
        "Total",
        millis(run.timings.total())
    ));
    ui.label(format!(
        "Frontier passes {} of {}",
        run.longest_relaxation_passes, run.pass_budget
    ));
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
