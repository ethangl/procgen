use std::{error::Error, fmt};

use procgen_core::Vec3;

/// Bounded local query domain for this experiment, in model lengths.
pub const MAX_COORDINATE: f32 = 8.0;
pub(crate) const MIN_GRID_SIDE: usize = 2;
pub(crate) const MAX_GRID_SIDE: usize = 128;

#[derive(Clone, Debug, PartialEq)]
pub enum FieldError {
    Parameter {
        name: &'static str,
        min: f32,
        max: f32,
    },
    Position,
    GridBounds,
    GridSide,
    SliceIndex {
        index: usize,
        side: usize,
    },
    Volume,
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parameter { name, min, max } => {
                write!(f, "{name} must be finite and in [{min}, {max}]")
            }
            Self::Position => write!(f, "positions must be finite and within +/-{MAX_COORDINATE}"),
            Self::GridBounds => write!(f, "grid bounds must increase on every axis"),
            Self::GridSide => write!(
                f,
                "sample count must be a power of two in [{MIN_GRID_SIDE}, {MAX_GRID_SIDE}]"
            ),
            Self::SliceIndex { index, side } => {
                write!(f, "slice index {index} must be less than {side}")
            }
            Self::Volume => write!(f, "volume samples must match the grid and be finite"),
        }
    }
}

impl Error for FieldError {}

pub(crate) fn validate_range(
    name: &'static str,
    value: f32,
    range: std::ops::RangeInclusive<f32>,
) -> Result<(), FieldError> {
    let (min, max) = (*range.start(), *range.end());
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(FieldError::Parameter { name, min, max })
    }
}

pub(crate) fn validate_position(p: Vec3) -> Result<(), FieldError> {
    if p.is_finite()
        && p.x.abs() <= MAX_COORDINATE
        && p.y.abs() <= MAX_COORDINATE
        && p.z.abs() <= MAX_COORDINATE
    {
        Ok(())
    } else {
        Err(FieldError::Position)
    }
}
