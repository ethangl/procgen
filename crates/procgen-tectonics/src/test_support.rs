use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, build_sphere_mesh};

use crate::{PlatePartition, PlatePartitionConfig, partition_plates};

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

pub const fn reference_partition_config() -> PlatePartitionConfig {
    PlatePartitionConfig {
        major_plate_count: 5,
        minor_plate_count: 11,
        major_head_start_rounds: 2,
        seed: 7,
    }
}

pub fn reference_partition() -> (SphereMesh, PlatePartition) {
    let mesh = mesh(512);
    let partition = partition_plates(&mesh, reference_partition_config()).unwrap();
    (mesh, partition)
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
