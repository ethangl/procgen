//! What a caller states about an evolution run, and what follows from it.
//!
//! A run is a duration. The step count is the run over the step, so a mesh
//! that needs a shorter step covers the same history in more of them rather
//! than covering less of it. Everything here can be checked before a mesh is
//! in hand; the one rule that cannot, the bound a step may not outrun, lives
//! with [`evolution`] because it reads the mesh.
//!
//! [`evolution`]: crate::evolution

use crate::{
    BoundaryDeformationConfig, MAX_GAP_RADIUS, MaterialTransportConfig, PlateEvolutionError,
    PlateLifecycleConfig, PoleDriftConfig, boundary_profiles, field::DEFAULT_STEP_DURATION,
    lifecycle,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateEvolutionConfig {
    /// Seed for everything a run hashes for itself, which today is the drift
    /// of the plate poles. It is evolution's own seed rather than the
    /// kinematics seed so that re-rolling the drift does not re-roll the
    /// motion it starts from.
    pub seed: u64,
    /// Model time the whole run covers.
    ///
    /// It is the run, and the step count follows from it: a finer mesh needs
    /// a shorter step to keep transport honest, and deriving the count means
    /// that shorter step slices the same span of the world's history more
    /// finely rather than covering less of it. A run stated as a step count
    /// did the opposite, and a mesh twice as fine ran half the history.
    pub run_duration: f32,
    /// Model time advanced per step. Every displacement is a speed times this
    /// duration measured against the mesh's cell width, so a finer mesh has a
    /// smaller cell width and moves more cells per step for the same motion,
    /// which is what a fixed model time per step should do. Zero freezes the
    /// world. See [`DEFAULT_STEP_DURATION`] for where the default sits.
    ///
    /// Bounded above by [`maximum_step_duration`]: past it a step carries
    /// material further than transport can see, so material jumps trenches
    /// without subducting and deformation is painted at boundary positions the
    /// plates have already left. That is not a coarser version of the same
    /// run, so evolution rejects it rather than running it.
    pub step_duration: f32,
    pub transport: MaterialTransportConfig,
    /// Profiles the boundaries current in each step raise into the carried
    /// deformation field. It sits here rather than beside evolution because
    /// deformation is a substage of a step exactly as transport is: a config
    /// evolution reads, not a result it is handed.
    pub deformation: BoundaryDeformationConfig,
    /// How far each plate's rotation vector moves at the end of a step.
    pub pole_drift: PoleDriftConfig,
    /// How the plate set itself changes: rifting of large continental plates
    /// and suturing of the continental pairs that have collided long enough.
    pub lifecycle: PlateLifecycleConfig,
}

impl PlateEvolutionConfig {
    /// Complete boundary-classification, deformation, transport, and
    /// pole-drift transitions the run takes: its duration over one step,
    /// rounded to the nearest whole step.
    ///
    /// A step duration of zero freezes the world and takes no steps at all,
    /// which is the same world a run of zero duration leaves.
    pub fn step_count(&self) -> usize {
        if self.step_duration <= 0.0
            || !self.step_duration.is_finite()
            || !self.run_duration.is_finite()
        {
            return 0;
        }
        (self.run_duration / self.step_duration).round().max(0.0) as usize
    }

    /// This config over a run of exactly `step_count` steps of
    /// `step_duration`.
    ///
    /// The fixtures pin what a given number of steps does, and a run stated as
    /// that product lands on that number exactly. Nothing outside a test
    /// should need it: a run is a duration.
    pub fn with_steps(self, step_count: usize, step_duration: f32) -> Self {
        Self {
            run_duration: step_count as f32 * step_duration,
            step_duration,
            ..self
        }
    }
}

impl Default for PlateEvolutionConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            run_duration: 5.0 * DEFAULT_STEP_DURATION,
            step_duration: DEFAULT_STEP_DURATION,
            transport: MaterialTransportConfig::default(),
            deformation: BoundaryDeformationConfig::default(),
            pole_drift: PoleDriftConfig::default(),
            lifecycle: PlateLifecycleConfig::default(),
        }
    }
}

/// Everything the config can be held to on its own.
pub(crate) fn validate(config: &PlateEvolutionConfig) -> Result<(), PlateEvolutionError> {
    if !config.step_duration.is_finite()
        || config.step_duration < 0.0
        || !config.run_duration.is_finite()
        || config.run_duration < 0.0
    {
        return Err(PlateEvolutionError::InvalidStepDuration);
    }
    if !(0.0..=MAX_GAP_RADIUS).contains(&config.transport.gap_radius) {
        return Err(PlateEvolutionError::InvalidGapRadius);
    }
    let drift = config.pole_drift;
    if [drift.axis_drift_rate, drift.speed_drift_rate]
        .iter()
        .any(|rate| !rate.is_finite() || *rate < 0.0)
    {
        return Err(PlateEvolutionError::InvalidPoleDriftRate);
    }
    if !drift.speed_drift_limit.is_finite() || !(0.0..=1.0).contains(&drift.speed_drift_limit) {
        return Err(PlateEvolutionError::InvalidSpeedDriftLimit);
    }
    // Infinity is the value that turns the reversion off, so it passes here
    // and the rule that the step must be shorter reads as satisfied by it.
    if drift.reversion_time.is_nan() || drift.reversion_time <= 0.0 {
        return Err(PlateEvolutionError::InvalidReversionTime);
    }
    boundary_profiles::validate_config(config.deformation)?;
    lifecycle::validate_config(config.lifecycle)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The run is the fact a config states and the count follows from it, so
    /// slicing a run more finely covers the same history rather than less of
    /// it. That is what a step count could not do.
    #[test]
    fn the_step_count_is_the_run_over_the_step() {
        let config = PlateEvolutionConfig::default();
        assert_eq!(config.step_count(), 5);
        assert_eq!(
            PlateEvolutionConfig {
                step_duration: config.step_duration / 2.0,
                ..config
            }
            .step_count(),
            10
        );
        // A frozen world takes no steps at all, which is the world a run of
        // no duration leaves too.
        for frozen in [
            PlateEvolutionConfig {
                step_duration: 0.0,
                ..config
            },
            PlateEvolutionConfig {
                run_duration: 0.0,
                ..config
            },
        ] {
            assert_eq!(frozen.step_count(), 0);
        }
    }
}
