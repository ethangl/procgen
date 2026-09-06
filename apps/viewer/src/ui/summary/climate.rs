use super::{stat, stat_grid};
use crate::model::GeneratedWorld;
use bevy_egui::egui;
use procgen_climate::AreaWeightedSummary;

pub(super) fn summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    planet_summary(ui, world);
    solar_forcing_summary(ui, world);
    radiative_equilibrium_summary(ui, world);
    seasonal_thermal_summary(ui, world);
    atmospheric_circulation_summary(ui, world);
    moisture_transport_summary(ui, world);
    cryosphere_summary(ui, world);
    climate_coupling_summary(ui, world);
}

fn climate_coupling_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    let diagnostics = world.climate_coupling_diagnostics;
    let config = world.config.climate_coupling;
    stat_grid(ui, "Climate coupling", "climate_coupling", |ui| {
        stat(ui, "Iterations", diagnostics.iterations);
        stat(ui, "Iteration limit", config.maximum_iterations);
        stat(
            ui,
            "Under-relaxation",
            format!("{:.3}", config.under_relaxation),
        );
        stat(
            ui,
            "Albedo RMS residual",
            format!("{:.3e}", diagnostics.albedo_residual_rms),
        );
        stat(
            ui,
            "Temperature RMS change",
            format!("{:.3e} K", diagnostics.temperature_change_rms_kelvin),
        );
        stat(
            ui,
            "Precipitation RMS change",
            format!(
                "{:.3e} kg/m2/day",
                diagnostics.precipitation_change_rms_kg_per_m2_per_day
            ),
        );
        stat(
            ui,
            "Cover RMS change",
            format!("{:.3e}", diagnostics.cover_fraction_change_rms),
        );
        stat(
            ui,
            "Radiative residual",
            format!(
                "{:.3e} W/m2",
                diagnostics.maximum_radiative_balance_error_watts_per_square_meter
            ),
        );
        stat(
            ui,
            "Moisture residual",
            format!(
                "{:.3e} kg/m2",
                diagnostics.moisture_mass_balance_error_kg_per_m2
            ),
        );
        stat(
            ui,
            "Snow residual",
            format!(
                "{:.3e} kg/m2",
                diagnostics.snow_mass_balance_error_kg_per_m2
            ),
        );
        stat(
            ui,
            "Land-ice residual",
            format!("{:.3e} kg/m2", diagnostics.land_ice_mass_balance_kg_per_m2),
        );
        stat(
            ui,
            "Sea-ice residual",
            format!("{:.3e}", diagnostics.sea_ice_cover_balance_error),
        );
    });
}

fn cryosphere_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    let diagnostics = &world.cryosphere.diagnostics;
    let config = world.config.climate_coupling.cryosphere;
    stat_grid(ui, "Cryosphere", "cryosphere", |ui| {
        stat(ui, "Maximum refinements", config.maximum_iterations);
        stat(ui, "Refinements used", diagnostics.maximum_iterations_used);
        area_weighted_stats(
            ui,
            "Selected snowfall",
            "kg/m2/day",
            &diagnostics.selected_snowfall_kg_per_m2_per_day,
            scientific_three,
        );
        area_weighted_stats(
            ui,
            "Selected melt",
            "kg/m2/day",
            &diagnostics.selected_melt_kg_per_m2_per_day,
            scientific_three,
        );
        area_weighted_stats(
            ui,
            "Snow cover",
            "fraction",
            &diagnostics.selected_snow_cover_fraction,
            scientific_three,
        );
        area_weighted_stats(
            ui,
            "Land-ice cover",
            "fraction",
            &diagnostics.land_ice_cover_fraction,
            scientific_three,
        );
        area_weighted_stats(
            ui,
            "Sea-ice cover",
            "fraction",
            &diagnostics.selected_sea_ice_cover_fraction,
            scientific_three,
        );
        area_weighted_stats(
            ui,
            "Annual snowfall",
            "kg/m2",
            &diagnostics.annual_snowfall_kg_per_m2,
            scientific_three,
        );
        area_weighted_stats(
            ui,
            "Annual snow melt",
            "kg/m2",
            &diagnostics.annual_snow_melt_kg_per_m2,
            scientific_three,
        );
        area_weighted_stats(
            ui,
            "Land-ice accumulation",
            "kg/m2",
            &diagnostics.annual_land_ice_accumulation_kg_per_m2,
            scientific_three,
        );
        area_weighted_stats(
            ui,
            "Land-ice ablation",
            "kg/m2",
            &diagnostics.annual_land_ice_ablation_kg_per_m2,
            scientific_three,
        );
        area_weighted_stats(
            ui,
            "Sea-ice growth",
            "fraction",
            &diagnostics.annual_sea_ice_growth_fraction,
            scientific_three,
        );
        area_weighted_stats(
            ui,
            "Sea-ice melt",
            "fraction",
            &diagnostics.annual_sea_ice_melt_fraction,
            scientific_three,
        );
        stat(
            ui,
            "Snow-covered cells",
            diagnostics.snow_covered_cell_count,
        );
        stat(ui, "Land-ice cells", diagnostics.land_ice_cell_count);
        stat(ui, "Sea-ice cells", diagnostics.sea_ice_cell_count);
        stat(
            ui,
            "Snow closure",
            format!(
                "{:.3e} kg/m2",
                diagnostics.maximum_snow_closure_error_kg_per_m2
            ),
        );
        stat(
            ui,
            "Sea-ice closure",
            format!("{:.3e}", diagnostics.maximum_sea_ice_closure_error),
        );
        stat(
            ui,
            "Snow mass residual",
            format!(
                "{:.3e} kg/m2",
                diagnostics.snow_mass_balance_error_kg_per_m2
            ),
        );
        stat(
            ui,
            "Land-ice mass balance",
            format!("{:.3e} kg/m2", diagnostics.land_ice_mass_balance_kg_per_m2),
        );
        stat(
            ui,
            "Sea-ice cover residual",
            format!("{:.3e}", diagnostics.sea_ice_cover_balance_error),
        );
    });
}

fn moisture_transport_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    let diagnostics = &world.moisture_transport.diagnostics;
    let config = world.config.climate_coupling.moisture_transport;
    stat_grid(ui, "Moisture and precipitation", "moisture", |ui| {
        stat(ui, "Steps", config.step_count);
        stat(
            ui,
            "Step duration",
            format!("{:.1} h", config.step_seconds / 3_600.0),
        );
        stat(
            ui,
            "Simulated duration",
            format!("{:.1} days", diagnostics.simulated_days),
        );
        area_weighted_stats(
            ui,
            "Humidity",
            "kg/m2",
            &diagnostics.humidity_kg_per_m2,
            fixed_one,
        );
        area_weighted_stats(
            ui,
            "Capacity",
            "kg/m2",
            &diagnostics.moisture_capacity_kg_per_m2,
            fixed_one,
        );
        area_weighted_stats(
            ui,
            "Evaporation",
            "kg/m2/day",
            &diagnostics.evaporation_kg_per_m2_per_day,
            scientific_three,
        );
        area_weighted_stats(
            ui,
            "Precipitation",
            "kg/m2/day",
            &diagnostics.precipitation_kg_per_m2_per_day,
            scientific_three,
        );
        area_weighted_stats(
            ui,
            "Condensation",
            "kg/m2/day",
            &diagnostics.condensation_kg_per_m2_per_day,
            scientific_three,
        );
        area_weighted_stats(
            ui,
            "Orographic",
            "kg/m2/day",
            &diagnostics.orographic_precipitation_kg_per_m2_per_day,
            scientific_three,
        );
        stat(ui, "Ocean cells", diagnostics.ocean_cell_count);
        stat(
            ui,
            "Precipitating cells",
            diagnostics.precipitating_cell_count,
        );
        stat(ui, "Orographic cells", diagnostics.orographic_cell_count);
        stat(
            ui,
            "Maximum orographic fraction",
            format!("{:.3}", diagnostics.maximum_orographic_fraction_per_step),
        );
        stat(
            ui,
            "Mass-balance residual",
            format!("{:.3e} kg/m2", diagnostics.mass_balance_error_kg_per_m2),
        );
    });
}

fn planet_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
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
        stat(
            ui,
            "Maximum land elevation",
            format!("{:.1} km", planet.maximum_land_elevation_meters / 1_000.0),
        );
    });
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
fn radiative_equilibrium_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    let diagnostics = &world.radiative_equilibrium.diagnostics;
    stat_grid(ui, "Radiative equilibrium", "radiative_equilibrium", |ui| {
        stat(ui, "Albedo", "Per-cell coupled");
        stat(
            ui,
            "Emissivity",
            format!(
                "{:.3}",
                world
                    .config
                    .climate_coupling
                    .radiative_equilibrium
                    .emissivity
            ),
        );
        area_weighted_stats(ui, "Daily", "K", &diagnostics.daily, fixed_one);
        area_weighted_stats(ui, "Annual", "K", &diagnostics.annual, fixed_one);
    });
}

fn seasonal_thermal_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    let diagnostics = &world.seasonal_thermal.diagnostics;
    let config = world.config.climate_coupling.seasonal_thermal;
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

fn atmospheric_circulation_summary(ui: &mut egui::Ui, world: &GeneratedWorld) {
    let diagnostics = &world.atmospheric_circulation.diagnostics;
    let config = world.config.climate_coupling.atmospheric_circulation;
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
