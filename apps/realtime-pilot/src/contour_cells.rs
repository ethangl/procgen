//! Face-consistent contour cycles and one QEF fit per surface sheet.
use procgen_core::Vec3;

use crate::{
    qef::{Constraint, fit},
    shell::Sample,
};

// Cube corners use x + 2*y + 4*z. Face corners follow their perimeter.
pub(super) const EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];
const FACES: [[usize; 4]; 6] = [
    [0, 1, 3, 2],
    [4, 5, 7, 6],
    [0, 1, 5, 4],
    [2, 3, 7, 6],
    [0, 2, 6, 4],
    [1, 3, 7, 5],
];
pub(super) struct Sheet {
    pub position: Vec3,
    pub edges: Vec<usize>,
}
pub(super) fn crosses(a: f32, b: f32) -> bool {
    (a >= 0.0) != (b >= 0.0)
}

pub(super) fn sheets(samples: [Sample; 8], ids: [usize; 8]) -> Vec<Sheet> {
    let mut neighbors = [[usize::MAX; 2]; 12];
    let mut degree = [0; 12];
    for face in FACES {
        let edges = std::array::from_fn::<_, 4, _>(|i| {
            let a = face[i];
            let b = face[(i + 1) % 4];
            EDGES
                .iter()
                .position(|&(u, v)| (a == u && b == v) || (a == v && b == u))
                .unwrap()
        });
        let cut: Vec<_> = edges
            .into_iter()
            .filter(|&e| {
                let (a, b) = EDGES[e];
                crosses(samples[a].density, samples[b].density)
            })
            .collect();
        let mut connect = |a: usize, b: usize| {
            neighbors[a][degree[a]] = b;
            degree[a] += 1;
            neighbors[b][degree[b]] = a;
            degree[b] += 1;
        };
        if cut.len() == 2 {
            connect(cut[0], cut[1]);
        }
        if cut.len() == 4 {
            // The bilinear saddle selects the connected diagonal. At an exact
            // saddle, shared sample identities break the tie on either face orientation.
            let d = face.map(|i| samples[i].density);
            let scale = d.iter().map(|v| v.abs()).fold(0.0_f32, f32::max);
            let d = d.map(|v| v / scale);
            let determinant = d[0] * d[2] - d[1] * d[3];
            let mut a = [ids[face[0]], ids[face[2]]];
            a.sort();
            let mut b = [ids[face[1]], ids[face[3]]];
            b.sort();
            if determinant > 0.0 || (determinant == 0.0 && a < b) {
                connect(edges[0], edges[1]);
                connect(edges[2], edges[3]);
            } else {
                connect(edges[0], edges[3]);
                connect(edges[1], edges[2]);
            }
        }
    }
    let mut visited = [false; 12];
    let mut result = Vec::new();
    for start in 0..12 {
        if degree[start] == 0 || visited[start] {
            continue;
        }
        let mut edges = Vec::new();
        let mut current = start;
        let mut previous = usize::MAX;
        loop {
            debug_assert_eq!(degree[current], 2);
            visited[current] = true;
            edges.push(current);
            let next = neighbors[current]
                .into_iter()
                .find(|&n| n != previous)
                .unwrap();
            previous = current;
            current = next;
            if current == start {
                break;
            }
            debug_assert!(!visited[current]);
        }
        let constraints: Vec<_> = edges
            .iter()
            .map(|&e| {
                let (a, b) = EDGES[e];
                let t = samples[a].density / (samples[a].density - samples[b].density);
                let position = corner(a) + (corner(b) - corner(a)) * t;
                Constraint {
                    position,
                    normal: density_gradient(samples, position).normalized(),
                }
            })
            .collect();
        result.push(Sheet {
            position: interpolate_positions(samples, fit(&constraints)),
            edges,
        });
    }
    result
}
fn corner(i: usize) -> Vec3 {
    Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32)
}
pub(super) fn interpolate_positions(samples: [Sample; 8], p: Vec3) -> Vec3 {
    samples.iter().enumerate().fold(Vec3::ZERO, |sum, (i, s)| {
        let w = (if i & 1 == 0 { 1.0 - p.x } else { p.x })
            * (if i & 2 == 0 { 1.0 - p.y } else { p.y })
            * (if i & 4 == 0 { 1.0 - p.z } else { p.z });
        sum + s.position * w
    })
}
fn density_gradient(samples: [Sample; 8], p: Vec3) -> Vec3 {
    let mut gradient = Vec3::ZERO;
    for (i, sample) in samples.iter().enumerate() {
        let c = corner(i);
        let w = Vec3::new(
            if c.x == 0.0 { 1.0 - p.x } else { p.x },
            if c.y == 0.0 { 1.0 - p.y } else { p.y },
            if c.z == 0.0 { 1.0 - p.z } else { p.z },
        );
        gradient = gradient
            + Vec3::new(
                (2.0 * c.x - 1.0) * w.y * w.z,
                w.x * (2.0 * c.y - 1.0) * w.z,
                w.x * w.y * (2.0 * c.z - 1.0),
            ) * sample.density;
    }
    gradient
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cycles(samples: [Sample; 8], ids: [usize; 8]) -> Vec<Vec<[usize; 2]>> {
        let mut result: Vec<_> = sheets(samples, ids)
            .into_iter()
            .map(|part| {
                let mut edges: Vec<_> = part
                    .edges
                    .into_iter()
                    .map(|e| {
                        let (a, b) = EDGES[e];
                        let mut edge = [ids[a], ids[b]];
                        edge.sort();
                        edge
                    })
                    .collect();
                edges.sort();
                edges
            })
            .collect();
        result.sort();
        result
    }
    #[test]
    fn all_sign_cases_partition_crossings_and_agree_under_cube_symmetries() {
        let axes = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];
        for mask in 0..256 {
            for varied in [false, true] {
                let samples = std::array::from_fn(|i| Sample {
                    position: corner(i),
                    density: (if mask & (1 << i) == 0 { -1.0 } else { 1.0 })
                        * (if varied { (i + 1) as f32 } else { 1.0 }),
                });
                let reference = cycles(samples, std::array::from_fn(|i| i));
                let mut expected: Vec<_> = EDGES
                    .iter()
                    .filter(|&&(a, b)| crosses(samples[a].density, samples[b].density))
                    .map(|&(a, b)| [a, b])
                    .collect();
                expected.sort();
                let mut actual: Vec<_> = reference.iter().flatten().copied().collect();
                actual.sort();
                assert_eq!(actual, expected);
                for permutation in axes {
                    for flip in 0..8 {
                        let ids = std::array::from_fn(|i| {
                            let j = i ^ flip;
                            (0..3).fold(0, |id, k| id | (((j >> k) & 1) << permutation[k]))
                        });
                        assert_eq!(
                            cycles(ids.map(|i| samples[i]), ids),
                            reference,
                            "mask={mask} varied={varied} permutation={permutation:?} flip={flip}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn disconnected_corner_sheets_have_separate_fits() {
        let samples = std::array::from_fn(|i| Sample {
            position: corner(i),
            density: if i == 0 || i == 7 { 1.0 } else { -1.0 },
        });
        let parts = sheets(samples, std::array::from_fn(|i| i));
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|s| s.edges.len() == 3));
        assert!(parts[0].position.distance_squared(parts[1].position) > 0.1);
    }
    #[test]
    fn planar_and_exact_zero_samples_keep_consistent_roots() {
        for offset in [0.0, 0.25, 1.0] {
            let samples = std::array::from_fn(|i| Sample {
                position: corner(i),
                density: offset - corner(i).x,
            });
            let parts = sheets(samples, std::array::from_fn(|i| i));
            if offset == 1.0 {
                assert!(parts.is_empty());
            } else {
                assert_eq!(parts.len(), 1);
                assert!((parts[0].position - Vec3::new(offset, 0.5, 0.5)).length() < 0.000001);
            }
        }
    }
}
