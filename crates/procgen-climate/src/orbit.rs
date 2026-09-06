use procgen_planet::Planet;
use std::f64::consts::PI;

#[derive(Clone, Copy)]
pub(crate) struct OrbitalState {
    pub distance_meters: f64,
    pub stellar_flux: f64,
    pub declination: f64,
    declination_sine: f64,
    declination_cosine: f64,
}

pub(crate) struct OrbitalSampler {
    midpoint_states: Vec<OrbitalState>,
}

impl OrbitalSampler {
    pub fn new(planet: Planet, sample_count: usize) -> Self {
        Self {
            midpoint_states: (0..sample_count)
                .map(|sample| orbital_state(planet, (sample as f64 + 0.5) / sample_count as f64))
                .collect(),
        }
    }

    pub fn midpoint_states(&self) -> &[OrbitalState] {
        &self.midpoint_states
    }
}

/// Returns the uniform annual interval containing `phase`.
pub(crate) fn selected_sample_index(phase: f64, sample_count: usize) -> usize {
    debug_assert!(sample_count > 0);
    ((phase.rem_euclid(1.0) * sample_count as f64).floor() as usize) % sample_count
}

pub(crate) fn daily_mean_at(latitude_sine: f64, state: OrbitalState) -> DailyMeanInsolation {
    let latitude_sine = latitude_sine.clamp(-1.0, 1.0);
    let latitude_cosine = (1.0 - latitude_sine * latitude_sine).sqrt();
    let meridional = latitude_sine * state.declination_sine;
    let diurnal = latitude_cosine * state.declination_cosine;
    // Infinite when the sun never crosses the horizon and NaN only at a pole
    // on the equinox.
    let sunset_argument = -meridional / diurnal;
    let (sunset_hour_angle, daylight) = if sunset_argument >= 1.0 {
        (0.0, Daylight::PolarNight)
    } else if sunset_argument <= -1.0 {
        (PI, Daylight::PolarDay)
    } else if sunset_argument.is_nan() {
        (0.0, Daylight::Cycles)
    } else {
        (sunset_argument.acos(), Daylight::Cycles)
    };
    let insolation = state.stellar_flux / PI
        * (sunset_hour_angle * meridional + diurnal * sunset_hour_angle.sin());
    DailyMeanInsolation {
        watts_per_square_meter: insolation.clamp(0.0, state.stellar_flux),
        daylight,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DailyMeanInsolation {
    pub watts_per_square_meter: f64,
    pub daylight: Daylight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Daylight {
    PolarNight,
    Cycles,
    PolarDay,
}

pub(crate) fn orbital_state(planet: Planet, phase: f64) -> OrbitalState {
    let orbit = planet.orbit;
    let mean_anomaly = phase * 2.0 * PI;
    let eccentric_anomaly = solve_kepler(mean_anomaly, orbit.eccentricity);
    let true_anomaly = 2.0
        * ((1.0 + orbit.eccentricity).sqrt() * (eccentric_anomaly * 0.5).sin())
            .atan2((1.0 - orbit.eccentricity).sqrt() * (eccentric_anomaly * 0.5).cos());
    let distance_meters =
        orbit.semi_major_axis_meters * (1.0 - orbit.eccentricity * eccentric_anomaly.cos());
    let stellar_longitude = true_anomaly + orbit.stellar_longitude_at_periapsis_radians;
    let declination = (orbit.obliquity_radians.sin() * stellar_longitude.sin()).asin();
    OrbitalState {
        distance_meters,
        stellar_flux: planet.star.luminosity_watts / (4.0 * PI * distance_meters.powi(2)),
        declination,
        declination_sine: declination.sin(),
        declination_cosine: declination.cos(),
    }
}

fn solve_kepler(mean_anomaly: f64, eccentricity: f64) -> f64 {
    if mean_anomaly == 0.0 || eccentricity == 0.0 {
        return mean_anomaly;
    }
    let mut lower = 0.0;
    let mut upper = 2.0 * PI;
    for _ in 0..64 {
        let midpoint = (lower + upper) * 0.5;
        let residual = midpoint - eccentricity * midpoint.sin() - mean_anomaly;
        if residual > 0.0 {
            upper = midpoint;
        } else {
            lower = midpoint;
        }
    }
    (lower + upper) * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_sample_interval_wraps_with_orbital_phase() {
        assert_eq!(selected_sample_index(0.0, 4), 0);
        assert_eq!(selected_sample_index(0.249, 4), 0);
        assert_eq!(selected_sample_index(0.25, 4), 1);
        assert_eq!(selected_sample_index(1.0, 4), 0);
        assert_eq!(selected_sample_index(-0.01, 4), 3);
        assert_eq!(selected_sample_index(-f64::MIN_POSITIVE, 4), 0);
    }
}
