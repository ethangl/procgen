//! The shapes a boundary raises, and the one configuration that holds them.
//!
//! A profile is a centre offset at the boundary cell and a decay to zero a
//! bounded number of mesh hops behind it. [`BoundaryEffect`] states one as a
//! centre and a depth, [`ContinentalRiftProfile`] states one as a centre and a
//! flank because a rift valley's shoulders stand above its floor, and both
//! convert into the [`PropagationProfile`] that `deformation` propagates.
//! Nothing here reads the mesh: these are the numbers, and `deformation` is
//! what does them to a world.

use crate::field::DEFAULT_STEP_DURATION;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryEffect {
    /// Signed deformation at the boundary cell.
    pub offset: f32,
    /// Mesh hops the effect propagates within the current owning plate.
    pub depth: usize,
}

/// Graben and shoulders of a continental divergent boundary. A rift valley
/// sits below flanks that stand above the plateau behind them: the East
/// African floor lies about a kilometre under shoulders one to two kilometres
/// over their plateau. The flank may therefore be either sign, as long as it
/// is above the centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContinentalRiftProfile {
    /// Subsidence at a continental divergent boundary cell.
    pub center_offset: f32,
    /// Offset one mesh hop away from the boundary. Positive raises rift
    /// shoulders over the plateau; negative widens the depression.
    pub flank_offset: f32,
    /// Mesh hop at which the flank reaches zero within the owning plate.
    pub decay_depth: usize,
}

impl ContinentalRiftProfile {
    pub const MIN_DECAY_DEPTH: usize = 2;

    pub fn is_valid(&self) -> bool {
        self.center_offset.is_finite()
            && self.flank_offset.is_finite()
            && self.center_offset < 0.0
            && self.flank_offset > self.center_offset
            && self.decay_depth >= Self::MIN_DECAY_DEPTH
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryDeformationConfig {
    pub convergent: BoundaryEffect,
    /// Continental side of a divergent boundary.
    pub rift: ContinentalRiftProfile,
    /// Profile a transform boundary raises per unit of *residual convergence*,
    /// rather than per unit of shear: pure lateral slip builds no relief, and
    /// what a transform makes comes from the small normal component at a bend,
    /// positive where the bend is transpressive and negative where it pulls
    /// apart. The default depth is 1 because that relief is narrow — a
    /// restraining bend is a range one cell wide at this resolution, not the
    /// belt a convergent boundary spreads over six.
    pub transform: BoundaryEffect,
    /// Continental side of a mixed-crust convergent boundary.
    pub collision: BoundaryEffect,
    /// Older oceanic side of any convergent boundary that has a polarity: the
    /// floor going down, whether the plate above it is a continent or a
    /// younger floor.
    pub trench: BoundaryEffect,
    /// Younger oceanic side of an ocean-ocean convergent boundary, which is
    /// the overriding plate.
    ///
    /// An arc is narrow — a volcanic front 100 to 200 km behind the trench —
    /// so the default depth is 2 hops rather than the six a collision belt
    /// spreads over. The offset matches `convergent` so that a floor at 0.08
    /// to 0.30 reaches the 0.5 datum once the boundary has held for the whole
    /// of [`Self::full_deformation_time`] and the volcanic uplift lands on
    /// top. That is what makes an arc an island chain rather than a submarine
    /// ridge, and it is the number to retune if arcs stay drowned.
    pub island_arc: BoundaryEffect,
    /// Motion magnitude at which a boundary effect reaches its full offset.
    pub saturation_speed: f32,
    /// Model time over which a saturated boundary raises its full profile
    /// offset. Each step adds the profile scaled by the step's duration over
    /// this time, so the default of nine default steps
    /// (`9 * DEFAULT_STEP_DURATION`) is the time a boundary needs to hold one
    /// regime to reach the magnitudes the removed final-boundary stage
    /// produced. The viewer's run is longer than that, so a boundary that
    /// converges throughout raises more and [`Self::maximum_magnitude`]
    /// catches the few cells that saturate.
    pub full_deformation_time: f32,
    /// Magnitude the accumulated field is clamped to. The default is the
    /// largest offset the default profiles can raise — the collision centre at
    /// 0.5, against 0.4 for the convergent, transform, and island arc centres
    /// and 0.2 for the trench and the rift centre — so it bites only where a
    /// boundary held one regime for longer than
    /// [`Self::full_deformation_time`]: 866 of the 65,536 cells at the
    /// viewer's default sixty-step run, and none at all over fifteen. The
    /// share grows without bound with run length, because uplift is added
    /// every step and nothing takes it away; see "Run length" in
    /// `docs/plate-movement.md`.
    pub maximum_magnitude: f32,
}

impl Default for BoundaryDeformationConfig {
    fn default() -> Self {
        Self {
            convergent: BoundaryEffect {
                offset: 0.4,
                depth: 6,
            },
            rift: ContinentalRiftProfile {
                center_offset: -0.2,
                flank_offset: 0.08,
                decay_depth: 3,
            },
            transform: BoundaryEffect {
                offset: 0.4,
                depth: 1,
            },
            collision: BoundaryEffect {
                offset: 0.5,
                depth: 5,
            },
            trench: BoundaryEffect {
                offset: -0.2,
                depth: 1,
            },
            island_arc: BoundaryEffect {
                offset: 0.4,
                depth: 2,
            },
            saturation_speed: 2.0,
            full_deformation_time: 9.0 * DEFAULT_STEP_DURATION,
            maximum_magnitude: 0.5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryDeformationError {
    InvalidConfig,
    InvalidRiftProfile,
}

impl fmt::Display for BoundaryDeformationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str(
                "deformation offsets must be finite, and saturation speed, full deformation time, and maximum magnitude must be finite and positive",
            ),
            Self::InvalidRiftProfile => write!(
                formatter,
                "rift profile must satisfy center < 0 and center < flank, and decay depth >= {}",
                ContinentalRiftProfile::MIN_DECAY_DEPTH
            ),
        }
    }
}

impl std::error::Error for BoundaryDeformationError {}

pub(crate) fn validate_config(
    config: BoundaryDeformationConfig,
) -> Result<(), BoundaryDeformationError> {
    if !config.rift.is_valid() {
        return Err(BoundaryDeformationError::InvalidRiftProfile);
    }
    let effects = [
        config.convergent,
        config.transform,
        config.collision,
        config.trench,
        config.island_arc,
    ];
    let positives = [
        config.saturation_speed,
        config.full_deformation_time,
        config.maximum_magnitude,
    ];
    if effects.iter().any(|effect| !effect.offset.is_finite())
        || positives
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(BoundaryDeformationError::InvalidConfig);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PropagationProfile {
    center_offset: f32,
    flank_offset: f32,
    decay_depth: usize,
}

impl PropagationProfile {
    pub(crate) fn depth(self) -> usize {
        self.decay_depth.saturating_sub(1)
    }

    pub(crate) fn offset_at(self, depth: usize) -> f32 {
        if depth == 0 {
            return self.center_offset;
        }
        if self.decay_depth == 1 {
            return 0.0;
        }
        self.flank_offset * (1.0 - (depth - 1) as f32 / (self.decay_depth - 1) as f32)
    }

    pub(crate) fn scaled(self, scale: f32) -> Self {
        Self {
            center_offset: self.center_offset * scale,
            flank_offset: self.flank_offset * scale,
            ..self
        }
    }
}

impl From<ContinentalRiftProfile> for PropagationProfile {
    fn from(profile: ContinentalRiftProfile) -> Self {
        Self {
            center_offset: profile.center_offset,
            flank_offset: profile.flank_offset,
            decay_depth: profile.decay_depth,
        }
    }
}

impl From<BoundaryEffect> for PropagationProfile {
    fn from(effect: BoundaryEffect) -> Self {
        let decay_depth = effect.depth.saturating_add(1);
        Self {
            center_offset: effect.offset,
            flank_offset: effect.offset * effect.depth as f32 / decay_depth as f32,
            decay_depth,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continental_rift_has_a_deep_center_raised_shoulders_and_bounded_decay_to_zero() {
        let source = PropagationProfile::from(ContinentalRiftProfile {
            center_offset: -0.8,
            flank_offset: 0.2,
            decay_depth: 4,
        });

        assert_eq!(source.offset_at(0), -0.8);
        assert_eq!(source.offset_at(1), 0.2);
        assert!((source.offset_at(2) - 0.2 * (2.0 / 3.0)).abs() < f32::EPSILON);
        assert!((source.offset_at(3) - 0.2 * (1.0 / 3.0)).abs() < f32::EPSILON);
        assert_eq!(source.offset_at(4), 0.0);
        assert!(
            (1..=source.depth()).all(|depth| source.offset_at(depth) > 0.0),
            "a shoulder stands above the plateau until it decays to zero"
        );
    }

    #[test]
    fn linear_effect_conversion_preserves_its_profile_and_zero_depth_edge_case() {
        let effect = BoundaryEffect {
            offset: 0.6,
            depth: 3,
        };
        let profile = PropagationProfile::from(effect);
        for depth in 0..=effect.depth {
            let expected = effect.offset * (1.0 - depth as f32 / (effect.depth + 1) as f32);
            assert!((profile.offset_at(depth) - expected).abs() < f32::EPSILON);
        }

        let point = PropagationProfile::from(BoundaryEffect {
            offset: -0.2,
            depth: 0,
        });
        assert_eq!(point.depth(), 0);
        assert_eq!(point.offset_at(0), -0.2);
        assert_eq!(point.offset_at(1), 0.0);
    }

    #[test]
    fn rejects_invalid_configuration() {
        assert_eq!(
            validate_config(BoundaryDeformationConfig::default()),
            Ok(())
        );
        for config in [
            BoundaryDeformationConfig {
                saturation_speed: 0.0,
                ..Default::default()
            },
            BoundaryDeformationConfig {
                full_deformation_time: 0.0,
                ..Default::default()
            },
            BoundaryDeformationConfig {
                full_deformation_time: f32::NAN,
                ..Default::default()
            },
            BoundaryDeformationConfig {
                maximum_magnitude: -1.0,
                ..Default::default()
            },
            BoundaryDeformationConfig {
                maximum_magnitude: f32::INFINITY,
                ..Default::default()
            },
            BoundaryDeformationConfig {
                convergent: BoundaryEffect {
                    offset: f32::NAN,
                    ..BoundaryDeformationConfig::default().convergent
                },
                ..Default::default()
            },
        ] {
            assert_eq!(
                validate_config(config),
                Err(BoundaryDeformationError::InvalidConfig)
            );
        }
        // A negative flank is the pre-shoulder graben and stays legal.
        assert_eq!(
            validate_config(BoundaryDeformationConfig {
                rift: ContinentalRiftProfile {
                    flank_offset: -0.1,
                    ..BoundaryDeformationConfig::default().rift
                },
                ..Default::default()
            }),
            Ok(())
        );
        for rift in [
            ContinentalRiftProfile {
                center_offset: 0.1,
                ..BoundaryDeformationConfig::default().rift
            },
            ContinentalRiftProfile {
                center_offset: -0.1,
                flank_offset: -0.2,
                ..BoundaryDeformationConfig::default().rift
            },
            ContinentalRiftProfile {
                decay_depth: 1,
                ..BoundaryDeformationConfig::default().rift
            },
        ] {
            assert_eq!(
                validate_config(BoundaryDeformationConfig {
                    rift,
                    ..Default::default()
                }),
                Err(BoundaryDeformationError::InvalidRiftProfile)
            );
        }
    }
}
