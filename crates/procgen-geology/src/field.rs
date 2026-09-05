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
