use procgen_core::Vec3;
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, build_sphere_mesh};

fn mesh(count: usize) -> SphereMesh {
    let points = fibonacci_sphere(FibonacciConfig {
        jitter: 0.5,
        seed: 42,
        ..FibonacciConfig::new(count)
    })
    .unwrap();
    build_sphere_mesh(points, 6_371.0).unwrap()
}

fn triangle_direction(mesh: &SphereMesh, triangle: usize, weights: [f32; 3]) -> Vec3 {
    let cells = mesh.vertex_cells[triangle];
    (mesh.cell_centers[cells[0]] * weights[0]
        + mesh.cell_centers[cells[1]] * weights[1]
        + mesh.cell_centers[cells[2]] * weights[2])
        .normalized()
}

fn assert_reconstructs(mesh: &SphereMesh, direction: Vec3, hint: usize) -> usize {
    let location = mesh.locate_delaunay(direction, hint);
    let sum = location.weights.iter().sum::<f32>();
    assert!((sum - 1.0).abs() <= 2.0e-6);
    assert!(location.weights.iter().all(|weight| *weight >= 0.0));

    let reconstructed = location
        .cells
        .iter()
        .zip(location.weights)
        .fold(Vec3::ZERO, |point, (&cell, weight)| {
            point + mesh.cell_centers[cell] * weight
        })
        .normalized();
    assert!(reconstructed.dot(direction) >= 1.0 - 2.0e-6);
    location.triangle
}

#[test]
fn locates_cell_centers_with_one_hot_weights() {
    let mesh = mesh(256);
    for cell in 0..mesh.cell_count() {
        let direction = mesh.cell_centers[cell].normalized();
        let expected_triangle = mesh
            .vertex_cells
            .iter()
            .enumerate()
            .filter_map(|(triangle, cells)| cells.contains(&cell).then_some(triangle))
            .min()
            .unwrap();
        for hint in [0, mesh.vertex_count() / 2, mesh.vertex_count() - 1] {
            let location = mesh.locate_delaunay(direction, hint);
            assert_eq!(
                location.triangle, expected_triangle,
                "cell {cell}, hint {hint}"
            );
            assert_eq!(location.cells, mesh.vertex_cells[expected_triangle]);
            for (&located_cell, &weight) in location.cells.iter().zip(&location.weights) {
                if located_cell == cell {
                    assert!((weight - 1.0).abs() <= 1.0e-6);
                } else {
                    assert!(weight <= 1.0e-6);
                }
            }
        }
    }
}

#[test]
fn locates_interiors_from_arbitrary_starting_hints() {
    let mesh = mesh(128);
    let target = mesh.vertex_count() / 3;
    let direction = triangle_direction(&mesh, target, [0.2, 0.3, 0.5]);

    for hint in 0..mesh.vertex_count() {
        let location = mesh.locate_delaunay(direction, hint);
        assert_eq!(location.triangle, target, "hint {hint}");
        assert_eq!(location.cells, mesh.vertex_cells[target]);
        for (actual, expected) in location.weights.into_iter().zip([0.2, 0.3, 0.5]) {
            assert!((actual - expected).abs() <= 2.0e-5, "hint {hint}");
        }
    }
    assert_reconstructs(&mesh, direction, mesh.vertex_count() - 1);
}

#[test]
fn shared_boundaries_have_hint_independent_ownership() {
    let mesh = mesh(256);
    let triangle = mesh.vertex_count() / 2;
    let neighbor = mesh.vertex_neighbors[triangle][0];
    let cells = mesh.vertex_cells[triangle];
    let edge_direction = (mesh.cell_centers[cells[0]] + mesh.cell_centers[cells[1]]).normalized();
    let expected = triangle.min(neighbor);

    for hint in [0, triangle, neighbor, mesh.vertex_count() - 1] {
        let location = mesh.locate_delaunay(edge_direction, hint);
        assert_eq!(location.triangle, expected, "hint {hint}");
        assert!(location.weights.contains(&0.0));
        assert_reconstructs(&mesh, edge_direction, hint);
    }
}

#[test]
#[should_panic(expected = "query direction must be finite")]
fn rejects_non_finite_direction() {
    let mesh = mesh(32);
    mesh.locate_delaunay(Vec3::new(f32::NAN, 0.0, 0.0), 0);
}

#[test]
#[should_panic(expected = "query direction must have unit length")]
fn rejects_non_unit_direction() {
    let mesh = mesh(32);
    mesh.locate_delaunay(Vec3::ZERO, 0);
}

#[test]
#[should_panic(expected = "index out of bounds")]
fn rejects_invalid_hint() {
    let mesh = mesh(32);
    mesh.locate_delaunay(Vec3::new(1.0, 0.0, 0.0), mesh.vertex_count());
}

#[test]
fn repeated_queries_are_deterministic() {
    let mesh = mesh(1_000);
    for triangle in (0..mesh.vertex_count()).step_by(37) {
        let direction = triangle_direction(&mesh, triangle, [0.17, 0.29, 0.54]);
        let expected = mesh.locate_delaunay(direction, triangle);
        for hint in [0, mesh.vertex_count() / 2, mesh.vertex_count() - 1] {
            assert_eq!(mesh.locate_delaunay(direction, hint), expected);
        }
    }
}

#[test]
fn handles_representative_large_mesh() {
    let mesh = mesh(65_536);
    for triangle in (0..mesh.vertex_count()).step_by(997) {
        let direction = triangle_direction(&mesh, triangle, [0.23, 0.31, 0.46]);
        assert_eq!(
            assert_reconstructs(&mesh, direction, (triangle + 31_337) % mesh.vertex_count()),
            triangle
        );
    }
}
