use super::{stat, stat_grid};
use crate::model::GeneratedWorld;
use bevy_egui::egui;
use procgen_climate::AreaWeightedSummary;

pub(super) fn planet_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    let planet = world.config.planet;
    stat_grid(ui, "Planet", "planet", |ui| {
        stat(
            ui,
            "Radius",
            format!("{:.1} km", planet.radius_meters / 1_000.0),
        );
        stat(
            ui,
            "Rotation period",
            format!("{:.1} s", planet.sidereal_rotation_period_seconds),
        );
        stat(
            ui,
            "Atmospheric gas constant",
            format!(
                "{:.2} J/kg/K",
                planet.atmospheric_specific_gas_constant_joules_per_kilogram_kelvin
            ),
        );
    });
}

pub(super) fn atmospheric_circulation_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    let diagnostics = &world.atmospheric_circulation.diagnostics;
    let config = world.config.atmospheric_circulation;
    stat_grid(
        ui,
        "Atmospheric circulation",
        "atmospheric_circulation",
        |ui| {
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
            area_weighted_stats(
                ui,
                "Wind speed",
                "m/s",
                &diagnostics.wind_speed_meters_per_second,
                scientific_three,
            );
            area_weighted_stats(
                ui,
                "Temperature gradient",
                "K/rad",
                &diagnostics.temperature_gradient_kelvin_per_radian,
                scientific_three,
            );
            area_weighted_stats(
                ui,
                "Pressure acceleration",
                "m/s2",
                &diagnostics.pressure_gradient_acceleration_meters_per_second_squared,
                scientific_three,
            );
            area_weighted_stats(
                ui,
                "Coriolis",
                "s^-1",
                &diagnostics.coriolis_parameter_per_second,
                scientific_three,
            );
            area_weighted_stats(
                ui,
                "Terrain steering applied",
                "fraction",
                &diagnostics.terrain_steering_fraction,
                scientific_three,
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

pub(super) fn seasonal_thermal_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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
        area_weighted_stats(
            ui,
            "Selected phase",
            "K",
            &diagnostics.selected_phase,
            fixed_one,
        );
        area_weighted_stats(ui, "Annual mean", "K", &diagnostics.annual_mean, fixed_one);
        area_weighted_stats(
            ui,
            "Annual minimum",
            "K",
            &diagnostics.annual_minimum,
            fixed_one,
        );
        area_weighted_stats(
            ui,
            "Annual maximum",
            "K",
            &diagnostics.annual_maximum,
            fixed_one,
        );
        area_weighted_stats(
            ui,
            "Annual amplitude",
            "K",
            &diagnostics.annual_amplitude,
            fixed_one,
        );
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

pub(super) fn radiative_equilibrium_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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
        area_weighted_stats(ui, "Daily", "K", &diagnostics.daily, fixed_one);
        area_weighted_stats(ui, "Annual", "K", &diagnostics.annual, fixed_one);
    });
}

fn area_weighted_stats(
    ui: &mut egui::Ui,
    prefix: &str,
    unit: &str,
    summary: &AreaWeightedSummary,
    format_value: fn(f64) -> String,
) {
    stat(
        ui,
        &format!("{prefix} range"),
        format!(
            "{} - {} {unit}",
            format_value(f64::from(summary.minimum)),
            format_value(f64::from(summary.maximum))
        ),
    );
    stat(
        ui,
        &format!("{prefix} global mean"),
        format!("{} {unit}", format_value(summary.area_weighted_mean)),
    );
}

fn fixed_one(value: f64) -> String {
    format!("{value:.1}")
}

fn scientific_three(value: f64) -> String {
    format!("{value:.3e}")
}

pub(super) fn solar_forcing_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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
        area_weighted_stats(ui, "Daily", "W/m2", &diagnostics.daily_mean, fixed_one);
        area_weighted_stats(ui, "Annual", "W/m2", &diagnostics.annual_mean, fixed_one);
        stat(ui, "Polar-night cells", diagnostics.polar_night_cell_count);
        stat(ui, "Polar-day cells", diagnostics.polar_day_cell_count);
        stat(
            ui,
            "Annual samples",
            world.config.solar_forcing.annual_sample_count,
        );
    });
}
