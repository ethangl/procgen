use procgen_sphere_mesh::SphereMesh;

/// Model time per step at the viewer's default mesh: the unit sphere's cell
/// width `sqrt(4 pi / 65_536)` is 0.0138, and a plate at the default maximum
/// angular speed of 1.0 covers unit distance per unit time, so one cell width
/// takes that long. Rounded to a round number, because nothing downstream
/// resolves the difference. Evolution advances by it and deformation measures
/// its accumulation against it, so it lives beside the cell width both are
/// scaled against.
pub const DEFAULT_STEP_DURATION: f32 = 0.014;

/// The mesh's one representative cell width: the side of a square with the
/// mean cell area. Evolution measures every accumulated displacement against
/// it, so a finer mesh moves more cells for the same motion.
pub(crate) fn mean_cell_width(mesh: &SphereMesh) -> f32 {
    (mesh.total_area() / mesh.cell_count() as f64).sqrt() as f32
}

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
