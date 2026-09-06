//! Deterministic climate stages over the authoritative spherical mesh.
//!
//! Solar forcing and radiative-equilibrium temperature are independent pure
//! stages. They contain no atmosphere, transport, heat capacity, or feedbacks.

mod field;
mod radiative_equilibrium;
mod solar_forcing;

pub use field::AreaWeightedSummary;
pub use radiative_equilibrium::{
    RadiativeEquilibriumConfig, RadiativeEquilibriumDiagnostics, RadiativeEquilibriumError,
    RadiativeEquilibriumTemperature, STEFAN_BOLTZMANN_CONSTANT,
    derive_radiative_equilibrium_temperature,
};
pub use solar_forcing::{
    ANNUAL_SAMPLE_RANGE, SolarForcing, SolarForcingConfig, SolarForcingDiagnostics,
    SolarForcingError, derive_solar_forcing,
};
