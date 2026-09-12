//! The shapes a boundary raises, and the one configuration that holds them.
//!
//! A profile is a centre offset at the boundary cell and a decay to zero a
//! bounded distance behind it. [`BoundaryEffect`] states one as a centre and a
//! depth, [`ContinentalRiftProfile`] states one as a centre and a flank
//! because a rift valley's shoulders stand above its floor, and both resolve
//! onto a mesh as the [`PropagationProfile`] that `deformation` propagates.
//!
//! Every depth here is a model length on the unit sphere, so a belt is as wide
//! on a fine mesh as on a coarse one. The one place a length becomes a hop
//! count is [`BoundaryDeformationConfig::profiles`], which `deformation` calls
//! once per step.

use crate::field::DEFAULT_STEP_DURATION;
use procgen_sphere_mesh::{default_hop_length, hops};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryEffect {
    /// Signed deformation a saturated boundary cell raises over one
    /// [`BoundaryDeformationConfig::full_deformation_time`]. It is a rate
    /// rather than a height: what a belt reaches is set against the sink in
    /// [`BoundaryDeformationConfig::erosion_time`].
    pub offset: f32,

    /// Model length on the unit sphere the effect propagates within the
    /// current owning plate. One default hop is about 88 km at Earth radius.
    pub depth: f32,
}

impl BoundaryEffect {
    /// The hop profile this effect raises on a mesh of `cell_count` cells. A
    /// linear effect reaches its own depth and decays to zero one hop past it.
    fn resolve(self, cell_count: usize) -> PropagationProfile {
        PropagationProfile::linear(self.offset, hops(cell_count, self.depth))
    }
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
    /// Model length on the unit sphere at which the flank reaches zero within
    /// the owning plate.
    pub decay_depth: f32,
}

impl ContinentalRiftProfile {
    /// Shortest decay a rift can have: on the default mesh the centre takes
    /// the first hop and the flank needs one of its own, so anything less
    /// would be a valley with no shoulder. A mesh too coarse to resolve two
    /// hops of it still draws the centre.
    pub fn minimum_decay_depth() -> f32 {
        2.0 * default_hop_length()
    }

    pub fn is_valid(&self) -> bool {
        self.center_offset.is_finite()
            && self.flank_offset.is_finite()
            && self.center_offset < 0.0
            && self.flank_offset > self.center_offset
            && self.decay_depth >= Self::minimum_decay_depth()
    }

    fn resolve(self, cell_count: usize) -> PropagationProfile {
        PropagationProfile::rift(
            self.center_offset,
            self.flank_offset,
            hops(cell_count, self.decay_depth),
        )
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
    /// apart. The default depth is one default hop because that relief is
    /// narrow — a restraining bend is a range some 90 km wide, not the belt a
    /// convergent boundary spreads over six times that.
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
    /// so the default depth is two default hops, about 180 km, rather than the
    /// six a collision belt spreads over. The offset matches `convergent`, and
    /// at that rate an arc that keeps converging carries a floor at 0.08 to
    /// 0.30 past the 0.5 datum with the volcanic uplift on top. That is what
    /// makes an arc an island chain rather than a submarine ridge, and it is
    /// the number to retune if arcs stay drowned.
    pub island_arc: BoundaryEffect,
    /// Motion magnitude at which a boundary effect reaches its full offset.
    pub saturation_speed: f32,
    /// Model time over which a saturated boundary raises one full profile
    /// offset. An offset is therefore a rate: a saturated boundary adds
    /// `offset * step_duration / full_deformation_time` of relief a step, and
    /// the default of nine default steps (`9 * DEFAULT_STEP_DURATION`) is the
    /// time a boundary needs to hold one regime to raise the whole of its
    /// offset once.
    ///
    /// It is not what a belt reaches. [`Self::erosion_time`] takes relief away
    /// in proportion to how much a parcel carries, so a boundary that holds
    /// its regime rises toward
    /// `offset * erosion_time / full_deformation_time`, about 3.3 offsets at
    /// the defaults, and [`Self::maximum_magnitude`] catches it there.
    pub full_deformation_time: f32,
    /// Magnitude the carried field is clamped to. The default is the largest
    /// offset the default profiles can raise — the collision centre at 0.5,
    /// against 0.4 for the convergent, transform, and island arc centres and
    /// 0.2 for the trench and the rift centre — and with erosion under it the
    /// clamp is the steady state of an active belt rather than an accumulator
    /// overflowing: uplift and decay would otherwise balance at about 1.33,
    /// which is above it, so a boundary that keeps converging reaches the
    /// clamp and holds there while a belt whose boundary moves on decays away
    /// from it. See "Relief decay" in `docs/plate-movement.md`.
    pub maximum_magnitude: f32,
    /// Model time constant of the relief sink. Every step, what a parcel of
    /// crust carries is multiplied by `1 - step_duration / erosion_time`
    /// before that step's boundaries add to it, so relief with no boundary
    /// under it decays toward zero and relief under a boundary rises to where
    /// uplift and decay balance.
    ///
    /// It is denudation at the scale of a whole orogen, which Ahnert's
    /// relation makes proportional to mean relief, with the isostatic rebound
    /// of the crust under the stripped rock folded in: rock leaves about six
    /// times faster than surface elevation falls, so the e-folding of the
    /// surface is 40 to 50 Myr rather than Ahnert's 7. The default of thirty
    /// default steps is 0.42 model time, about 45 Myr at the reading in
    /// `docs/plate-movement.md`, which is what leaves the Appalachians low but
    /// standing after 300 Myr.
    ///
    /// Infinity disables the sink, which is the world before it existed.
    pub erosion_time: f32,
}

impl Default for BoundaryDeformationConfig {
    fn default() -> Self {
        Self {
            convergent: BoundaryEffect {
                // The fold-and-thrust front at the suture, and no longer the
                // plateau behind it: crustal thickness floats that out of the
                // material the collision buried, so what this paints would be
                // counted twice. Halved and narrowed from 0.4 over six hops
                // when that landed.
                offset: 0.2,
                depth: 3.0 * default_hop_length(),
            },

            rift: ContinentalRiftProfile {
                center_offset: -0.2,
                flank_offset: 0.08,
                decay_depth: 3.0 * default_hop_length(),
            },
            transform: BoundaryEffect {
                offset: 0.4,
                depth: default_hop_length(),
            },
            collision: BoundaryEffect {
                offset: 0.5,
                depth: 5.0 * default_hop_length(),
            },
            trench: BoundaryEffect {
                offset: -0.2,
                depth: default_hop_length(),
            },
            island_arc: BoundaryEffect {
                offset: 0.4,
                depth: 2.0 * default_hop_length(),
            },
            saturation_speed: 2.0,
            full_deformation_time: 9.0 * DEFAULT_STEP_DURATION,
            maximum_magnitude: 0.5,
            erosion_time: 30.0 * DEFAULT_STEP_DURATION,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryDeformationError {
    InvalidConfig,
    InvalidRiftProfile,
    InvalidErosionTime,
}

impl fmt::Display for BoundaryDeformationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str(
                "deformation offsets must be finite, depths must be finite and non-negative, and saturation speed, full deformation time, and maximum magnitude must be finite and positive",
            ),
            Self::InvalidRiftProfile => write!(
                formatter,
                "rift profile must satisfy center < 0 and center < flank, and decay depth >= {}",
                ContinentalRiftProfile::minimum_decay_depth()
            ),
            Self::InvalidErosionTime => formatter
                .write_str("erosion time must be positive; infinity disables relief decay"),
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
    // Infinity is the one value here that is not finite and is still valid: it
    // is how a caller turns the sink off, exactly as an infinite `suture_time`
    // turns suturing off.
    if config.erosion_time.is_nan() || config.erosion_time <= 0.0 {
        return Err(BoundaryDeformationError::InvalidErosionTime);
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

    if effects
        .iter()
        .any(|effect| !effect.offset.is_finite() || !effect.depth.is_finite() || effect.depth < 0.0)
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
    /// A linear effect that holds `offset` at the boundary cell and decays to
    /// zero one hop past `depth`.
    pub(crate) fn linear(offset: f32, depth: usize) -> Self {
        let decay_depth = depth.saturating_add(1);
        Self {
            center_offset: offset,
            flank_offset: offset * depth as f32 / decay_depth as f32,
            decay_depth,
        }
    }

    /// A rift's graben and shoulders, the flank falling to zero at
    /// `decay_depth` hops.
    pub(crate) fn rift(center_offset: f32, flank_offset: f32, decay_depth: usize) -> Self {
        Self {
            center_offset,
            flank_offset,
            decay_depth,
        }
    }

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

/// The six profiles a [`BoundaryDeformationConfig`] raises, resolved onto one
/// mesh. Building them once a step is what keeps the length-to-hop conversion
/// off the per-edge path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PropagationProfiles {
    pub(crate) convergent: PropagationProfile,
    pub(crate) rift: PropagationProfile,
    pub(crate) transform: PropagationProfile,
    pub(crate) collision: PropagationProfile,
    pub(crate) trench: PropagationProfile,
    pub(crate) island_arc: PropagationProfile,
}

impl BoundaryDeformationConfig {
    /// Resolves every configured depth onto a mesh of `cell_count` cells. This
    /// is the one place a model length becomes a hop count.
    pub(crate) fn profiles(&self, cell_count: usize) -> PropagationProfiles {
        PropagationProfiles {
            convergent: self.convergent.resolve(cell_count),
            rift: self.rift.resolve(cell_count),
            transform: self.transform.resolve(cell_count),
            collision: self.collision.resolve(cell_count),
            trench: self.trench.resolve(cell_count),
            island_arc: self.island_arc.resolve(cell_count),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_sphere_mesh::DEFAULT_CELL_COUNT;

    #[test]
    fn continental_rift_has_a_deep_center_raised_shoulders_and_bounded_decay_to_zero() {
        let source = PropagationProfile::rift(-0.8, 0.2, 4);

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
        let (offset, hop_depth) = (0.6, 3);
        let profile = PropagationProfile::linear(offset, hop_depth);
        for depth in 0..=hop_depth {
            let expected = offset * (1.0 - depth as f32 / (hop_depth + 1) as f32);
            assert!((profile.offset_at(depth) - expected).abs() < f32::EPSILON);
        }

        let point = PropagationProfile::linear(-0.2, 0);
        assert_eq!(point.depth(), 0);
        assert_eq!(point.offset_at(0), -0.2);
        assert_eq!(point.offset_at(1), 0.0);
    }

    /// The one invariant the whole slice rests on: a default written as a
    /// multiple of the default hop length resolves back to the hop count it
    /// replaced, so the default mesh deforms exactly as it did.
    #[test]
    fn default_depths_resolve_to_the_hop_counts_they_replaced() {
        let config = BoundaryDeformationConfig::default();
        for (effect, depth) in [
            (config.convergent, 3),
            (config.transform, 1),
            (config.collision, 5),
            (config.trench, 1),
            (config.island_arc, 2),
        ] {
            assert_eq!(hops(DEFAULT_CELL_COUNT, effect.depth), depth);
        }
        assert_eq!(hops(DEFAULT_CELL_COUNT, config.rift.decay_depth), 3);
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
            BoundaryDeformationConfig {
                collision: BoundaryEffect {
                    depth: -1.0,
                    ..BoundaryDeformationConfig::default().collision
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
                decay_depth: default_hop_length(),
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
