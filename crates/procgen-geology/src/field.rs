use procgen_core::{RandomStream, Vec3};
use procgen_sphere_mesh::SphereMesh;
use procgen_tectonics::StageInputError;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeologyInputError {
    Hotspots,
    OceanicPeaks,
    VolcanicArcs,
    Cratons,
    Basins,
    Elevation,
    Isostasy,
}

impl fmt::Display for GeologyInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let field = match self {
            Self::Hotspots => "hotspot aggregate",
            Self::OceanicPeaks => "oceanic-peak aggregate",
            Self::VolcanicArcs => "volcanic-arc aggregate",
            Self::Cratons => "craton",
            Self::Basins => "sedimentary-basin",
            Self::Elevation => "geological-elevation",
            Self::Isostasy => "isostatic-adjustment",
        };
        write!(formatter, "{field} field is inconsistent with the mesh")
    }
}

impl std::error::Error for GeologyInputError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeologyStageError {
    Input(StageInputError),
    Geology(GeologyInputError),
    InvalidConfig,
}

impl fmt::Display for GeologyStageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => error.fmt(formatter),
            Self::Geology(error) => error.fmt(formatter),
            Self::InvalidConfig => formatter.write_str(
                "stage configuration values must be finite, fractions between zero and one and lengths not negative",
            ),
        }
    }
}

impl std::error::Error for GeologyStageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            Self::Geology(error) => Some(error),
            Self::InvalidConfig => None,
        }
    }
}

impl From<GeologyInputError> for GeologyStageError {
    fn from(error: GeologyInputError) -> Self {
        Self::Geology(error)
    }
}

impl From<StageInputError> for GeologyStageError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
    }
}

/// Aggregate change produced by one elevation effect.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ElevationEffectDiagnostics {
    pub affected_cell_count: usize,
    /// Signed sum of the effect's actual per-cell changes after clamping.
    pub total_delta: f64,
    pub maximum_absolute_delta: f32,
}

impl ElevationEffectDiagnostics {
    fn record_change(&mut self, before: f32, after: f32) {
        let delta = after - before;
        if delta != 0.0 {
            self.affected_cell_count += 1;
            self.total_delta += f64::from(delta);
            self.maximum_absolute_delta = self.maximum_absolute_delta.max(delta.abs());
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SignedEffectDiagnostics {
    pub rise: ElevationEffectDiagnostics,
    pub sink: ElevationEffectDiagnostics,
}

pub(crate) trait ElevationEffectRecorder {
    fn record(&mut self, before: f32, after: f32);
}

impl ElevationEffectRecorder for ElevationEffectDiagnostics {
    fn record(&mut self, before: f32, after: f32) {
        self.record_change(before, after);
    }
}

impl ElevationEffectRecorder for SignedEffectDiagnostics {
    fn record(&mut self, before: f32, after: f32) {
        if after > before {
            self.rise.record_change(before, after);
        } else {
            self.sink.record_change(before, after);
        }
    }
}

pub(crate) fn apply_elevation_effect(
    elevations: &mut [f32],
    diagnostics: &mut impl ElevationEffectRecorder,
    mut effect: impl FnMut(usize, f32) -> f32,
) {
    for (cell, elevation) in elevations.iter_mut().enumerate() {
        let before = *elevation;
        *elevation = effect(cell, before);
        diagnostics.record(before, *elevation);
    }
}

pub(crate) fn lerp(value: f32, target: f32, amount: f32) -> f32 {
    value + (target - value) * amount
}

pub(crate) fn unit_interval(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

pub(crate) struct MaxWinsField<T> {
    values: Vec<f32>,
    winners: Vec<Option<T>>,
    contribution_counts: Vec<usize>,
}

impl<T: Copy + Ord> MaxWinsField<T> {
    pub(crate) fn new(cell_count: usize) -> Self {
        Self {
            values: vec![0.0; cell_count],
            winners: vec![None; cell_count],
            contribution_counts: vec![0; cell_count],
        }
    }

    pub(crate) fn claim(&mut self, cell: usize, value: f32, index: T) {
        self.contribution_counts[cell] += 1;
        let wins = self.winners[cell].is_none_or(|winner| {
            value > self.values[cell] || (value == self.values[cell] && index < winner)
        });
        if wins {
            self.values[cell] = value;
            self.winners[cell] = Some(index);
        }
    }

    pub(crate) fn affected_cell_count(&self) -> usize {
        self.contribution_counts
            .iter()
            .filter(|&&count| count > 0)
            .count()
    }

    pub(crate) fn overlap_cell_count(&self) -> usize {
        self.contribution_counts
            .iter()
            .filter(|&&count| count > 1)
            .count()
    }

    pub(crate) fn into_parts(self) -> (Vec<f32>, Vec<Option<T>>) {
        (self.values, self.winners)
    }
}

/// A hashed surface position inside `cell`, for the `peak`-th peak the cell
/// carries.
///
/// A cell can hold more than one peak of a field whose density asks for more
/// than one, and two peaks at the same point are one peak, so each takes its
/// own block of three draws. Peak zero takes draws zero to two, which is the
/// block a one-peak-per-cell rule took.
///
/// The point is a convex blend of the cell center and two adjacent corners,
/// so it is inside the cell by construction rather than by a test.
/// `maximum_offset` is how far toward the corner pair it may reach.
pub(crate) fn position_in_cell(
    mesh: &SphereMesh,
    cell: usize,
    stream: RandomStream,
    peak: usize,
    maximum_offset: f32,
) -> Vec3 {
    let corners = mesh.cell_corners(cell);
    let base = DRAWS_PER_POSITION * peak as u64;
    let item = cell as u64;
    let corner_index = stream.sample_u64(item, base) as usize % corners.len();
    let mut first_weight = stream.unit_f32(item, base + 1);
    let mut second_weight = stream.unit_f32(item, base + 2);
    if first_weight + second_weight > 1.0 {
        first_weight = 1.0 - first_weight;
        second_weight = 1.0 - second_weight;
    }
    first_weight *= maximum_offset;
    second_weight *= maximum_offset;
    mesh.interpolate_cell_triangle(
        cell,
        corner_index,
        [
            1.0 - first_weight - second_weight,
            first_weight,
            second_weight,
        ],
    )
}

/// Draws one position takes: the corner pair it sits against and the two
/// weights that place it between them.
const DRAWS_PER_POSITION: u64 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_effect_diagnostics_dispatches_rise_sink_and_ignores_zero() {
        let mut diagnostics = SignedEffectDiagnostics::default();
        diagnostics.record(0.25, 0.5);
        diagnostics.record(0.75, 0.25);
        diagnostics.record(0.5, 0.5);

        assert_eq!(diagnostics.rise.affected_cell_count, 1);
        assert_eq!(diagnostics.rise.total_delta, 0.25);
        assert_eq!(diagnostics.rise.maximum_absolute_delta, 0.25);
        assert_eq!(diagnostics.sink.affected_cell_count, 1);
        assert_eq!(diagnostics.sink.total_delta, -0.5);
        assert_eq!(diagnostics.sink.maximum_absolute_delta, 0.5);
    }

    #[test]
    fn zero_value_gets_a_winner_and_equal_ties_use_the_lower_index() {
        let mut field = MaxWinsField::new(1);
        field.claim(0, 0.0, 3);
        field.claim(0, 0.0, 1);

        assert_eq!(field.affected_cell_count(), 1);
        assert_eq!(field.overlap_cell_count(), 1);
        assert_eq!(field.into_parts(), (vec![0.0], vec![Some(1)]));
    }
}
