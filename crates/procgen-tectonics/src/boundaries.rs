use crate::{PlateKinematics, PlatePartition};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

const CONVERGENCE_TO_SHEAR_THRESHOLD: f32 = 0.5;

/// Dense per-edge classification. `Interior` is the sentinel for non-boundary
/// edges so the array remains directly indexable by mesh edge id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BoundaryClass {
    Interior,
    Convergent,
    Divergent,
    Transform,
}

impl BoundaryClass {
    pub fn from_relative_motion(convergence: f32, shear: f32) -> Self {
        if convergence.abs() > shear * CONVERGENCE_TO_SHEAR_THRESHOLD {
            if convergence > 0.0 {
                Self::Convergent
            } else {
                Self::Divergent
            }
        } else {
            Self::Transform
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryClassification {
    pub edge_classes: Vec<BoundaryClass>,
    /// Signed normal speed of each side toward the other side. Entry zero is
    /// the speed of edge cell zero toward cell one; entry one is the speed of
    /// edge cell one toward cell zero. Their sum is the edge convergence.
    pub edge_normal_speeds: Vec<[f32; 2]>,
    /// Absolute relative speed parallel to the boundary, retained for
    /// downstream geological stages.
    pub edge_shear: Vec<f32>,
}

impl BoundaryClassification {
    pub fn count(&self, class: BoundaryClass) -> usize {
        self.edge_classes
            .iter()
            .filter(|&&candidate| candidate == class)
            .count()
    }

    /// Signed normal closing speed. Positive values converge; negative values
    /// diverge.
    pub fn convergence(&self, edge: usize) -> f32 {
        let speeds = self.edge_normal_speeds[edge];
        speeds[0] + speeds[1]
    }

    pub(crate) fn matches_edge_count(&self, edge_count: usize) -> bool {
        self.edge_classes.len() == edge_count
            && self.edge_normal_speeds.len() == edge_count
            && self.edge_shear.len() == edge_count
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryClassificationError {
    CellCountMismatch,
    PlateCountMismatch,
}

impl fmt::Display for BoundaryClassificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CellCountMismatch => {
                formatter.write_str("plate assignments must match the mesh cell count")
            }
            Self::PlateCountMismatch => {
                formatter.write_str("plate count must match the available angular velocities")
            }
        }
    }
}

impl std::error::Error for BoundaryClassificationError {}

/// Classifies the current, static plate boundaries from local relative motion.
/// No ownership, geometry, or crust state is advanced by this operation.
pub fn classify_boundaries(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    kinematics: &PlateKinematics,
) -> Result<BoundaryClassification, BoundaryClassificationError> {
    if partition.cell_plates.len() != mesh.cell_count() {
        return Err(BoundaryClassificationError::CellCountMismatch);
    }
    if partition.plate_count() != kinematics.angular_velocities.len() {
        return Err(BoundaryClassificationError::PlateCountMismatch);
    }

    let mut edge_classes = Vec::with_capacity(mesh.edge_count());
    let mut edge_normal_speeds = Vec::with_capacity(mesh.edge_count());
    let mut edge_shear = Vec::with_capacity(mesh.edge_count());

    for edge in &mesh.edges {
        let plate_0 = partition.cell_plates[edge.cells[0]];
        let plate_1 = partition.cell_plates[edge.cells[1]];
        let (class, normal_speeds, shear) = if plate_0 == plate_1 {
            (BoundaryClass::Interior, [0.0; 2], 0.0)
        } else {
            let unit_position =
                (mesh.vertices[edge.vertices[0]] + mesh.vertices[edge.vertices[1]]).normalized();
            let position = unit_position * mesh.radius;
            let normal =
                (mesh.cell_centers[edge.cells[1]] - mesh.cell_centers[edge.cells[0]]).normalized();
            let tangent = unit_position.cross(normal).normalized();
            let velocity_0 = kinematics.velocity_at(plate_0, position);
            let velocity_1 = kinematics.velocity_at(plate_1, position);
            let normal_speeds = [velocity_0.dot(normal), -velocity_1.dot(normal)];
            let convergence = normal_speeds.iter().sum();
            let relative_velocity = velocity_0 - velocity_1;
            let shear = relative_velocity.dot(tangent).abs();
            (
                BoundaryClass::from_relative_motion(convergence, shear),
                normal_speeds,
                shear,
            )
        };

        edge_classes.push(class);
        edge_normal_speeds.push(normal_speeds);
        edge_shear.push(shear);
    }

    Ok(BoundaryClassification {
        edge_classes,
        edge_normal_speeds,
        edge_shear,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{reference_partition, two_plate_boundary_partition};
    use crate::{PlateKinematicsConfig, generate_plate_kinematics};
    use procgen_core::Vec3;

    #[test]
    fn classification_is_deterministic_complete_and_static() {
        let (mesh, partition) = reference_partition();
        let kinematics =
            generate_plate_kinematics(partition.plate_count(), PlateKinematicsConfig::new(7))
                .unwrap();

        let first = classify_boundaries(&mesh, &partition, &kinematics).unwrap();
        assert_eq!(
            first,
            classify_boundaries(&mesh, &partition, &kinematics).unwrap()
        );
        assert_eq!(first.edge_classes.len(), mesh.edge_count());
        assert_eq!(first.edge_normal_speeds.len(), mesh.edge_count());
        assert_eq!(first.edge_shear.len(), mesh.edge_count());
        assert_eq!(
            first.count(BoundaryClass::Interior),
            mesh.edges
                .iter()
                .filter(|edge| partition.cell_plates[edge.cells[0]]
                    == partition.cell_plates[edge.cells[1]])
                .count()
        );
        assert!(first.count(BoundaryClass::Convergent) > 0);
        assert!(first.count(BoundaryClass::Divergent) > 0);
        assert!(first.count(BoundaryClass::Transform) > 0);
    }

    #[test]
    fn classifies_relative_motion_from_scalar_components() {
        assert_eq!(
            BoundaryClass::from_relative_motion(1.0, 0.0),
            BoundaryClass::Convergent
        );
        assert_eq!(
            BoundaryClass::from_relative_motion(-1.0, 0.0),
            BoundaryClass::Divergent
        );
        assert_eq!(
            BoundaryClass::from_relative_motion(0.25, 1.0),
            BoundaryClass::Transform
        );
    }

    #[test]
    fn cell_to_cell_relative_motion_is_convergent() {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let unit_position =
            (mesh.vertices[edge.vertices[0]] + mesh.vertices[edge.vertices[1]]).normalized();
        let normal =
            (mesh.cell_centers[edge.cells[1]] - mesh.cell_centers[edge.cells[0]]).normalized();
        let kinematics = PlateKinematics {
            angular_velocities: vec![unit_position.cross(normal), Vec3::ZERO],
        };

        let boundaries = classify_boundaries(&mesh, &partition, &kinematics).unwrap();

        assert_eq!(
            boundaries.edge_classes[edge_index],
            BoundaryClass::Convergent
        );
        assert!(boundaries.convergence(edge_index) > 0.0);
    }
}
