use procgen_tectonics::StageInputError;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeologyInputError {
    Hotspots,
    VolcanicArcs,
    Cratons,
    Basins,
    Elevation,
}

impl fmt::Display for GeologyInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let field = match self {
            Self::Hotspots => "hotspot aggregate",
            Self::VolcanicArcs => "volcanic-arc aggregate",
            Self::Cratons => "craton",
            Self::Basins => "sedimentary-basin",
            Self::Elevation => "geological-elevation",
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
            Self::InvalidConfig => formatter
                .write_str("geological elevation values must be finite and between zero and one"),
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
    pub(crate) fn record(&mut self, before: f32, after: f32) {
        let delta = after - before;
        if delta != 0.0 {
            self.affected_cell_count += 1;
            self.total_delta += f64::from(delta);
            self.maximum_absolute_delta = self.maximum_absolute_delta.max(delta.abs());
        }
    }
}

pub(crate) fn apply_elevation_effect(
    elevations: &mut [f32],
    mut effect: impl FnMut(usize, f32) -> f32,
    mut record: impl FnMut(f32, f32),
) {
    for (cell, elevation) in elevations.iter_mut().enumerate() {
        let before = *elevation;
        *elevation = effect(cell, before);
        record(before, *elevation);
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

#[cfg(test)]
mod tests {
    use super::*;

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
