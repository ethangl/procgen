//! Deterministic climate stages over the authoritative spherical mesh.
//!
//! Solar forcing, radiative-equilibrium temperature, and local seasonal thermal
//! response are independent pure stages. None contains lateral transport,
//! atmospheric physics, or coupled feedbacks.

mod field;
mod radiative_equilibrium;
mod seasonal_thermal;
mod solar_forcing;

pub use field::AreaWeightedSummary;
pub use radiative_equilibrium::{
    RadiativeEquilibriumConfig, RadiativeEquilibriumDiagnostics, RadiativeEquilibriumError,
    RadiativeEquilibriumTemperature, STEFAN_BOLTZMANN_CONSTANT,
    derive_radiative_equilibrium_temperature,
};
pub use seasonal_thermal::{
    ORBITAL_PERIOD_DAYS_RANGE, SeasonalThermalConfig, SeasonalThermalDiagnostics,
    SeasonalThermalError, SeasonalThermalInputs, SeasonalThermalResponse, THERMAL_CAPACITY_RANGE,
    THERMAL_SAMPLE_RANGE, derive_seasonal_thermal_response,
};
pub use solar_forcing::{
    ANNUAL_SAMPLE_RANGE, SolarForcing, SolarForcingConfig, SolarForcingDiagnostics,
    SolarForcingError, derive_solar_forcing,
};
