use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageInputError {
    Cells,
    Plates,
    PlateOwnership,
    EmptyPlate,
    Boundaries,
    CrustClasses,
    CrustBirth,
    CrustBirthAfterRun,
    Elevation,
    BaseElevation,
    Deformation,
    SeafloorAge,
}

impl fmt::Display for StageInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cells => formatter.write_str("plate assignments must match the mesh cell count"),
            Self::Plates => formatter.write_str("plate data must match the partition plate count"),
            Self::PlateOwnership => formatter.write_str("every cell must reference a valid plate"),
            Self::EmptyPlate => formatter.write_str("every plate must own at least one cell"),
            Self::Boundaries => {
                formatter.write_str("boundary arrays must match the mesh edge count")
            }
            Self::CrustClasses => {
                formatter.write_str("crust classes must match the mesh cell count")
            }
            Self::CrustBirth => {
                formatter.write_str("crust birth times must match the mesh cell count")
            }
            Self::CrustBirthAfterRun => {
                formatter.write_str("no crust can be born after the run's elapsed time")
            }
            Self::Elevation => {
                formatter.write_str("elevation values must match the mesh cell count")
            }
            Self::BaseElevation => {
                formatter.write_str("base-elevation values must match the mesh cell count")
            }
            Self::Deformation => {
                formatter.write_str("deformation values must match the mesh cell count")
            }
            Self::SeafloorAge => {
                formatter.write_str("seafloor-age values must match the mesh cell count")
            }
        }
    }
}

impl std::error::Error for StageInputError {}
