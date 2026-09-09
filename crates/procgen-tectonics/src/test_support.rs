pub use procgen_core::fingerprint;
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, build_sphere_mesh};

use crate::{
    BoundaryClass, BoundaryClassification, CrustClassification, CrustClassificationConfig,
    PlateEvolutionConfig, PlateKinematicsConfig, PlatePartition, PlatePartitionConfig,
    classify_crust, evolve_plate_ownership, generate_plate_kinematics, partition_plates,
};

pub fn mesh(cell_count: usize) -> SphereMesh {
    build_sphere_mesh(
        fibonacci_sphere(FibonacciConfig {
            count: cell_count,
            jitter: 0.5,
            seed: 7,
        })
        .unwrap(),
        1.0,
    )
    .unwrap()
}

/// Arcs and minor-plate size scaled to the 512-cell reference mesh: enough
/// crack faces for merging to matter, splitting into plates of roughly
/// thirty-two cells.
pub fn reference_partition_config() -> PlatePartitionConfig {
    PlatePartitionConfig {
        arc_count: 20,
        piece_fraction: 32.0 / 512.0,
        growth_roughness: 0,
        seed: 7,
        ..PlatePartitionConfig::default()
    }
}

pub fn reference_partition() -> (SphereMesh, PlatePartition) {
    let mesh = mesh(512);
    let partition = partition_plates(&mesh, reference_partition_config()).unwrap();
    (mesh, partition)
}

pub fn final_state_fixture() -> (
    SphereMesh,
    PlatePartition,
    CrustClassification,
    BoundaryClassification,
) {
    let (mesh, initial) = reference_partition();
    let crust = classify_crust(&mesh, &initial, CrustClassificationConfig::new(17)).unwrap();
    let kinematics =
        generate_plate_kinematics(&mesh, &initial, &crust, PlateKinematicsConfig::new(7)).unwrap();
    let evolution = evolve_plate_ownership(
        &mesh,
        &initial,
        &crust,
        &kinematics,
        PlateEvolutionConfig::default(),
    )
    .unwrap();
    (mesh, evolution.partition, crust, evolution.boundaries)
}

pub fn two_plate_boundary_partition() -> (SphereMesh, usize, PlatePartition) {
    let mesh = mesh(32);
    let edge_index = 0;
    let edge = mesh.edges[edge_index];
    let mut cell_plates = vec![0; mesh.cell_count()];
    cell_plates[edge.cells[1]] = 1;
    let partition = PlatePartition {
        cell_plates,
        plate_count: 2,
    };
    (mesh, edge_index, partition)
}

pub fn empty_boundaries(mesh: &SphereMesh) -> BoundaryClassification {
    BoundaryClassification {
        edge_classes: vec![BoundaryClass::Interior; mesh.edge_count()],
        edge_normal_speeds: vec![[0.0; 2]; mesh.edge_count()],
        edge_shear: vec![0.0; mesh.edge_count()],
    }
}
