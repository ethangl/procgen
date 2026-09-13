use procgen_core::Vec3;

pub(crate) struct Constraint {
    pub position: Vec3,
    pub normal: Vec3,
}

/// Pseudoinverse centered on the edge-intersection mass point. An outside-cell
/// minimizer is explicitly replaced by the mass point, never component-clamped.
pub(crate) fn fit(constraints: &[Constraint]) -> Vec3 {
    assert!(!constraints.is_empty());
    let mass = constraints
        .iter()
        .fold(Vec3::ZERO, |sum, c| sum + c.position)
        * (constraints.len() as f32).recip();
    let mut matrix = [[0.0; 3]; 3];
    let mut rhs = [0.0; 3];
    for constraint in constraints {
        let n = constraint.normal;
        let row = [n.x, n.y, n.z];
        let distance = n.dot(constraint.position - mass);
        for i in 0..3 {
            rhs[i] += row[i] * distance;
            for j in 0..3 {
                matrix[i][j] += row[i] * row[j];
            }
        }
    }
    let mut vectors = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    // Fixed Jacobi sweeps; sqrt and algebra only, no angle function deciding rank.
    for _ in 0..12 {
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            let off = matrix[p][q];
            if off == 0.0 {
                continue;
            }
            let tau = (matrix[q][q] - matrix[p][p]) / (2.0 * off);
            let t = if tau >= 0.0 { 1.0 } else { -1.0 } / (tau.abs() + (1.0 + tau * tau).sqrt());
            let c = (1.0 + t * t).sqrt().recip();
            let s = t * c;
            let pp = matrix[p][p];
            let qq = matrix[q][q];
            matrix[p][p] = pp - t * off;
            matrix[q][q] = qq + t * off;
            matrix[p][q] = 0.0;
            matrix[q][p] = 0.0;
            for k in 0..3 {
                if k != p && k != q {
                    let kp = matrix[k][p];
                    let kq = matrix[k][q];
                    matrix[k][p] = c * kp - s * kq;
                    matrix[p][k] = matrix[k][p];
                    matrix[k][q] = s * kp + c * kq;
                    matrix[q][k] = matrix[k][q];
                }
                let kp = vectors[k][p];
                let kq = vectors[k][q];
                vectors[k][p] = c * kp - s * kq;
                vectors[k][q] = s * kp + c * kq;
            }
        }
    }
    let largest = matrix[0][0].max(matrix[1][1]).max(matrix[2][2]);
    let mut delta = Vec3::ZERO;
    for (i, row) in matrix.iter().enumerate() {
        // Discard directions below 1e-5 of the strongest normal-equation eigenvalue.
        if row[i] <= largest * 0.00001 {
            continue;
        }
        let axis = Vec3::new(vectors[0][i], vectors[1][i], vectors[2][i]);
        delta = delta + axis * (axis.dot(Vec3::new(rhs[0], rhs[1], rhs[2])) / row[i]);
    }
    let candidate = mass + delta;
    if [candidate.x, candidate.y, candidate.z]
        .iter()
        .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    {
        candidate
    } else {
        mass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn planar_nullspace_and_zero_gradients_choose_mass_point() {
        let positions = [
            Vec3::new(0.25, 0.0, 0.0),
            Vec3::new(0.25, 1.0, 0.0),
            Vec3::new(0.25, 0.0, 1.0),
            Vec3::new(0.25, 1.0, 1.0),
        ];
        for normal in [Vec3::X, Vec3::ZERO] {
            assert_eq!(
                fit(&positions.map(|position| Constraint { position, normal })),
                Vec3::new(0.25, 0.5, 0.5)
            );
        }
    }
    #[test]
    fn sharp_corner_is_recovered_and_nearly_parallel_data_stays_bounded() {
        let corner = Vec3::new(0.2, 0.3, 0.4);
        let constraints = [
            Constraint {
                position: Vec3::new(0.2, 0.8, 0.8),
                normal: Vec3::X,
            },
            Constraint {
                position: Vec3::new(0.8, 0.3, 0.8),
                normal: Vec3::Y,
            },
            Constraint {
                position: Vec3::new(0.8, 0.8, 0.4),
                normal: Vec3::Z,
            },
        ];
        assert!((fit(&constraints) - corner).length() < 0.000001);
        let parallel = [
            Constraint {
                position: Vec3::new(0.2, 0.0, 0.0),
                normal: Vec3::X,
            },
            Constraint {
                position: Vec3::new(0.2, 1.0, 1.0),
                normal: Vec3::new(1.0, 0.000001, 0.0).normalized(),
            },
        ];
        assert!((fit(&parallel) - Vec3::new(0.2, 0.5, 0.5)).length() < 0.00001);
    }
    #[test]
    fn rotated_planes_recover_their_known_intersection() {
        let corner = Vec3::new(0.2, 0.3, 0.4);
        let constraints = [
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(-2.0, 1.0, 1.0),
            Vec3::new(1.0, -1.0, 1.0),
        ]
        .map(|n| {
            let normal = n.normalized();
            Constraint {
                position: corner + normal.cross(Vec3::Y) * 0.1,
                normal,
            }
        });
        assert!((fit(&constraints) - corner).length() < 0.000001);
    }

    #[test]
    fn outside_minimizer_uses_mass_point_not_clamp() {
        let constraints = [
            Constraint {
                position: Vec3::new(0.0, 0.2, 0.5),
                normal: Vec3::Y,
            },
            Constraint {
                position: Vec3::new(1.0, 0.8, 0.5),
                normal: Vec3::new(1.0, 1.0, 0.0).normalized(),
            },
        ];
        assert_eq!(fit(&constraints), Vec3::new(0.5, 0.5, 0.5));
    }
}
