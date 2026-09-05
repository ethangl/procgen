#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FieldSummary {
    pub minimum: f32,
    pub maximum: f32,
    pub mean: f32,
}

impl FieldSummary {
    pub fn from_values(values: &[f32]) -> Self {
        summarize_field(values, |_| {})
    }
}

pub(crate) fn summarize_field(values: &[f32], mut inspect: impl FnMut(f32)) -> FieldSummary {
    let Some((&first, rest)) = values.split_first() else {
        return FieldSummary::default();
    };
    inspect(first);
    let (minimum, maximum, total) = rest.iter().fold(
        (first, first, f64::from(first)),
        |(minimum, maximum, total), &value| {
            inspect(value);
            (
                minimum.min(value),
                maximum.max(value),
                total + f64::from(value),
            )
        },
    );
    FieldSummary {
        minimum,
        maximum,
        mean: (total / values.len() as f64) as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_empty_and_populated_fields() {
        assert_eq!(FieldSummary::from_values(&[]), FieldSummary::default());
        assert_eq!(
            FieldSummary::from_values(&[-2.0, 1.0, 4.0]),
            FieldSummary {
                minimum: -2.0,
                maximum: 4.0,
                mean: 1.0,
            }
        );
    }
}
