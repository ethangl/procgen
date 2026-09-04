use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, build_sphere_mesh};

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
