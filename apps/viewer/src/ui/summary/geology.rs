use super::{field_summary_stats, format_field_range, stat, stat_grid};
use crate::model::GeneratedWorld;
use bevy_egui::egui;
use procgen_geology::ElevationEffectDiagnostics;

pub(super) fn hotspot_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    stat_grid(ui, "Mantle hotspots", "hotspots", |ui| {
        stat(ui, "Hotspots", world.hotspots.hotspots.len());
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
}

pub(super) fn volcanic_arc_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    stat_grid(ui, "Volcanic arcs", "volcanic_arcs", |ui| {
        stat(ui, "Segments", world.volcanic_arcs.segments.len());
        stat(
            ui,
            "Qualifying edges",
            world.volcanic_arcs.diagnostics.qualifying_edge_count,
        );
        stat(
            ui,
            "Boundary cells",
            world.volcanic_arcs.diagnostics.boundary_cell_count,
        );
        stat(
            ui,
            "Arc cells",
            world.volcanic_arcs.diagnostics.arc_cell_count,
        );
        stat(
            ui,
            "Affected cells",
            world.volcanic_arcs.diagnostics.affected_cell_count,
        );
        stat(
            ui,
            "Overlap cells",
            world.volcanic_arcs.diagnostics.overlap_cell_count,
        );
        stat(
            ui,
            "Peak candidates",
            world.volcanic_arcs.diagnostics.peak_count,
        );
        stat(
            ui,
            "Short segments discarded",
            world
                .volcanic_arcs
                .diagnostics
                .discarded_short_segment_count,
        );
        stat(
            ui,
            "Landlocked discarded",
            world
                .volcanic_arcs
                .diagnostics
                .discarded_landlocked_segment_count,
        );
    });
}

pub(super) fn oceanic_peak_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    stat_grid(ui, "Seamounts and abyssal hills", "oceanic_peaks", |ui| {
        field_summary_stats(ui, &world.oceanic_peaks.diagnostics.density);
        stat(
            ui,
            "Oceanic cells",
            world.oceanic_peaks.diagnostics.oceanic_cell_count,
        );
        stat(
            ui,
            "Hotspot candidates",
            world.oceanic_peaks.diagnostics.hotspot_candidate_cell_count,
        );
        stat(
            ui,
            "Young-age candidates",
            world
                .oceanic_peaks
                .diagnostics
                .young_seafloor_candidate_cell_count,
        );
        stat(
            ui,
            "Overlap cells",
            world.oceanic_peaks.diagnostics.overlap_cell_count,
        );
        stat(ui, "Peaks", world.oceanic_peaks.diagnostics.peak_count);
        stat(
            ui,
            "Seamount peaks",
            world.oceanic_peaks.diagnostics.seamount_peak_count,
        );
        stat(
            ui,
            "Abyssal-hill peaks",
            world.oceanic_peaks.diagnostics.abyssal_hill_peak_count,
        );
        stat(
            ui,
            "Height range",
            format_field_range(&world.oceanic_peaks.diagnostics.height),
        );
    });
}

pub(super) fn craton_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    stat_grid(ui, "Cratons", "cratons", |ui| {
        field_summary_stats(ui, &world.cratons.diagnostics.strength);
        stat(
            ui,
            "Boundary cells",
            world.cratons.diagnostics.boundary_cell_count,
        );
        stat(
            ui,
            "Continental land cells",
            world.cratons.diagnostics.continental_land_cell_count,
        );
        stat(
            ui,
            "Craton cells",
            world.cratons.diagnostics.craton_cell_count,
        );
        stat(
            ui,
            "Full-strength cells",
            world.cratons.diagnostics.full_strength_cell_count,
        );
        stat(
            ui,
            "Maximum boundary distance",
            world
                .cratons
                .diagnostics
                .maximum_boundary_distance
                .map_or_else(|| "None".to_owned(), |distance| distance.to_string()),
        );
    });
}

pub(super) fn basin_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    stat_grid(ui, "Sedimentary basins", "basins", |ui| {
        stat(
            ui,
            "Candidates",
            world.basins.diagnostics.candidate_cell_count,
        );
        stat(ui, "Components", world.basins.diagnostics.component_count);
        stat(ui, "Basins", world.basins.diagnostics.basin_count);
        stat(ui, "Basin cells", world.basins.diagnostics.basin_cell_count);
        stat(
            ui,
            "Rejected small",
            world.basins.diagnostics.rejected_small_component_count,
        );
        stat(
            ui,
            "Rejected ocean-exposed",
            world
                .basins
                .diagnostics
                .rejected_ocean_exposed_component_count,
        );
        stat(
            ui,
            "Basin size range",
            world.basins.diagnostics.basin_cell_count_range.map_or_else(
                || "None".to_owned(),
                |(minimum, maximum)| format!("{minimum} - {maximum}"),
            ),
        );
    });
}

pub(super) fn geological_elevation_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    let diagnostics = &world.geological_elevation.diagnostics;
    stat_grid(ui, "Geological elevation", "geological_elevation", |ui| {
        field_summary_stats(ui, &diagnostics.elevation);
        effect_stats(ui, "Hotspots", diagnostics.hotspots);
        effect_stats(ui, "Volcanic arcs", diagnostics.volcanic_arcs);
        effect_stats(ui, "Cratons", diagnostics.cratons);
        effect_stats(ui, "Basins", diagnostics.basins);
    });
}

pub(super) fn isostatic_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    let diagnostics = &world.isostasy.diagnostics;
    stat_grid(ui, "Isostatic adjustment", "isostasy", |ui| {
        stat(
            ui,
            "Support range",
            format_field_range(&diagnostics.support),
        );
        stat(
            ui,
            "Elevation range",
            format_field_range(&diagnostics.elevation),
        );
        stat(ui, "Oceanic unchanged", diagnostics.oceanic_cell_count);
        stat(
            ui,
            "Basin floors preserved",
            diagnostics.preserved_basin_cell_count,
        );
        effect_stats(ui, "Rise", diagnostics.adjustment.rise);
        effect_stats(ui, "Sink", diagnostics.adjustment.sink);
    });
}

fn effect_stats(ui: &mut egui::Ui, label: &str, effect: ElevationEffectDiagnostics) {
    stat(ui, &format!("{label} affected"), effect.affected_cell_count);
    stat(
        ui,
        &format!("{label} total delta"),
        format!("{:.3}", effect.total_delta),
    );
    stat(
        ui,
        &format!("{label} max abs delta"),
        format!("{:.3}", effect.maximum_absolute_delta),
    );
}
