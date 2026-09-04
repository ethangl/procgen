use procgen_core::Vec3;
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, SphericalDelaunay, TopologyError, build_sphere_mesh};

fn points(count: usize, jitter: f32) -> Vec<Vec3> {
    let config = FibonacciConfig {
        jitter,
        seed: 42,
        ..FibonacciConfig::new(count)
    };
    fibonacci_sphere(config).unwrap()
}

fn assert_outward(hull: &SphericalDelaunay) {
    for (triangle_index, triangle) in hull.triangles().iter().enumerate() {
        let [a, b, c] = triangle.map(|point| hull.points()[point]);
        let normal = (b - a).cross(c - a);
        let centroid = (a + b + c) * (1.0 / 3.0);
        assert!(normal.dot(centroid) > 0.0, "triangle {triangle_index}");
    }
}

#[test]
fn validates_inputs() {
    assert_eq!(
        SphericalDelaunay::build(vec![Vec3::ZERO; 3]).unwrap_err(),
        TopologyError::TooFewPoints
    );
    assert_eq!(
        SphericalDelaunay::build(vec![Vec3::ZERO; 4]).unwrap_err(),
        TopologyError::PointNotOnUnitSphere {
            index: 0,
            length: 0.0
        }
    );
}

#[test]
fn delaunay_is_a_closed_outward_triangulation() {
    let count = 1_000;
    let hull = SphericalDelaunay::build(points(count, 0.5)).unwrap();

    assert_eq!(hull.triangle_count(), 2 * count - 4);
    assert_eq!(hull.opposite_half_edges().len(), hull.triangle_count() * 3);

    let mut used = vec![false; count];
    assert_outward(&hull);
    for triangle in hull.triangles() {
        for &point in triangle {
            used[point] = true;
        }
    }
    assert!(used.into_iter().all(|is_used| is_used));

    for (edge, &opposite) in hull.opposite_half_edges().iter().enumerate() {
        assert_eq!(hull.opposite_half_edges()[opposite], edge);
    }

    let unique_edges: Vec<_> = hull.unique_edges().collect();
    assert_eq!(unique_edges.len(), 3 * count - 6);
    for edge in unique_edges {
        let opposite = hull.opposite_half_edges()[edge];
        assert_eq!(hull.edge_origin(edge), hull.edge_destination(opposite));
        assert_eq!(hull.edge_destination(edge), hull.edge_origin(opposite));
    }
}

#[test]
fn input_order_does_not_determine_face_winding() {
    let mut reversed = points(128, 0.5);
    reversed.reverse();
    let hull = SphericalDelaunay::build(reversed).unwrap();

    assert_outward(&hull);
}

#[test]
fn voronoi_has_complete_symmetric_topology() {
    let count = 1_000;
    let mesh = build_sphere_mesh(points(count, 0.5), 1.0).unwrap();

    assert_eq!(mesh.cell_count(), count);
    assert_eq!(mesh.vertex_count(), 2 * count - 4);
    assert_eq!(mesh.edge_count(), 3 * count - 6);
    assert_eq!(mesh.cell_offsets.len(), count + 1);
    assert_eq!(mesh.corners.len(), 6 * count - 12);

    for cell in 0..mesh.cell_count() {
        let corners = mesh.cell_corners(cell);
        assert!(corners.len() >= 3);
        for corner in corners {
            let edge = mesh.edges[corner.edge];
            assert!(edge.vertices.contains(&corner.vertex));
            assert!(edge.cells.contains(&cell));
            assert!(edge.cells.contains(&corner.neighbor));
            assert!(
                mesh.cell_corners(corner.neighbor)
                    .iter()
                    .any(|neighbor| neighbor.neighbor == cell)
            );
        }
    }
}

#[test]
fn vertices_and_areas_cover_the_requested_sphere() {
    let radius = 6_371.0;
    let mesh = build_sphere_mesh(points(1_000, 0.5), radius).unwrap();

    for vertex in &mesh.vertices {
        assert!((vertex.length() - radius).abs() < radius * 1.0e-5);
    }
    let actual: f32 = mesh.cell_areas.iter().sum();
    let expected = 4.0 * std::f32::consts::PI * radius * radius;
    assert!((actual - expected).abs() < expected * 1.0e-4);
}

#[test]
fn handles_reference_scale() {
    let mesh: SphereMesh = build_sphere_mesh(points(20_400, 0.5), 1.0).unwrap();
    assert_eq!(mesh.cell_count(), 20_400);
    assert_eq!(mesh.vertex_count(), 40_796);
}
