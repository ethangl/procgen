use crate::{BoundaryClassification, CrustClassification, PlatePartition};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageInputError {
    Cells,
    Plates,
    PlateOwnership,
    Boundaries,
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
        }
    }
}

impl std::error::Error for StageInputError {}

pub(crate) fn validate_ownership_and_crust(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
) -> Result<(), StageInputError> {
    partition.validate(mesh)?;
    if crust.plate_classes.len() != partition.plate_count {
        return Err(StageInputError::Plates);
    }
    Ok(())
}

pub(crate) fn validate_boundaries(
    mesh: &SphereMesh,
    boundaries: &BoundaryClassification,
) -> Result<(), StageInputError> {
    if !boundaries.matches_edge_count(mesh.edge_count()) {
        return Err(StageInputError::Boundaries);
    }
    Ok(())
}
