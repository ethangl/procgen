mod climate;
mod geology;
mod tectonics;

use super::section;
use crate::model::{GeneratedWorld, GenerationTimings, GeologyWorld, Phase, TectonicsWorld};
use bevy_egui::egui;
use procgen_tectonics::FieldSummary;

/// The sidebar reports only the active phase, and only once that phase has
/// results.
pub(super) fn phase_summary(ui: &mut egui::Ui, phase: Phase, world: &GeneratedWorld) {
    match phase {
        Phase::Tectonics => match world.tectonics() {
            Some(results) => {
                mesh_summary(ui, results);
                tectonics::summary(ui, results);
                timing_summary(ui, &results.timings);
            }
            None => not_generated(ui, phase),
        },
        Phase::Geology => match world.geology() {
            Some(results) => {
                geology_seeds(ui, results);
                geology::summary(ui, results);
                timing_summary(ui, &results.timings);
            }
            None => not_generated(ui, phase),
        },
        Phase::Climate => match world.climate() {
            Some(results) => {
                climate::summary(ui, results);
                timing_summary(ui, &results.timings);
            }
            None => not_generated(ui, phase),
        },
    }
}

fn not_generated(ui: &mut egui::Ui, phase: Phase) {
    ui.label(format!("{} has not been generated.", phase.label()));
}

fn mesh_summary(ui: &mut egui::Ui, world: &TectonicsWorld) {
    stat_grid(ui, "Active mesh", "mesh-stats", |ui| {
        stat(ui, "Cells", world.voronoi.cell_count());
        stat(ui, "Vertices", world.voronoi.vertex_count());
        stat(ui, "Edges", world.voronoi.edge_count());
        stat(ui, "Plates", world.plates.plate_count);
        stat(ui, "Sampling seed", world.config.fibonacci.seed);
        stat(
            ui,
            "Jitter",
            format!("{:.2}", world.config.fibonacci.jitter),
        );
        stat(ui, "Plate seed", world.config.plates.seed);
        stat(ui, "Crust seed", world.config.crust.seed);
        stat(ui, "Motion seed", world.config.kinematics.seed);
        stat(ui, "Evolution seed", world.config.evolution.seed);
    });
}

fn geology_seeds(ui: &mut egui::Ui, world: &GeologyWorld) {
    stat_grid(ui, "Geology seeds", "geology-seeds", |ui| {
        stat(ui, "Hotspot seed", world.config.hotspots.seed);
        stat(ui, "Peak seed", world.config.oceanic_peaks.seed);
    });
}

fn timing_summary(ui: &mut egui::Ui, timings: &GenerationTimings) {
    stat_grid(ui, "Timings", "timings", |ui| {
        for stage in timings.stages() {
            stat(ui, stage.label, millis(stage.duration));
        }
        stat(ui, "Total", millis(timings.total()));
    });
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
    stat(ui, "Range", format_field_range(summary));
    stat(ui, "Mean", format!("{:.3}", summary.mean));
}

fn format_field_range(summary: &FieldSummary) -> String {
    format!("{:.3} - {:.3}", summary.minimum, summary.maximum)
}

fn millis(duration: std::time::Duration) -> String {
    format!("{:.2} ms", duration.as_secs_f64() * 1_000.0)
}
