//! Deterministic climate stages over the authoritative spherical mesh.
//!
//! Solar forcing, radiative-equilibrium temperature, local seasonal thermal
//! response, and coarse atmospheric circulation are independent pure stages.
//! They contain no coupled feedbacks.

mod circulation;
mod field;
mod orbit;
mod radiative_equilibrium;
mod seasonal_thermal;
mod solar_forcing;

pub use circulation::{
    AtmosphericCirculation, AtmosphericCirculationConfig, AtmosphericCirculationDiagnostics,
    AtmosphericCirculationError, AtmosphericCirculationInputs, DRAG_RATE_RANGE,
    MAXIMUM_WIND_SPEED_RANGE, TERRAIN_STEERING_RANGE, derive_atmospheric_circulation,
};
pub use field::AreaWeightedSummary;
pub use radiative_equilibrium::{
    RadiativeEquilibriumConfig, RadiativeEquilibriumDiagnostics, RadiativeEquilibriumError,
    RadiativeEquilibriumTemperature, STEFAN_BOLTZMANN_CONSTANT,
    derive_radiative_equilibrium_temperature,
};
pub use seasonal_thermal::{
    ORBITAL_PERIOD_DAYS_RANGE, SeasonalThermalConfig, SeasonalThermalDiagnostics,
    SeasonalThermalError, SeasonalThermalInputs, SeasonalThermalResponse, Surface,
    SurfaceThermalDiagnostics, THERMAL_CAPACITY_RANGE, derive_seasonal_thermal_response,
};
pub use solar_forcing::{
    ANNUAL_SAMPLE_RANGE, SolarForcing, SolarForcingConfig, SolarForcingDiagnostics,
    SolarForcingError, derive_solar_forcing,
};
