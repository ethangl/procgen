//! Deterministic climate stages over the authoritative spherical mesh.
//!
//! Solar forcing, radiative-equilibrium temperature, local seasonal thermal
//! response, coarse atmospheric circulation, moisture transport, and the
//! cryosphere are pure stages. A separate bounded orchestrator couples only
//! their explicit per-cell surface albedo feedback and retains no generation
//! state between calls.

use std::ops::RangeInclusive;

pub(crate) const SECONDS_PER_DAY: f64 = 86_400.0;

pub(crate) fn validate_range<T: PartialOrd, E>(
    value: T,
    range: &RangeInclusive<T>,
    error: E,
) -> Result<(), E> {
    if range.contains(&value) {
        Ok(())
    } else {
        Err(error)
    }
}

mod circulation;
mod coupling;
mod cryosphere;
mod field;
mod moisture;
mod orbit;
mod radiative_equilibrium;
mod seasonal_thermal;
mod solar_forcing;

pub use circulation::{
    AtmosphericCirculation, AtmosphericCirculationConfig, AtmosphericCirculationDiagnostics,
    AtmosphericCirculationError, AtmosphericCirculationInputs, CALM_WIND_SPEED_METERS_PER_SECOND,
    DRAG_RATE_RANGE, MAXIMUM_WIND_SPEED_RANGE, TERRAIN_STEERING_RANGE,
    derive_atmospheric_circulation,
};
pub use coupling::{
    CLIMATE_COUPLING_FRACTION_TOLERANCE_RANGE, CLIMATE_COUPLING_ITERATION_LIMIT_RANGE,
    CLIMATE_COUPLING_TOLERANCE_RANGE, ClimateAlbedoConfig, ClimateCoupling, ClimateCouplingConfig,
    ClimateCouplingDiagnostics, ClimateCouplingError, ClimateCouplingInputs,
    derive_coupled_climate,
};
pub use cryosphere::{
    CRYOSPHERE_CLOSURE_TOLERANCE_RANGE, CRYOSPHERE_FRACTION_RATE_RANGE,
    CRYOSPHERE_ITERATION_LIMIT_RANGE, CRYOSPHERE_MASS_RANGE, CRYOSPHERE_RATE_RANGE,
    CRYOSPHERE_TEMPERATURE_RANGE, Cryosphere, CryosphereConfig, CryosphereDiagnostics,
    CryosphereError, CryosphereInputs, derive_cryosphere,
};
pub use field::{AreaWeightedSummary, Surface};
pub use moisture::{
    MOISTURE_CAPACITY_RANGE, MOISTURE_RATE_RANGE, MOISTURE_STEP_COUNT_RANGE,
    MOISTURE_STEP_SECONDS_RANGE, MoistureTransport, MoistureTransportConfig,
    MoistureTransportDiagnostics, MoistureTransportError, MoistureTransportInputs,
    OROGRAPHIC_COEFFICIENT_RANGE, REFERENCE_TEMPERATURE_KELVIN_RANGE,
    TEMPERATURE_SENSITIVITY_RANGE, TRANSPORT_FRACTION_RANGE, derive_moisture_transport,
};
pub use radiative_equilibrium::{
    RadiativeEquilibriumConfig, RadiativeEquilibriumDiagnostics, RadiativeEquilibriumError,
    RadiativeEquilibriumTemperature, STEFAN_BOLTZMANN_CONSTANT,
    derive_radiative_equilibrium_temperature,
};
pub use seasonal_thermal::{
    ORBITAL_PERIOD_DAYS_RANGE, SeasonalThermalConfig, SeasonalThermalDiagnostics,
    SeasonalThermalError, SeasonalThermalInputs, SeasonalThermalResponse,
    SurfaceThermalDiagnostics, THERMAL_CAPACITY_RANGE, derive_seasonal_thermal_response,
};
pub use solar_forcing::{
    ANNUAL_SAMPLE_RANGE, SolarForcing, SolarForcingConfig, SolarForcingDiagnostics,
    SolarForcingError, derive_solar_forcing,
};
