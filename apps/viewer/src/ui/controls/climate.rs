use super::super::{drag_value, section, slider};
use crate::model::GenerationSettings;
use bevy_egui::egui;
use procgen_climate::{
    AtmosphericCirculationConfig, CLIMATE_COUPLING_FRACTION_TOLERANCE_RANGE,
    CLIMATE_COUPLING_ITERATION_LIMIT_RANGE, CLIMATE_COUPLING_TOLERANCE_RANGE,
    CRYOSPHERE_CLOSURE_TOLERANCE_RANGE, CRYOSPHERE_FRACTION_RATE_RANGE,
    CRYOSPHERE_ITERATION_LIMIT_RANGE, CRYOSPHERE_MASS_RANGE, CRYOSPHERE_RATE_RANGE,
    CRYOSPHERE_TEMPERATURE_RANGE, ClimateCouplingConfig, CryosphereConfig, DRAG_RATE_RANGE,
    MAXIMUM_WIND_SPEED_RANGE, MOISTURE_CAPACITY_RANGE, MOISTURE_RATE_RANGE,
    MOISTURE_STEP_COUNT_RANGE, MOISTURE_STEP_SECONDS_RANGE, MoistureTransportConfig,
    ORBITAL_PERIOD_DAYS_RANGE, OROGRAPHIC_COEFFICIENT_RANGE, REFERENCE_TEMPERATURE_KELVIN_RANGE,
    RadiativeEquilibriumConfig, SeasonalThermalConfig, TEMPERATURE_SENSITIVITY_RANGE,
    TERRAIN_STEERING_RANGE, THERMAL_CAPACITY_RANGE, TRANSPORT_FRACTION_RANGE,
};

pub(super) fn generation_controls(ui: &mut egui::Ui, generation: &mut GenerationSettings) {
    section(ui, "Radiative equilibrium", |ui| {
        radiative_equilibrium_controls(ui, &mut generation.climate_coupling.radiative_equilibrium)
    });
    section(ui, "Seasonal thermal response", |ui| {
        seasonal_thermal_controls(ui, &mut generation.climate_coupling.seasonal_thermal)
    });
    section(ui, "Atmospheric circulation", |ui| {
        atmospheric_circulation_controls(
            ui,
            &mut generation.climate_coupling.atmospheric_circulation,
        )
    });
    section(ui, "Moisture and precipitation", |ui| {
        moisture_transport_controls(ui, &mut generation.climate_coupling.moisture_transport)
    });
    section(ui, "Cryosphere", |ui| {
        cryosphere_controls(ui, &mut generation.climate_coupling.cryosphere)
    });
    section(ui, "Climate coupling", |ui| {
        climate_coupling_controls(ui, &mut generation.climate_coupling)
    });
}

fn climate_coupling_controls(ui: &mut egui::Ui, config: &mut ClimateCouplingConfig) {
    drag_value(
        ui,
        "Maximum iterations",
        &mut config.maximum_iterations,
        CLIMATE_COUPLING_ITERATION_LIMIT_RANGE,
        1.0,
    );
    slider(
        ui,
        "Under-relaxation",
        &mut config.under_relaxation,
        0.001..=1.0,
    );
    drag_value(
        ui,
        "Albedo RMS tolerance",
        &mut config.albedo_tolerance,
        CLIMATE_COUPLING_FRACTION_TOLERANCE_RANGE,
        0.001,
    );
    drag_value(
        ui,
        "Temperature RMS tolerance",
        &mut config.temperature_tolerance_kelvin,
        CLIMATE_COUPLING_TOLERANCE_RANGE,
        0.1,
    );
    drag_value(
        ui,
        "Precipitation RMS tolerance",
        &mut config.precipitation_tolerance_kg_per_m2_per_day,
        CLIMATE_COUPLING_TOLERANCE_RANGE,
        0.001,
    );
    drag_value(
        ui,
        "Cover RMS tolerance",
        &mut config.cover_fraction_tolerance,
        CLIMATE_COUPLING_FRACTION_TOLERANCE_RANGE,
        0.001,
    );
    slider(ui, "Land albedo", &mut config.albedo.land, 0.0..=1.0);
    slider(ui, "Ocean albedo", &mut config.albedo.ocean, 0.0..=1.0);
    slider(ui, "Snow albedo", &mut config.albedo.snow, 0.0..=1.0);
    slider(ui, "Ice albedo", &mut config.albedo.ice, 0.0..=1.0);
}

fn radiative_equilibrium_controls(ui: &mut egui::Ui, config: &mut RadiativeEquilibriumConfig) {
    slider(ui, "Emissivity", &mut config.emissivity, 0.01..=1.0);
}

fn seasonal_thermal_controls(ui: &mut egui::Ui, config: &mut SeasonalThermalConfig) {
    drag_value(
        ui,
        "Land heat capacity",
        &mut config.land_heat_capacity,
        THERMAL_CAPACITY_RANGE,
        1.0e6,
    );
    drag_value(
        ui,
        "Ocean heat capacity",
        &mut config.ocean_heat_capacity,
        THERMAL_CAPACITY_RANGE,
        1.0e6,
    );
    drag_value(
        ui,
        "Orbital period days",
        &mut config.orbital_period_days,
        ORBITAL_PERIOD_DAYS_RANGE,
        1.0,
    );
}

fn atmospheric_circulation_controls(ui: &mut egui::Ui, config: &mut AtmosphericCirculationConfig) {
    drag_value(
        ui,
        "Surface drag per second",
        &mut config.surface_drag_per_second,
        DRAG_RATE_RANGE,
        1.0e-6,
    );
    slider(
        ui,
        "Terrain steering",
        &mut config.terrain_steering,
        TERRAIN_STEERING_RANGE,
    );
    drag_value(
        ui,
        "Maximum wind speed",
        &mut config.maximum_wind_speed_meters_per_second,
        MAXIMUM_WIND_SPEED_RANGE,
        1.0,
    );
}

fn moisture_transport_controls(ui: &mut egui::Ui, config: &mut MoistureTransportConfig) {
    drag_value(
        ui,
        "Steps",
        &mut config.step_count,
        MOISTURE_STEP_COUNT_RANGE,
        1.0,
    );
    drag_value(
        ui,
        "Step seconds",
        &mut config.step_seconds,
        MOISTURE_STEP_SECONDS_RANGE,
        3_600.0,
    );
    drag_value(
        ui,
        "Reference capacity",
        &mut config.reference_capacity_kg_per_m2,
        MOISTURE_CAPACITY_RANGE,
        1.0,
    );
    drag_value(
        ui,
        "Reference temperature",
        &mut config.reference_temperature_kelvin,
        REFERENCE_TEMPERATURE_KELVIN_RANGE,
        1.0,
    );
    drag_value(
        ui,
        "Capacity temperature response",
        &mut config.capacity_temperature_sensitivity_per_kelvin,
        TEMPERATURE_SENSITIVITY_RANGE,
        0.001,
    );
    drag_value(
        ui,
        "Minimum capacity",
        &mut config.minimum_capacity_kg_per_m2,
        MOISTURE_CAPACITY_RANGE,
        0.1,
    );
    drag_value(
        ui,
        "Maximum capacity",
        &mut config.maximum_capacity_kg_per_m2,
        MOISTURE_CAPACITY_RANGE,
        1.0,
    );
    drag_value(
        ui,
        "Ocean evaporation rate",
        &mut config.ocean_evaporation_rate_per_second,
        MOISTURE_RATE_RANGE,
        1.0e-7,
    );
    drag_value(
        ui,
        "Rainfall rate",
        &mut config.rainfall_rate_per_second,
        MOISTURE_RATE_RANGE,
        1.0e-7,
    );
    drag_value(
        ui,
        "Orographic coefficient",
        &mut config.orographic_coefficient_per_meter,
        OROGRAPHIC_COEFFICIENT_RANGE,
        1.0e-5,
    );
    slider(
        ui,
        "Maximum orographic fraction",
        &mut config.maximum_orographic_fraction_per_step,
        TRANSPORT_FRACTION_RANGE,
    );
    slider(
        ui,
        "Maximum transport fraction",
        &mut config.maximum_transport_fraction_per_step,
        TRANSPORT_FRACTION_RANGE,
    );
}

fn cryosphere_controls(ui: &mut egui::Ui, config: &mut CryosphereConfig) {
    drag_value(
        ui,
        "Maximum refinements",
        &mut config.maximum_iterations,
        CRYOSPHERE_ITERATION_LIMIT_RANGE,
        1.0,
    );
    drag_value(
        ui,
        "Closure tolerance",
        &mut config.closure_tolerance,
        CRYOSPHERE_CLOSURE_TOLERANCE_RANGE,
        1.0e-6,
    );
    drag_value(
        ui,
        "Snowfall temperature",
        &mut config.snowfall_temperature_kelvin,
        CRYOSPHERE_TEMPERATURE_RANGE,
        0.1,
    );
    drag_value(
        ui,
        "Melt temperature",
        &mut config.melt_temperature_kelvin,
        CRYOSPHERE_TEMPERATURE_RANGE,
        0.1,
    );
    drag_value(
        ui,
        "Full snow cover",
        &mut config.full_snow_cover_kg_per_m2,
        CRYOSPHERE_MASS_RANGE,
        1.0,
    );
    drag_value(
        ui,
        "Seasonal snow capacity",
        &mut config.seasonal_snow_capacity_kg_per_m2,
        CRYOSPHERE_MASS_RANGE,
        5.0,
    );
    drag_value(
        ui,
        "Snow melt factor",
        &mut config.snow_melt_kg_per_m2_per_kelvin_day,
        CRYOSPHERE_RATE_RANGE,
        0.1,
    );
    drag_value(
        ui,
        "Land-ice melt factor",
        &mut config.land_ice_melt_kg_per_m2_per_kelvin_day,
        CRYOSPHERE_RATE_RANGE,
        0.1,
    );
    drag_value(
        ui,
        "Sea-ice growth factor",
        &mut config.sea_ice_growth_fraction_per_kelvin_day,
        CRYOSPHERE_FRACTION_RATE_RANGE,
        0.001,
    );
    drag_value(
        ui,
        "Sea-ice melt factor",
        &mut config.sea_ice_melt_fraction_per_kelvin_day,
        CRYOSPHERE_FRACTION_RATE_RANGE,
        0.001,
    );
}
