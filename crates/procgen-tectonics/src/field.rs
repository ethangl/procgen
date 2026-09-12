/// Model time per step at the viewer's default mesh: the unit sphere's cell
/// width `sqrt(4 pi / 65_536)` is 0.0138, and a plate at the default maximum
/// angular speed of 1.0 covers unit distance per unit time, so one cell width
/// takes that long. Rounded to a round number, because nothing downstream
/// resolves the difference. That width is
/// [`procgen_sphere_mesh::default_hop_length`], which every model length in
/// this crate's defaults is written as a multiple of.
///
/// It is a time, so every rate and every age a run reads is measured against
/// it and none of them changes meaning when it does. What it buys at this
/// value is one cell of travel a step for the fastest starting plate, and
/// never more than the two cells transport can see: at the viewer's defaults
/// [`crate::maximum_step_duration`] returns 0.0151, so this leaves eight
/// percent of spare reach for drift and a rift to take.
pub const DEFAULT_STEP_DURATION: f32 = 0.014;

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
