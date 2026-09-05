use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageInputError {
    Cells,
    Plates,
    PlateOwnership,
    Boundaries,
    Elevation,
}

impl fmt::Display for StageInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cells => formatter.write_str("plate assignments must match the mesh cell count"),
            Self::Plates => formatter.write_str("plate data must match the partition plate count"),
            Self::PlateOwnership => formatter.write_str("every cell must reference a valid plate"),
            Self::Boundaries => {
                formatter.write_str("boundary arrays must match the mesh edge count")
            }
            Self::Elevation => {
                formatter.write_str("elevation values must match the mesh cell count")
            }
        }
    }
}

impl std::error::Error for StageInputError {}
