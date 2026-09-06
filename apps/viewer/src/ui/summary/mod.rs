mod climate;
mod geology;
mod tectonics;

use super::section;
use crate::model::GeneratedWorld;
use bevy_egui::egui;
use procgen_tectonics::FieldSummary;

pub(super) fn world_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    active_world_summary(ui, world);
    tectonics::summary(ui, world);
    geology::summary(ui, world);
    climate::summary(ui, world);
    timing_summary(ui, world);
}

fn active_world_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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
        stat(ui, "Peak seed", world.config.oceanic_peaks.seed);
        stat(
            ui,
            "Jitter",
            format!("{:.2}", world.config.fibonacci.jitter),
        );
    });
}

fn timing_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    stat_grid(ui, "Timings", "timings", |ui| {
        for stage in world.timings.stages() {
            stat(ui, stage.label, millis(stage.duration));
        }
        stat(ui, "Total", millis(world.timings.total()));
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
