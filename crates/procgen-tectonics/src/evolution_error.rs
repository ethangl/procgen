//! What can stop an evolution run, stated once.
//!
//! A run reads four configs beside its own — transport, deformation, pole
//! drift, and the plate lifecycle — and three stage outputs, and it holds the
//! kinematics config to the rules of the stage that owns it. Every one of them
//! can be wrong, so the variants sit here rather than among the run's own
//! moving parts, and the upstream errors arrive through `From` rather than as
//! copied strings.
//!
//! Three variants are about how the step sits against a time the run reads
//! rather than about either value on its own: a step may outrun neither what
//! transport can see, nor the sink that takes relief away, nor the pull back
//! toward the motion the run started from.

use crate::{
    BoundaryClassificationError, BoundaryDeformationError, MAX_GAP_RADIUS, PlateKinematicsError,
    StageInputError, TRANSPORT_REACH_HOPS,
};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlateEvolutionError {
    InvalidStepDuration,
    StepOutrunsReach,
    StepOutrunsErosion,
    InvalidGapRadius,
    InvalidPoleDriftRate,
    InvalidSpeedDriftLimit,
    InvalidReversionTime,
    StepOutrunsReversion,
    InvalidRiftRate,
    InvalidRiftAreaFraction,
    InvalidRiftCurvature,
    InvalidRiftOpeningSpeed,
    InvalidSutureTime,
    Kinematics(PlateKinematicsError),
    Deformation(BoundaryDeformationError),
    Input(StageInputError),
    Boundary(BoundaryClassificationError),
}

impl fmt::Display for PlateEvolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidStepDuration => formatter
                .write_str("run duration and step duration must be finite and non-negative"),
            Self::StepOutrunsReach => write!(
                formatter,
                "step duration must not carry a plate further than the \
                 {TRANSPORT_REACH_HOPS} cells transport looks"
            ),
            Self::StepOutrunsErosion => formatter.write_str(
                "step duration must be shorter than the erosion time, so that a step \
                 keeps some fraction of the relief it carries",
            ),
            Self::InvalidGapRadius => {
                write!(formatter, "gap radius must lie in [0, {MAX_GAP_RADIUS}]")
            }
            Self::InvalidPoleDriftRate => {
                formatter.write_str("pole drift rates must be finite and non-negative")
            }
            Self::InvalidSpeedDriftLimit => {
                formatter.write_str("speed drift limit must be finite and between 0 and 1")
            }
            Self::InvalidReversionTime => formatter.write_str(
                "pole drift reversion time must be positive; infinity disables the reversion",
            ),
            Self::StepOutrunsReversion => formatter.write_str(
                "step duration must be shorter than the pole drift reversion time, so that \
                 a step approaches the motion the run started from rather than crossing it",
            ),
            Self::InvalidRiftRate => {
                formatter.write_str("rift rate must be finite and non-negative")
            }
            Self::InvalidRiftAreaFraction => {
                formatter.write_str("minimum rift area fraction must lie in [0, 1]")
            }
            Self::InvalidRiftCurvature => {
                formatter.write_str("rift curvature must be finite and non-negative")
            }
            Self::InvalidRiftOpeningSpeed => {
                formatter.write_str("rift opening speed must be finite and non-negative")
            }
            Self::InvalidSutureTime => {
                formatter.write_str("suture time must be non-negative; infinity disables suturing")
            }
            Self::Kinematics(error) => error.fmt(formatter),
            Self::Deformation(error) => error.fmt(formatter),
            Self::Input(error) => error.fmt(formatter),
            Self::Boundary(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PlateEvolutionError {}

impl From<PlateKinematicsError> for PlateEvolutionError {
    fn from(error: PlateKinematicsError) -> Self {
        Self::Kinematics(error)
    }
}

impl From<BoundaryDeformationError> for PlateEvolutionError {
    fn from(error: BoundaryDeformationError) -> Self {
        Self::Deformation(error)
    }
}

impl From<StageInputError> for PlateEvolutionError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
    }
}

impl From<BoundaryClassificationError> for PlateEvolutionError {
    fn from(error: BoundaryClassificationError) -> Self {
        Self::Boundary(error)
    }
}
