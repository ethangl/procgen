use super::{GenerationTimings, GeologyWorld, TectonicsWorld};
use procgen_climate::{
    AtmosphericCirculation, ClimateCoupling, ClimateCouplingConfig, ClimateCouplingDiagnostics,
    ClimateCouplingInputs, Cryosphere, MoistureTransport, RadiativeEquilibriumTemperature,
    SeasonalThermalResponse, SolarForcing, SolarForcingConfig, derive_coupled_climate,
    derive_solar_forcing,
};
use procgen_planet::Planet;
use std::error::Error;

/// Settings for the climate phase.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimateSettings {
    /// Earth-like defaults are explicit and caller-editable.
    pub planet: Planet,
    pub solar_forcing: SolarForcingConfig,
    pub coupling: ClimateCouplingConfig,
}

impl Default for ClimateSettings {
    fn default() -> Self {
        Self {
            planet: Planet::EARTH,
            solar_forcing: SolarForcingConfig::default(),
            coupling: ClimateCouplingConfig::EARTHLIKE,
        }
    }
}

/// Results of the climate phase.
pub struct ClimateWorld {
    pub solar_forcing: SolarForcing,
    pub radiative_equilibrium: RadiativeEquilibriumTemperature,
    pub seasonal_thermal: SeasonalThermalResponse,
    pub atmospheric_circulation: AtmosphericCirculation,
    pub moisture_transport: MoistureTransport,
    pub cryosphere: Cryosphere,
    pub cell_albedo: Vec<f32>,
    pub coupling_diagnostics: ClimateCouplingDiagnostics,
    pub timings: GenerationTimings,
    pub config: ClimateSettings,
}

impl ClimateWorld {
    pub fn generate(
        tectonics: &TectonicsWorld,
        geology: &GeologyWorld,
        config: ClimateSettings,
    ) -> Result<Self, Box<dyn Error>> {
        let mesh = &tectonics.voronoi;
        let mut timings = GenerationTimings::default();
        let solar_forcing = timings.record("Solar forcing", || {
            derive_solar_forcing(mesh, config.planet, config.solar_forcing)
        })?;
        let climate = timings.record("Coupled climate", || {
            derive_coupled_climate(
                mesh,
                ClimateCouplingInputs {
                    planet: config.planet,
                    solar_forcing: &solar_forcing,
                    solar_forcing_config: config.solar_forcing,
                    final_elevation: geology.isostasy.field(),
                },
                config.coupling,
            )
        })?;
        let ClimateCoupling {
            cell_albedo,
            radiative_equilibrium,
            seasonal_thermal,
            atmospheric_circulation,
            moisture_transport,
            cryosphere,
            diagnostics: coupling_diagnostics,
        } = climate;

        Ok(Self {
            solar_forcing,
            radiative_equilibrium,
            seasonal_thermal,
            atmospheric_circulation,
            moisture_transport,
            cryosphere,
            cell_albedo,
            coupling_diagnostics,
            timings,
            config,
        })
    }

    pub fn validate(&self, tectonics: &TectonicsWorld) -> Result<(), Box<dyn Error>> {
        let mesh = &tectonics.voronoi;
        if self.config.solar_forcing.annual_sample_count
            != self.seasonal_thermal.annual_sample_count
            || self.cell_albedo.len() != mesh.cell_count()
        {
            return Err("climate results do not match their settings".into());
        }

        self.solar_forcing.validate(mesh)?;
        self.radiative_equilibrium.validate(mesh)?;
        self.seasonal_thermal.validate(mesh)?;
        self.atmospheric_circulation.validate(mesh)?;
        self.moisture_transport.validate(mesh)?;
        self.cryosphere.validate(mesh)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{geology_settings, tectonics_settings, tectonics_world};

    #[test]
    fn climate_runs_on_the_upstream_phases_alone() {
        let tectonics = tectonics_world(tectonics_settings(128, 13));
        let geology = GeologyWorld::generate(&tectonics, geology_settings(128, 13)).unwrap();
        let climate =
            ClimateWorld::generate(&tectonics, &geology, ClimateSettings::default()).unwrap();

        climate.validate(&tectonics).unwrap();
        assert!(
            climate
                .atmospheric_circulation
                .diagnostics
                .wind_speed_meters_per_second
                .maximum
                > 0.0
        );
        assert!(
            climate
                .atmospheric_circulation
                .diagnostics
                .speed_capped_cell_count
                < tectonics.voronoi.cell_count()
        );
        assert!(
            climate.coupling_diagnostics.iterations <= climate.config.coupling.maximum_iterations
        );
        assert!(
            climate
                .moisture_transport
                .diagnostics
                .mass_balance_error_kg_per_m2
                .abs()
                <= 1.0e-10
        );
    }
}
