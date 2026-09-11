use super::{field_summary_stats, format_field_range, stat, stat_grid};
use crate::model::TectonicsWorld;
use bevy_egui::egui;
use procgen_tectonics::BoundaryClass;

pub(super) fn summary(ui: &mut egui::Ui, world: &TectonicsWorld) {
    crust_summary(ui, world);
    birth_prior_summary(ui, world);
    evolution_summary(ui, world);
    boundary_summary(ui, world);
    seafloor_age_summary(ui, world);
    deformation_summary(ui, world);
    base_elevation_summary(ui, world);
    elevation_summary(ui, world);
}

fn crust_summary(ui: &mut egui::Ui, world: &TectonicsWorld) {
    stat_grid(ui, "Crust", "crust", |ui| {
        // The classification's own numbers describe the world before step
        // zero; the cell counts are what evolution left behind.
        stat(
            ui,
            "Target continent area",
            format!("{:.2}%", world.config.crust.continental_fraction * 100.0),
        );
        stat(
            ui,
            "Grown continent area",
            format!("{:.2}%", world.crust.continental_fraction * 100.0),
        );
        stat(ui, "Nuclei", world.config.crust.nucleus_count);
        stat(ui, "Continents", world.crust.component_count);
        let [oceanic_cells, continental_cells] = world.cell_crust().cell_counts();
        stat(ui, "Oceanic cells", oceanic_cells);
        stat(ui, "Continental cells", continental_cells);
    });
}

fn birth_prior_summary(ui: &mut egui::Ui, world: &TectonicsWorld) {
    stat_grid(ui, "Crust birth prior", "birth_prior", |ui| {
        field_summary_stats(ui, &world.birth_prior.hops);
        stat(ui, "Oceanic cells", world.birth_prior.oceanic_cell_count);
        stat(ui, "Ridge cells", world.birth_prior.ridge_cell_count);
        stat(ui, "Ridge plates", world.birth_prior.ridge_plate_count);
        stat(
            ui,
            "Ridge-less plates",
            world.birth_prior.ridge_less_plate_count,
        );
        stat(ui, "Fallback cells", world.birth_prior.fallback_cell_count);
    });
}

fn evolution_summary(ui: &mut egui::Ui, world: &TectonicsWorld) {
    stat_grid(ui, "Plate evolution", "evolution", |ui| {
        stat(ui, "Active steps", world.evolution.active_step_count);
        stat(ui, "Owner changes", world.evolution.owner_change_count);
        stat(
            ui,
            "Subducted particles",
            world.evolution.subducted_particle_count,
        );
        stat(ui, "Born particles", world.evolution.born_particle_count);
        stat(
            ui,
            "Collided cell events",
            world.evolution.collided_cell_count,
        );
        stat(
            ui,
            "Deepest collision",
            world.evolution.maximum_collision_stack,
        );
        stat(
            ui,
            "Sampled cell events",
            world.evolution.sampled_cell_count,
        );
        stat(
            ui,
            "Continental particles",
            format!(
                "{} to {}",
                world.evolution.starting_continental_particle_count,
                world.evolution.final_continental_particle_count
            ),
        );
        stat(ui, "Rifts", world.evolution.rift_count);
        stat(ui, "Failed rifts", world.evolution.failed_rift_count);
        stat(ui, "Sutures", world.evolution.suture_count);
    });
}

fn boundary_summary(ui: &mut egui::Ui, world: &TectonicsWorld) {
    // Not static any more: the poles drift, so these are the classes the run
    // ended on rather than the ones it started from.
    stat_grid(ui, "Final boundaries", "boundaries", |ui| {
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

fn seafloor_age_summary(ui: &mut egui::Ui, world: &TectonicsWorld) {
    stat_grid(ui, "Seafloor age", "seafloor_age", |ui| {
        field_summary_stats(ui, &world.seafloor_age.diagnostics.summary);
        stat(
            ui,
            "Oceanic cells",
            world.seafloor_age.diagnostics.oceanic_cell_count,
        );
    });
}

fn deformation_summary(ui: &mut egui::Ui, world: &TectonicsWorld) {
    stat_grid(ui, "Boundary deformation", "deformation", |ui| {
        field_summary_stats(ui, &world.deformation.diagnostics.summary);
        stat(
            ui,
            "Source cell events",
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

fn base_elevation_summary(ui: &mut egui::Ui, world: &TectonicsWorld) {
    stat_grid(ui, "Base elevation", "base_elevation", |ui| {
        field_summary_stats(ui, &world.base_elevation.diagnostics.summary);
        stat(
            ui,
            "Oceanic range",
            format_field_range(&world.base_elevation.diagnostics.oceanic),
        );
        stat(
            ui,
            "Dynamic topography",
            format_field_range(&world.base_elevation.diagnostics.dynamic_topography),
        );
        stat(
            ui,
            "Basement",
            format_field_range(&world.base_elevation.diagnostics.basement),
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
        stat(
            ui,
            "Margin cells",
            world.base_elevation.diagnostics.margin_cell_count,
        );
        stat(
            ui,
            "Margin depth",
            format_field_range(&world.base_elevation.diagnostics.margin_depth),
        );
    });
}

fn elevation_summary(ui: &mut egui::Ui, world: &TectonicsWorld) {
    stat_grid(ui, "Tectonic elevation", "elevation", |ui| {
        field_summary_stats(ui, &world.elevation.diagnostics);
        stat(ui, "Sea level", format!("{:.3}", world.elevation.sea_level));
        stat(ui, "Land cells", world.elevation.field().land_cell_count());
    });
}
