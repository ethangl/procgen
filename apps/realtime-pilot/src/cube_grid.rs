//! A whole-cube quad grid whose six faces share one vertex per edge and corner
//! position. Preview meshes need that sharing: two faces meeting at an edge must
//! evaluate the field at the identical direction, or the seam splits open.
use std::collections::BTreeMap;

use procgen_core::Vec3;
use procgen_cubesphere::{CubeFace, equiangular_tangent};

/// Signed integer cube coordinates identify a sample independently of the requesting face.
fn cube_key(face: CubeFace, x: usize, y: usize, side: usize) -> [i32; 3] {
    let frame = face.frame();
    let p = frame.normal * side as f32 + frame.u_axis * (2 * x) as f32 - frame.u_axis * side as f32
        + frame.v_axis * (2 * y) as f32
        - frame.v_axis * side as f32;
    [p.x as i32, p.y as i32, p.z as i32]
}

/// Unit directions plus the quads indexing them, `side` quads across each face.
/// Quad corners are ordered (0,0), (1,0), (0,1), (1,1) in face coordinates.
pub fn face_grid(side: usize) -> (Vec<Vec3>, Vec<[usize; 4]>) {
    let mut ids = BTreeMap::new();
    let mut directions = Vec::new();
    let mut quads = Vec::new();
    for face in CubeFace::ALL {
        let mut face_ids = Vec::with_capacity((side + 1).pow(2));
        for y in 0..=side {
            for x in 0..=side {
                let key = cube_key(face, x, y, side);
                let id = *ids.entry(key).or_insert_with(|| {
                    let id = directions.len();
                    let p = key.map(|v| equiangular_tangent(v as f32 / side as f32));
                    directions.push(Vec3::new(p[0], p[1], p[2]).normalized());
                    id
                });
                face_ids.push(id);
            }
        }
        for y in 0..side {
            for x in 0..side {
                let i = y * (side + 1) + x;
                quads.push([
                    face_ids[i],
                    face_ids[i + 1],
                    face_ids[i + side + 1],
                    face_ids[i + side + 2],
                ]);
            }
        }
    }
    (directions, quads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_cubesphere::{FaceCoordinates, face_to_direction};
    #[test]
    fn every_edge_and_corner_has_one_address_and_matches_face_mapping() {
        let n = 8;
        let mut requests: BTreeMap<[i32; 3], Vec<Vec3>> = BTreeMap::new();
        for face in CubeFace::ALL {
            for y in 0..=n {
                for x in 0..=n {
                    let direction = face_to_direction(FaceCoordinates {
                        face,
                        u: -1.0 + 2.0 * x as f32 / n as f32,
                        v: -1.0 + 2.0 * y as f32 / n as f32,
                    })
                    .unwrap();
                    requests
                        .entry(cube_key(face, x, y, n))
                        .or_default()
                        .push(direction);
                }
            }
        }
        assert_eq!(requests.len(), 6 * n * n + 2);
        assert_eq!(requests.values().filter(|v| v.len() == 3).count(), 8);
        assert_eq!(
            requests.values().filter(|v| v.len() == 2).count(),
            12 * (n - 1)
        );
        for points in requests.values() {
            for p in points {
                assert_eq!(*p, points[0]);
            }
        }
        let (directions, quads) = face_grid(n);
        assert!(
            directions
                .iter()
                .all(|d| requests.values().any(|p| p[0] == *d))
        );
        let mut incidence = vec![0; directions.len()];
        for q in quads {
            for c in q {
                incidence[c] += 1;
            }
        }
        assert_eq!(incidence.iter().filter(|&&n| n == 3).count(), 8);
        assert!(incidence.iter().all(|&n| n == 3 || n == 4));
    }
}
