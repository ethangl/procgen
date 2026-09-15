use std::{error::Error, fmt};

/// A design parameter that fell outside its declared range, or was not finite.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldError {
    Parameter {
        name: &'static str,
        min: f32,
        max: f32,
    },
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parameter { name, min, max } => {
                write!(f, "{name} must be finite and in [{min}, {max}]")
            }
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
