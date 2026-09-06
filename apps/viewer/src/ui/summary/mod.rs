mod climate;
mod geology;
mod tectonics;

use super::{field_summary_stats, format_field_range, millis, stat, stat_grid};
use crate::model::GeneratedWorld;
use bevy_egui::egui;

pub(super) fn world_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    active_world_summary(ui, world);
    tectonics::crust_summary(ui, world);
    tectonics::evolution_summary(ui, world);
    tectonics::boundary_summary(ui, world);
    tectonics::seafloor_age_summary(ui, world);
    tectonics::deformation_summary(ui, world);
    tectonics::base_elevation_summary(ui, world);
    tectonics::elevation_summary(ui, world);
    geology::hotspot_summary(ui, world);
    geology::volcanic_arc_summary(ui, world);
    geology::oceanic_peak_summary(ui, world);
    geology::craton_summary(ui, world);
    geology::basin_summary(ui, world);
    geology::geological_elevation_summary(ui, world);
    geology::isostatic_summary(ui, world);
    climate::planet_summary(ui, world);
    climate::solar_forcing_summary(ui, world);
    climate::radiative_equilibrium_summary(ui, world);
    climate::seasonal_thermal_summary(ui, world);
    climate::atmospheric_circulation_summary(ui, world);
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
