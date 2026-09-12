//! What a caller states about a moisture run, and what a mesh makes of it.
//!
//! The config holds the days of weather to simulate; the mesh decides how many
//! steps that takes and how long each stands for. Nothing here reads a field,
//! so [`moisture`] can be about the solver alone.
//!
//! [`moisture`]: crate::moisture

use crate::SECONDS_PER_DAY;
use procgen_sphere_mesh::{default_hop_length, mean_cell_width};
use std::ops::RangeInclusive;

/// A day to a decade. It is the one thing a caller states about a run's
/// length; the schedule the mesh makes of it is derived and is not bounded
/// separately, because nobody configures it.
pub const MOISTURE_SIMULATED_DAYS_RANGE: RangeInclusive<f64> = 1.0..=3_650.0;

/// Steps the default mesh takes. Transport is advective, so a step may not
/// carry moisture further than a cell: a mesh with cells half as wide needs
/// steps half as long, and twice as many of them to cover the same weather.
/// The count therefore goes as the reciprocal of the cell width, and this is
/// what it was measured at on the default mesh.
const DEFAULT_MESH_STEP_COUNT: f64 = 120.0;
/// How long a mesh's transport runs for, and in how many pieces.
///
/// The pieces are the mesh's business and the length is the config's, so a
/// finer mesh resolves the same weather rather than less of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoistureSchedule {
    pub step_count: usize,
    pub step_seconds: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoistureTransportConfig {
    /// Days of weather the transport simulates. The step count follows from
    /// the mesh and the seconds a step stands for follow from both, so this
    /// is the whole of what a caller states about the run's length.
    pub simulated_days: f64,
    /// Column water capacity at the reference temperature, in kg/m2.
    pub reference_capacity_kg_per_m2: f64,
    pub reference_temperature_kelvin: f64,
    /// Exponential capacity response per kelvin.
    pub capacity_temperature_sensitivity_per_kelvin: f64,
    pub minimum_capacity_kg_per_m2: f64,
    pub maximum_capacity_kg_per_m2: f64,
    /// Ocean relaxation rate toward local moisture capacity.
    pub ocean_evaporation_rate_per_second: f64,
    /// Background conversion of airborne moisture to rainfall.
    pub rainfall_rate_per_second: f64,
    /// Conversion rate per meter of positive terrain ascent.
    pub orographic_coefficient_per_meter: f64,
    /// Hard bound on the fraction removed orographically in one step.
    ///
    /// It is a fraction per step rather than per second, and so is
    /// `maximum_transport_fraction_per_step`. The two are the open leads for
    /// why moisture reaches the same number of cells inland on every mesh
    /// instead of the same distance; see "Resolution independence" in
    /// `docs/plate-movement.md`. Neither is shown to be the cause and the fix
    /// is a climate slice.
    pub maximum_orographic_fraction_per_step: f64,
    /// CFL-style bound on the humidity exported from a cell in one step. It is
    /// per step, with the same open question as
    /// `maximum_orographic_fraction_per_step`.
    pub maximum_transport_fraction_per_step: f64,
}

impl MoistureTransportConfig {
    /// The schedule a mesh of `cell_count` cells runs this config on.
    ///
    /// The step count scales with the reciprocal of the cell width, because a
    /// step may not carry moisture past a cell; the seconds a step stands for
    /// are then the simulated days shared out over them. On the default mesh
    /// that is the 120 steps of six hours this was measured at.
    pub fn schedule(&self, cell_count: usize) -> MoistureSchedule {
        let width_ratio =
            f64::from(default_hop_length()) / f64::from(mean_cell_width(1.0, cell_count));
        let step_count = (DEFAULT_MESH_STEP_COUNT * width_ratio).round().max(1.0);
        let step_seconds = self.simulated_days * SECONDS_PER_DAY / step_count;
        // A validated run is a positive number of days over at least one step,
        // so the arithmetic cannot produce anything else.
        debug_assert!(step_seconds > 0.0);
        MoistureSchedule {
            step_count: step_count as usize,
            step_seconds,
        }
    }
}

impl MoistureTransportConfig {
    /// Convenient Earth-like choices. The solver contains no planet-specific values.
    pub const EARTHLIKE: Self = Self {
        simulated_days: 30.0,
        reference_capacity_kg_per_m2: 25.0,
        reference_temperature_kelvin: 288.0,
        capacity_temperature_sensitivity_per_kelvin: 0.07,
        minimum_capacity_kg_per_m2: 0.2,
        maximum_capacity_kg_per_m2: 100.0,
        ocean_evaporation_rate_per_second: 1.5e-6,
        rainfall_rate_per_second: 8.0e-7,
        orographic_coefficient_per_meter: 2.0e-4,
        maximum_orographic_fraction_per_step: 0.35,
        maximum_transport_fraction_per_step: 0.5,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_sphere_mesh::DEFAULT_CELL_COUNT;

    /// The run is the days and the mesh decides the slicing, so a finer mesh
    /// resolves the same weather rather than less of it.
    #[test]
    fn the_schedule_is_the_days_over_the_steps_the_mesh_needs() {
        let config = MoistureTransportConfig::EARTHLIKE;
        assert_eq!(
            config.schedule(DEFAULT_CELL_COUNT),
            MoistureSchedule {
                step_count: 120,
                step_seconds: 21_600.0,
            }
        );
        // Four times the cells halves the cell width, so the same month takes
        // twice the steps and each stands for half as long.
        assert_eq!(
            config.schedule(4 * DEFAULT_CELL_COUNT),
            MoistureSchedule {
                step_count: 240,
                step_seconds: 10_800.0,
            }
        );
    }
}
