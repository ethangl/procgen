use crate::SphereMesh;
use procgen_core::Vec3;
use std::fmt;

const UNIT_SPHERE_TOLERANCE: f32 = 1.0e-4;
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointLocationError {
    NonFiniteDirection,
    DirectionNotOnUnitSphere { length: f32 },
    InvalidTriangleHint { hint: usize, triangle_count: usize },
    WalkFailed,
}

impl fmt::Display for PointLocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteDirection => formatter.write_str("query direction is not finite"),
            Self::DirectionNotOnUnitSphere { length } => write!(
                formatter,
                "query direction has length {length}, expected unit length"
            ),
            Self::InvalidTriangleHint {
                hint,
                triangle_count,
            } => write!(
                formatter,
                "triangle hint {hint} is outside the {triangle_count}-triangle mesh"
            ),
            Self::WalkFailed => formatter.write_str("triangle walk did not converge"),
        }
    }
}

impl std::error::Error for PointLocationError {}

impl SphereMesh {
    /// Locates a unit direction in this mesh's spherical Delaunay triangulation.
    ///
    /// Pass the previous result's `triangle` as `hint` for spatially coherent
    /// queries. `None` starts at triangle zero. The walk crosses triangle
    /// adjacency and never scans every triangle. Directions within
    /// `1e-7` of a shared great-circle edge use the lowest-indexed incident
    /// triangle, independent of the starting hint; weights on that boundary
    /// are clamped to zero and renormalized.
    ///
    /// Non-finite or non-unit directions and out-of-range hints return an
    /// error. `WalkFailed` indicates inconsistent mesh topology or numerical
    /// degeneracy rather than a recoverable miss.
    pub fn locate_delaunay(
        &self,
        direction: Vec3,
        hint: Option<usize>,
    ) -> Result<DelaunayLocation, PointLocationError> {
        if !direction.x.is_finite() || !direction.y.is_finite() || !direction.z.is_finite() {
            return Err(PointLocationError::NonFiniteDirection);
        }
        let length = direction.length();
        if (length - 1.0).abs() > UNIT_SPHERE_TOLERANCE {
            return Err(PointLocationError::DirectionNotOnUnitSphere { length });
        }

        let triangle_count = self.vertex_cells.len();
        let mut triangle = hint.unwrap_or(0);
        if triangle >= triangle_count {
            return Err(PointLocationError::InvalidTriangleHint {
                hint: triangle,
                triangle_count,
            });
        }

        for _ in 0..triangle_count {
            let sides = self.triangle_sides(triangle, direction);
            let crossed_edge = sides
                .iter()
                .enumerate()
                .filter(|(_, side)| **side < -BOUNDARY_EPSILON)
                .min_by(|a, b| a.1.total_cmp(b.1))
                .map(|(edge, _)| edge);
            match crossed_edge {
                Some(edge) => triangle = self.vertex_neighbors[triangle][edge],
                None => {
                    triangle = self.canonical_boundary_triangle(triangle, direction);
                    return Ok(self.location_in_triangle(triangle, direction));
                }
            }
        }

        Err(PointLocationError::WalkFailed)
    }

    fn triangle_sides(&self, triangle: usize, direction: Vec3) -> [f64; 3] {
        let points = self.vertex_cells[triangle].map(|cell| self.cell_centers[cell]);
        std::array::from_fn(|edge| {
            let a = points[edge];
            let b = points[(edge + 1) % 3];
            let cross = cross_f64(a, b);
            dot_f64(cross, direction) / length_f64(cross)
        })
    }

    fn canonical_boundary_triangle(&self, initial: usize, direction: Vec3) -> usize {
        let mut pending = vec![initial];
        let mut candidates = vec![initial];
        let mut best = initial;

        while let Some(triangle) = pending.pop() {
            let sides = self.triangle_sides(triangle, direction);
            for (edge, side) in sides.into_iter().enumerate() {
                if side.abs() > BOUNDARY_EPSILON {
                    continue;
                }
                let neighbor = self.vertex_neighbors[triangle][edge];
                if candidates.contains(&neighbor)
                    || self
                        .triangle_sides(neighbor, direction)
                        .iter()
                        .any(|neighbor_side| *neighbor_side < -BOUNDARY_EPSILON)
                {
                    continue;
                }
                best = best.min(neighbor);
                candidates.push(neighbor);
                pending.push(neighbor);
            }
        }

        best
    }

    fn location_in_triangle(&self, triangle: usize, direction: Vec3) -> DelaunayLocation {
        let cells = self.vertex_cells[triangle];
        let [a, b, c] = cells.map(|cell| self.cell_centers[cell]);
        let sides = self.triangle_sides(triangle, direction);
        let mut weights = [
            dot_f64(cross_f64(b, c), direction),
            dot_f64(cross_f64(c, a), direction),
            dot_f64(cross_f64(a, b), direction),
        ];
        let sum = weights.iter().sum::<f64>();
        for (corner, weight) in weights.iter_mut().enumerate() {
            let opposite_edge = (corner + 1) % 3;
            *weight = if sides[opposite_edge].abs() <= BOUNDARY_EPSILON {
                0.0
            } else {
                (*weight / sum).max(0.0)
            };
        }
        let clamped_sum = weights.iter().sum::<f64>();

        DelaunayLocation {
            triangle,
            cells,
            weights: weights.map(|weight| (weight / clamped_sum) as f32),
        }
    }
}

fn cross_f64(a: Vec3, b: Vec3) -> [f64; 3] {
    let (ax, ay, az) = (f64::from(a.x), f64::from(a.y), f64::from(a.z));
    let (bx, by, bz) = (f64::from(b.x), f64::from(b.y), f64::from(b.z));
    [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
}

fn dot_f64(a: [f64; 3], b: Vec3) -> f64 {
    a[0] * f64::from(b.x) + a[1] * f64::from(b.y) + a[2] * f64::from(b.z)
}

fn length_f64(vector: [f64; 3]) -> f64 {
    dot_array_f64(vector, vector).sqrt()
}

fn dot_array_f64(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
