use super::{field_summary_stats, format_field_range, stat, stat_grid};
use crate::model::GeneratedWorld;
use bevy_egui::egui;
use procgen_tectonics::{BoundaryClass, CrustClass};

pub(super) fn crust_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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
}

pub(super) fn evolution_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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
}

pub(super) fn boundary_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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
}

pub(super) fn seafloor_age_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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
}

pub(super) fn deformation_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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
}

pub(super) fn base_elevation_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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
}

pub(super) fn elevation_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    stat_grid(ui, "Tectonic elevation", "elevation", |ui| {
        field_summary_stats(ui, &world.elevation.diagnostics);
    });
}
