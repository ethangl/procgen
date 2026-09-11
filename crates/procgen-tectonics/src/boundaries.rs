use crate::{CellCrust, PlateKinematics, PlatePartition, StageInputError};
use procgen_sphere_mesh::SphereMesh;
use std::{cmp::Ordering, fmt};

/// Ratio of shear a boundary's convergence must exceed to be read as normal
/// motion rather than transform motion.
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
    pub const ALL: [Self; 4] = [
        Self::Interior,
        Self::Convergent,
        Self::Divergent,
        Self::Transform,
    ];

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

/// Dense boundary state with one contiguous array per edge attribute for
/// data-parallel CPU access and future backend transfer. Stage boundaries
/// validate that every attribute covers the same mesh edges.
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

    /// Motion magnitude relevant to the edge's boundary class. Normal motion
    /// drives convergent and divergent edges; shear drives transform edges.
    /// Interior edges have no boundary strength.
    pub fn strength(&self, edge: usize) -> Option<f32> {
        match self.edge_classes[edge] {
            BoundaryClass::Convergent | BoundaryClass::Divergent => {
                Some(self.convergence(edge).abs())
            }
            BoundaryClass::Transform => Some(self.edge_shear[edge]),
            BoundaryClass::Interior => None,
        }
    }

    /// Validates that every dense boundary attribute covers the mesh edges.
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), StageInputError> {
        if self.edge_classes.len() != mesh.edge_count()
            || self.edge_normal_speeds.len() != mesh.edge_count()
            || self.edge_shear.len() != mesh.edge_count()
        {
            return Err(StageInputError::Boundaries);
        }
        Ok(())
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
    if partition.plate_count != kinematics.angular_velocities.len() {
        return Err(BoundaryClassificationError::PlateCountMismatch);
    }
    Ok(classify_validated(mesh, partition, kinematics))
}

/// The classification itself, over inputs already known to agree. The
/// kinematics stage classifies its own first pass to reach the subducting
/// fractions, and it has validated both sides already.
pub(crate) fn classify_validated(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    kinematics: &PlateKinematics,
) -> BoundaryClassification {
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
            let convergence = normal_speeds[0] + normal_speeds[1];
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

    BoundaryClassification {
        edge_classes,
        edge_normal_speeds,
        edge_shear,
    }
}

/// The share of each plate's boundary edges that are subducting slab: the
/// convergent ones where the plate's own cell is the one going under, which is
/// the cell [`crate::material_order`] ranks `Less`.
///
/// It is the one thing about a plate that predicts how fast it moves — Forsyth
/// and Uyeda's result, and what [`crate::plate_speed`] reads. Two integer
/// counts per plate over one pass of the edges, so the fraction is exact.
///
/// An `Equal` edge is not slab: two continents, or two floors of one age, pull
/// neither side. A plate owning no boundary edge at all reads zero, which
/// compaction makes unreachable for a plate that owns a cell.
///
/// `boundaries` need not be the classification of the current ownership.
/// Evolution passes the boundaries its step began with over the ownership
/// transport has since moved, so an edge counts when its two cells are on
/// different plates now and it was convergent then.
pub fn subducting_fractions(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: CellCrust<'_>,
    boundaries: &BoundaryClassification,
) -> Vec<f64> {
    let mut slab = vec![0_usize; partition.plate_count];
    let mut boundary = vec![0_usize; partition.plate_count];
    for (index, edge) in mesh.edges.iter().enumerate() {
        let plates = edge.cells.map(|cell| partition.cell_plates[cell]);
        if plates[0] == plates[1] {
            continue;
        }
        boundary[plates[0]] += 1;
        boundary[plates[1]] += 1;
        if boundaries.edge_classes[index] != BoundaryClass::Convergent {
            continue;
        }
        // The lesser cell's floor is the slab, so its plate is the one pulled.
        match crust.order(edge.cells[0], edge.cells[1]) {
            Ordering::Less => slab[plates[0]] += 1,
            Ordering::Greater => slab[plates[1]] += 1,
            Ordering::Equal => {}
        }
    }
    slab.iter()
        .zip(&boundary)
        .map(|(&slab, &boundary)| match boundary {
            0 => 0.0,
            boundary => slab as f64 / boundary as f64,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        mesh as test_mesh, reference_crust_config, reference_partition,
        two_plate_boundary_partition,
    };
    use crate::{PlateKinematicsConfig, classify_crust, generate_plate_kinematics};
    use procgen_core::Vec3;

    /// Cell count of the hemisphere fixture. Its equator runs through 92
    /// edges, an even number, so a test can converge exactly half of them.
    const HEMISPHERE_CELLS: usize = 512;

    /// Two plates split at the equator, every shared edge converging. The
    /// crust each plate carries is the test's to choose, so one fixture covers
    /// every polarity a convergent boundary can have. It is the shape the
    /// volcanic-arc tests use for the same reason.
    fn hemispheres() -> (SphereMesh, PlatePartition, BoundaryClassification) {
        let mesh = test_mesh(HEMISPHERE_CELLS);
        let cell_plates: Vec<usize> = mesh
            .cell_centers
            .iter()
            .map(|center| usize::from(center.z < 0.0))
            .collect();
        let mut boundaries = BoundaryClassification {
            edge_classes: vec![BoundaryClass::Interior; mesh.edge_count()],
            edge_normal_speeds: vec![[0.0; 2]; mesh.edge_count()],
            edge_shear: vec![0.0; mesh.edge_count()],
        };
        for (index, edge) in mesh.edges.iter().enumerate() {
            if cell_plates[edge.cells[0]] != cell_plates[edge.cells[1]] {
                boundaries.edge_classes[index] = BoundaryClass::Convergent;
                boundaries.edge_normal_speeds[index] = [0.5, 0.5];
            }
        }
        let partition = PlatePartition {
            cell_plates,
            plate_count: 2,
        };
        (mesh, partition, boundaries)
    }

    /// The per-cell birth that gives each hemisphere one crust.
    fn hemisphere_crust(partition: &PlatePartition, births: [Option<f32>; 2]) -> Vec<Option<f32>> {
        partition
            .cell_plates
            .iter()
            .map(|&plate| births[plate])
            .collect()
    }

    #[test]
    fn a_subducting_plate_owns_the_whole_of_a_convergent_boundary() {
        let (mesh, partition, boundaries) = hemispheres();
        let fractions = |births: [Option<f32>; 2]| {
            let cell_birth = hemisphere_crust(&partition, births);
            subducting_fractions(
                &mesh,
                &partition,
                CellCrust {
                    cell_birth: &cell_birth,
                },
                &boundaries,
            )
        };

        // Older floor under younger — a birth further back is the older — so
        // every boundary edge the older plate owns is its own slab and none of
        // the younger plate's is.
        assert_eq!(fractions([Some(0.5), Some(0.1)]), vec![0.0, 1.0]);
        assert_eq!(fractions([Some(0.1), Some(0.5)]), vec![1.0, 0.0]);
        // Ocean under continent is the same polarity read the other way.
        assert_eq!(fractions([Some(0.1), None]), vec![1.0, 0.0]);
        // Two continents and two floors of one age each rank `Equal`, which
        // pulls neither plate.
        assert_eq!(fractions([None, None]), vec![0.0, 0.0]);
        assert_eq!(fractions([Some(0.1), Some(0.1)]), vec![0.0, 0.0]);
    }

    #[test]
    fn only_convergent_edges_are_slab_and_only_they_are_counted() {
        let (mesh, partition, mut boundaries) = hemispheres();
        let cell_birth = hemisphere_crust(&partition, [Some(0.5), Some(0.1)]);
        let crust = CellCrust {
            cell_birth: &cell_birth,
        };

        // A boundary that shears carries no slab whatever lies either side.
        let shared: Vec<usize> = (0..mesh.edge_count())
            .filter(|&edge| boundaries.edge_classes[edge] != BoundaryClass::Interior)
            .collect();
        for &edge in &shared {
            boundaries.edge_classes[edge] = BoundaryClass::Transform;
        }
        assert_eq!(
            subducting_fractions(&mesh, &partition, crust, &boundaries),
            vec![0.0, 0.0]
        );

        // Half the shared edges converging is half the perimeter of the older
        // plate, exactly: the fraction is two integer counts and one divide.
        assert!(
            shared.len().is_multiple_of(2),
            "{} shared edges",
            shared.len()
        );
        for &edge in &shared[..shared.len() / 2] {
            boundaries.edge_classes[edge] = BoundaryClass::Convergent;
        }
        assert_eq!(
            subducting_fractions(&mesh, &partition, crust, &boundaries),
            vec![0.0, 0.5]
        );
    }

    #[test]
    fn classification_is_deterministic_complete_and_static() {
        let (mesh, partition) = reference_partition();
        let crust = classify_crust(&mesh, reference_crust_config()).unwrap();
        let kinematics =
            generate_plate_kinematics(&mesh, &partition, &crust, PlateKinematicsConfig::new(7))
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
    fn boundary_strength_uses_class_relevant_motion() {
        let boundaries = BoundaryClassification {
            edge_classes: vec![
                BoundaryClass::Interior,
                BoundaryClass::Convergent,
                BoundaryClass::Transform,
            ],
            edge_normal_speeds: vec![[0.0, 0.0], [0.4, 0.6], [0.1, 0.2]],
            edge_shear: vec![0.0, 0.2, 1.5],
        };

        assert_eq!(boundaries.strength(0), None);
        assert_eq!(boundaries.strength(1), Some(1.0));
        assert_eq!(boundaries.strength(2), Some(1.5));
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
            base_speeds: vec![1.0; 2],
        };

        let boundaries = classify_boundaries(&mesh, &partition, &kinematics).unwrap();

        assert_eq!(
            boundaries.edge_classes[edge_index],
            BoundaryClass::Convergent
        );
        assert!(boundaries.convergence(edge_index) > 0.0);
    }
}
