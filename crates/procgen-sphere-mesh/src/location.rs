use crate::{SphereMesh, UNIT_SPHERE_TOLERANCE};
use procgen_core::Vec3;

const BOUNDARY_EPSILON: f64 = 1.0e-7;

/// A direction located in the spherical Delaunay triangulation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DelaunayLocation {
    /// Index into [`SphereMesh::vertex_cells`] and [`SphereMesh::vertex_neighbors`].
    pub triangle: usize,
    /// The three cells at the triangle's vertices, in outward-facing order.
    pub cells: [usize; 3],
    /// Planar barycentric weights corresponding to `cells`.
    ///
    /// The weighted cell-center position, normalized back onto the sphere,
    /// reconstructs the query direction. These weights therefore interpolate
    /// continuous per-cell control fields across Delaunay edges.
    pub weights: [f32; 3],
}

#[derive(Clone, Copy)]
struct EdgeSides {
    /// Scalar triple products in directed triangle-edge order.
    numerators: [f64; 3],
    /// Signed great-circle distances in directed triangle-edge order.
    distances: [f64; 3],
}

impl EdgeSides {
    fn new(points: [[f64; 3]; 3], direction: [f64; 3]) -> Self {
        let crosses = std::array::from_fn(|edge| cross(points[edge], points[(edge + 1) % 3]));
        let numerators = crosses.map(|edge_normal| dot(edge_normal, direction));
        let distances = std::array::from_fn(|edge| numerators[edge] / length(crosses[edge]));
        Self {
            numerators,
            distances,
        }
    }

    fn exit_edge(self) -> Option<usize> {
        self.distances
            .iter()
            .enumerate()
            .filter(|(_, distance)| **distance < -BOUNDARY_EPSILON)
            .min_by(|a, b| a.1.total_cmp(b.1))
            .map(|(edge, _)| edge)
    }

    fn touches_boundary(self) -> bool {
        (0..3).any(|edge| self.on_edge(edge))
    }

    fn on_edge(self, edge: usize) -> bool {
        self.distances[edge].abs() <= BOUNDARY_EPSILON
    }

    fn weights(self) -> [f32; 3] {
        // Directed edges are [a-b, b-c, c-a], so the side opposite each
        // barycentric corner is [b-c, c-a, a-b].
        let weights = std::array::from_fn(|corner| {
            let opposite_edge = (corner + 1) % 3;
            if self.on_edge(opposite_edge) {
                0.0
            } else {
                self.numerators[opposite_edge]
            }
        });
        let sum = weights.iter().sum::<f64>();
        weights.map(|weight| (weight / sum) as f32)
    }
}

impl SphereMesh {
    /// Locates a unit direction in this mesh's spherical Delaunay triangulation.
    ///
    /// Pass the previous result's `triangle` as `hint` for spatially coherent
    /// queries, and pass triangle zero for the first query. The walk crosses
    /// triangle adjacency and never scans every triangle. Directions within
    /// the internal tolerance of a shared great-circle edge use the
    /// lowest-indexed incident triangle, independent of the starting hint;
    /// weights on that boundary are clamped to zero and renormalized.
    ///
    /// The direction must be finite and unit length, and the hint must index an
    /// existing triangle. Violating either caller contract panics. Walk
    /// exhaustion also panics because it indicates inconsistent mesh topology
    /// or numerical degeneracy rather than a recoverable miss.
    pub fn locate_delaunay(&self, direction: Vec3, hint: usize) -> DelaunayLocation {
        assert!(direction.is_finite(), "query direction must be finite");
        let direction_length = direction.length();
        assert!(
            (direction_length - 1.0).abs() <= UNIT_SPHERE_TOLERANCE,
            "query direction must have unit length"
        );
        let direction = to_f64(direction);
        let mut triangle = hint;

        for _ in 0..self.vertex_count() {
            let mut sides = self.edge_sides(triangle, direction);
            if let Some(edge) = sides.exit_edge() {
                triangle = self.vertex_neighbors[triangle][edge];
                continue;
            }

            if sides.touches_boundary() {
                (triangle, sides) = self.canonical_boundary_triangle(triangle, sides, direction);
            }
            return DelaunayLocation {
                triangle,
                cells: self.vertex_cells[triangle],
                weights: sides.weights(),
            };
        }

        panic!("Delaunay triangle walk did not converge");
    }

    fn edge_sides(&self, triangle: usize, direction: [f64; 3]) -> EdgeSides {
        let points = self.vertex_cells[triangle].map(|cell| to_f64(self.cell_centers[cell]));
        EdgeSides::new(points, direction)
    }

    fn canonical_boundary_triangle(
        &self,
        initial: usize,
        initial_sides: EdgeSides,
        direction: [f64; 3],
    ) -> (usize, EdgeSides) {
        let mut component = vec![(initial, initial_sides)];
        let mut cursor = 0;

        while cursor < component.len() {
            let (triangle, sides) = component[cursor];
            cursor += 1;
            for edge in 0..3 {
                if !sides.on_edge(edge) {
                    continue;
                }
                let neighbor = self.vertex_neighbors[triangle][edge];
                if component
                    .iter()
                    .any(|(candidate, _)| *candidate == neighbor)
                {
                    continue;
                }
                let neighbor_sides = self.edge_sides(neighbor, direction);
                if neighbor_sides.exit_edge().is_some() {
                    continue;
                }
                component.push((neighbor, neighbor_sides));
            }
        }

        component
            .into_iter()
            .min_by_key(|(triangle, _)| *triangle)
            .expect("boundary component contains the initial triangle")
    }
}

fn to_f64(vector: Vec3) -> [f64; 3] {
    [
        f64::from(vector.x),
        f64::from(vector.y),
        f64::from(vector.z),
    ]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(vector: [f64; 3]) -> f64 {
    dot(vector, vector).sqrt()
}
