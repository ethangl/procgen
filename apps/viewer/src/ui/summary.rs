use super::{field_summary_stats, format_field_range, millis, stat, stat_grid};
use crate::model::GeneratedWorld;
use bevy_egui::egui;
use procgen_climate::AreaWeightedSummary;
use procgen_geology::ElevationEffectDiagnostics;
use procgen_tectonics::{BoundaryClass, CrustClass};

pub(super) fn world_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    active_world_summary(ui, world);
    crust_summary(ui, world);
    evolution_summary(ui, world);
    boundary_summary(ui, world);
    seafloor_age_summary(ui, world);
    deformation_summary(ui, world);
    base_elevation_summary(ui, world);
    elevation_summary(ui, world);
    hotspot_summary(ui, world);
    volcanic_arc_summary(ui, world);
    oceanic_peak_summary(ui, world);
    craton_summary(ui, world);
    basin_summary(ui, world);
    geological_elevation_summary(ui, world);
    isostatic_summary(ui, world);
    solar_forcing_summary(ui, world);
    radiative_equilibrium_summary(ui, world);
    seasonal_thermal_summary(ui, world);
    atmospheric_circulation_summary(ui, world);
    timing_summary(ui, world);
}

fn atmospheric_circulation_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    let diagnostics = &world.atmospheric_circulation.diagnostics;
    let config = world.config.atmospheric_circulation;
    stat_grid(
        ui,
        "Atmospheric circulation",
        "atmospheric_circulation",
        |ui| {
            stat(
                ui,
                "Planet radius",
                format!("{:.1} km", world.config.planet.radius_meters / 1_000.0),
            );
            stat(
                ui,
                "Rotation period",
                format!(
                    "{:.1} s",
                    world.config.planet.rotation.sidereal_period_seconds
                ),
            );
            stat(
                ui,
                "Atmospheric gas constant",
                format!(
                    "{:.2} J/kg/K",
                    world
                        .config
                        .planet
                        .atmosphere
                        .specific_gas_constant_joules_per_kilogram_kelvin
                ),
            );
            stat(
                ui,
                "Surface drag",
                format!("{:.3e} s^-1", config.surface_drag_per_second),
            );
            stat(
                ui,
                "Terrain steering",
                format!("{:.2}", config.terrain_steering),
            );
            stat(
                ui,
                "Maximum wind",
                format!("{:.1} m/s", config.maximum_wind_speed_meters_per_second),
            );
            circulation_field_stats(
                ui,
                "Wind speed",
                "m/s",
                &diagnostics.wind_speed_meters_per_second,
            );
            circulation_field_stats(
                ui,
                "Temperature gradient",
                "K/rad",
                &diagnostics.temperature_gradient_kelvin_per_radian,
            );
            circulation_field_stats(
                ui,
                "Pressure acceleration",
                "m/s2",
                &diagnostics.pressure_gradient_acceleration_meters_per_second_squared,
            );
            circulation_field_stats(
                ui,
                "Coriolis",
                "s^-1",
                &diagnostics.coriolis_parameter_per_second,
            );
            circulation_field_stats(
                ui,
                "Terrain steering applied",
                "fraction",
                &diagnostics.terrain_steering_fraction,
            );
            stat(ui, "Calm cells", diagnostics.calm_cell_count);
            stat(
                ui,
                "Terrain-steered cells",
                diagnostics.terrain_steered_cell_count,
            );
            stat(
                ui,
                "Speed-capped cells",
                diagnostics.speed_capped_cell_count,
            );
            stat(
                ui,
                "Maximum tangency error",
                format!(
                    "{:.3e} m/s",
                    diagnostics.maximum_tangency_error_meters_per_second
                ),
            );
        },
    );
}

fn circulation_field_stats(
    ui: &mut egui::Ui,
    label: &str,
    unit: &str,
    summary: &AreaWeightedSummary,
) {
    stat(
        ui,
        &format!("{label} range"),
        format!("{:.3e} - {:.3e} {unit}", summary.minimum, summary.maximum),
    );
    stat(
        ui,
        &format!("{label} global mean"),
        format!("{:.3e} {unit}", summary.area_weighted_mean),
    );
}

fn seasonal_thermal_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    let diagnostics = &world.seasonal_thermal.diagnostics;
    let config = world.config.seasonal_thermal;
    stat_grid(ui, "Seasonal thermal response", "seasonal_thermal", |ui| {
        stat(
            ui,
            "Land heat capacity",
            format!("{:.3e} J/m2/K", config.land_heat_capacity),
        );
        stat(
            ui,
            "Ocean heat capacity",
            format!("{:.3e} J/m2/K", config.ocean_heat_capacity),
        );
        stat(
            ui,
            "Orbital period",
            format!("{:.3} days", config.orbital_period_days),
        );
        stat(
            ui,
            "Orbital samples",
            world.config.solar_forcing.annual_sample_count,
        );
        area_weighted_stats(ui, "Selected phase", "K", &diagnostics.selected_phase);
        area_weighted_stats(ui, "Annual mean", "K", &diagnostics.annual_mean);
        area_weighted_stats(ui, "Annual minimum", "K", &diagnostics.annual_minimum);
        area_weighted_stats(ui, "Annual maximum", "K", &diagnostics.annual_maximum);
        area_weighted_stats(ui, "Annual amplitude", "K", &diagnostics.annual_amplitude);
        stat(ui, "Land cells", diagnostics.land.cell_count);
        stat(ui, "Ocean cells", diagnostics.ocean.cell_count);
        stat(
            ui,
            "Selected land mean",
            optional_temperature(diagnostics.land.selected_area_weighted_mean_kelvin),
        );
        stat(
            ui,
            "Selected ocean mean",
            optional_temperature(diagnostics.ocean.selected_area_weighted_mean_kelvin),
        );
        stat(
            ui,
            "Periodic closure",
            format!(
                "{:.3e} K",
                diagnostics.maximum_periodic_closure_error_kelvin
            ),
        );
        stat(
            ui,
            "Fixed-point iterations",
            diagnostics.maximum_fixed_point_iterations,
        );
    });
}

fn optional_temperature(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.1} K"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn radiative_equilibrium_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    let diagnostics = &world.radiative_equilibrium.diagnostics;
    stat_grid(ui, "Radiative equilibrium", "radiative_equilibrium", |ui| {
        stat(
            ui,
            "Albedo",
            format!("{:.3}", world.config.radiative_equilibrium.albedo),
        );
        stat(
            ui,
            "Emissivity",
            format!("{:.3}", world.config.radiative_equilibrium.emissivity),
        );
        area_weighted_stats(ui, "Daily", "K", &diagnostics.daily);
        area_weighted_stats(ui, "Annual", "K", &diagnostics.annual);
    });
}

fn area_weighted_stats(ui: &mut egui::Ui, prefix: &str, unit: &str, summary: &AreaWeightedSummary) {
    stat(
        ui,
        &format!("{prefix} range"),
        format!("{:.1} - {:.1} {unit}", summary.minimum, summary.maximum),
    );
    stat(
        ui,
        &format!("{prefix} global mean"),
        format!("{:.1} {unit}", summary.area_weighted_mean),
    );
}

fn solar_forcing_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    let diagnostics = &world.solar_forcing.diagnostics;
    stat_grid(ui, "Solar forcing", "solar_forcing", |ui| {
        stat(
            ui,
            "Orbital phase",
            format!("{:.3}", diagnostics.orbital_phase),
        );
        stat(
            ui,
            "Orbital distance",
            format!("{:.3} Gm", diagnostics.orbital_distance_meters / 1.0e9),
        );
        stat(
            ui,
            "Solar declination",
            format!(
                "{:.2} deg",
                diagnostics.solar_declination_radians.to_degrees()
            ),
        );
        stat(
            ui,
            "Stellar flux",
            format!(
                "{:.1} W/m2",
                diagnostics.stellar_flux_watts_per_square_meter
            ),
        );
        area_weighted_stats(ui, "Daily", "W/m2", &diagnostics.daily_mean);
        area_weighted_stats(ui, "Annual", "W/m2", &diagnostics.annual_mean);
        stat(ui, "Polar-night cells", diagnostics.polar_night_cell_count);
        stat(ui, "Polar-day cells", diagnostics.polar_day_cell_count);
        stat(
            ui,
            "Annual samples",
            world.config.solar_forcing.annual_sample_count,
        );
    });
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

fn crust_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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

fn evolution_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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

fn boundary_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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

fn seafloor_age_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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

fn deformation_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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

fn base_elevation_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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

fn elevation_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    stat_grid(ui, "Tectonic elevation", "elevation", |ui| {
        field_summary_stats(ui, &world.elevation.diagnostics);
    });
}

fn hotspot_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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

fn volcanic_arc_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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

fn oceanic_peak_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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

fn craton_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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

fn basin_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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

fn geological_elevation_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    let diagnostics = &world.geological_elevation.diagnostics;
    stat_grid(ui, "Geological elevation", "geological_elevation", |ui| {
        field_summary_stats(ui, &diagnostics.elevation);
        effect_stats(ui, "Hotspots", diagnostics.hotspots);
        effect_stats(ui, "Volcanic arcs", diagnostics.volcanic_arcs);
        effect_stats(ui, "Cratons", diagnostics.cratons);
        effect_stats(ui, "Basins", diagnostics.basins);
    });
}

fn isostatic_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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

fn timing_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    stat_grid(ui, "Timings", "timings", |ui| {
        for stage in world.timings.stages() {
            stat(ui, stage.label, millis(stage.duration));
        }
        stat(ui, "Total", millis(world.timings.total()));
    });
}
