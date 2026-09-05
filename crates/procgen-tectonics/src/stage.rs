use crate::{BoundaryClassification, CrustClassification, PlatePartition};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageInputError {
    Cells,
    Plates,
    Boundaries,
}

impl fmt::Display for StageInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cells => formatter.write_str("plate assignments must match the mesh cell count"),
            Self::Plates => {
                formatter.write_str("plate classes must match the partition plate count")
            }
            Self::Boundaries => {
                formatter.write_str("boundary arrays must match the mesh edge count")
            }
        }
    }
}

impl std::error::Error for StageInputError {}

/// Validates the shared final ownership and crust inputs, plus boundary state
/// for stages that consume it.
pub(crate) fn validate_final_state(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    boundaries: Option<&BoundaryClassification>,
) -> Result<(), StageInputError> {
    if partition.cell_plates.len() != mesh.cell_count() {
        return Err(StageInputError::Cells);
    }
    if crust.plate_classes.len() != partition.plate_count {
        return Err(StageInputError::Plates);
    }
    if boundaries.is_some_and(|boundaries| !boundaries.matches_edge_count(mesh.edge_count())) {
        return Err(StageInputError::Boundaries);
    }
    Ok(())
}
