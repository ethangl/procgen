use crate::{PlateKinematics, PlatePartition};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

const CONVERGENCE_TO_SHEAR_THRESHOLD: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BoundaryClass {
    Interior,
    Convergent,
    Divergent,
    Transform,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryClassification {
    pub edge_classes: Vec<BoundaryClass>,
    /// Signed normal closing speed. Positive values converge; negative values diverge.
    pub edge_convergence: Vec<f32>,
    /// Absolute relative speed parallel to the boundary.
    pub edge_shear: Vec<f32>,
}

impl BoundaryClassification {
    pub fn count(&self, class: BoundaryClass) -> usize {
        self.edge_classes
            .iter()
            .filter(|&&candidate| candidate == class)
            .count()
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
            Self::PlateCountMismatch => formatter.write_str(
                "plate assignments and seeds must reference available angular velocities",
            ),
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
    if partition.plate_count() != kinematics.angular_velocities.len()
        || partition
            .cell_plates
            .iter()
            .any(|&plate| plate >= kinematics.angular_velocities.len())
    {
        return Err(BoundaryClassificationError::PlateCountMismatch);
    }

    let mut edge_classes = Vec::with_capacity(mesh.edge_count());
    let mut edge_convergence = Vec::with_capacity(mesh.edge_count());
    let mut edge_shear = Vec::with_capacity(mesh.edge_count());

    for edge in &mesh.edges {
        let plate_0 = partition.cell_plates[edge.cells[0]];
        let plate_1 = partition.cell_plates[edge.cells[1]];
        if plate_0 == plate_1 {
            edge_classes.push(BoundaryClass::Interior);
            edge_convergence.push(0.0);
            edge_shear.push(0.0);
            continue;
        }

        let position = (mesh.vertices[edge.vertices[0]] + mesh.vertices[edge.vertices[1]])
            .normalized()
            * mesh.radius;
        let normal =
            (mesh.cell_centers[edge.cells[1]] - mesh.cell_centers[edge.cells[0]]).normalized();
        let tangent = position.normalized().cross(normal).normalized();
        let relative_velocity =
            kinematics.velocity_at(plate_0, position) - kinematics.velocity_at(plate_1, position);
        let convergence = relative_velocity.dot(normal);
        let shear = relative_velocity.dot(tangent).abs();
        let class = if convergence.abs() > shear * CONVERGENCE_TO_SHEAR_THRESHOLD {
            if convergence > 0.0 {
                BoundaryClass::Convergent
            } else {
                BoundaryClass::Divergent
            }
        } else {
            BoundaryClass::Transform
        };

        edge_classes.push(class);
        edge_convergence.push(convergence);
        edge_shear.push(shear);
    }

    Ok(BoundaryClassification {
        edge_classes,
        edge_convergence,
        edge_shear,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PlateKinematicsConfig, PlatePartitionConfig, generate_plate_kinematics, partition_plates,
    };
    use procgen_core::Vec3;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::build_sphere_mesh;

    fn mesh() -> SphereMesh {
        build_sphere_mesh(
            fibonacci_sphere(FibonacciConfig {
                count: 512,
                jitter: 0.5,
                seed: 7,
            })
            .unwrap(),
            1.0,
        )
        .unwrap()
    }

    #[test]
    fn classification_is_deterministic_complete_and_static() {
        let mesh = mesh();
        let partition = partition_plates(
            &mesh,
            PlatePartitionConfig {
                major_plate_count: 5,
                minor_plate_count: 11,
                major_head_start_rounds: 2,
                seed: 7,
            },
        )
        .unwrap();
        let original_partition = partition.clone();
        let kinematics =
            generate_plate_kinematics(partition.plate_count(), PlateKinematicsConfig::new(7))
                .unwrap();

        let first = classify_boundaries(&mesh, &partition, &kinematics).unwrap();
        assert_eq!(
            first,
            classify_boundaries(&mesh, &partition, &kinematics).unwrap()
        );
        assert_eq!(partition, original_partition);
        assert_eq!(first.edge_classes.len(), mesh.edge_count());
        assert_eq!(first.edge_convergence.len(), mesh.edge_count());
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
    fn known_rotations_produce_expected_boundary_classes() {
        let mesh = mesh();
        let edge = mesh.edges[0];
        let mut cell_plates = vec![0; mesh.cell_count()];
        cell_plates[edge.cells[1]] = 1;
        let partition = PlatePartition {
            cell_plates,
            plate_seeds: vec![edge.cells[0], edge.cells[1]],
            major_plate_count: 2,
        };
        let position =
            (mesh.vertices[edge.vertices[0]] + mesh.vertices[edge.vertices[1]]).normalized();
        let normal =
            (mesh.cell_centers[edge.cells[1]] - mesh.cell_centers[edge.cells[0]]).normalized();
        let tangent = position.cross(normal).normalized();

        for (relative_velocity, expected) in [
            (normal, BoundaryClass::Convergent),
            (-normal, BoundaryClass::Divergent),
            (tangent, BoundaryClass::Transform),
        ] {
            // At a unit position, omega = position x velocity yields omega x position = velocity.
            let kinematics = PlateKinematics {
                angular_velocities: vec![position.cross(relative_velocity), Vec3::ZERO],
            };
            let boundaries = classify_boundaries(&mesh, &partition, &kinematics).unwrap();
            assert_eq!(boundaries.edge_classes[0], expected);
        }
    }
}
