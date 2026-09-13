//! Bounded surface-shell traversal. Distant leaves need only address metadata.
use crate::{PlanetDesignField, VOXEL_DENSITY_LIMIT_M, VoxelChunkAddress, VoxelPosition};
use std::{
    cmp::Ordering,
    collections::{BTreeSet, BinaryHeap},
};

#[derive(Clone, Copy)]
struct Candidate {
    address: VoxelChunkAddress,
    distance_squared: f32,
}
impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .distance_squared
            .total_cmp(&self.distance_squared)
            .then_with(|| other.address.cmp(&self.address))
    }
}

/// Distances use integer camera-relative bounds before conversion. This also
/// accepts cameras outside the root without narrowing their world position.
pub(crate) fn distance_squared(address: VoxelChunkAddress, camera: VoxelPosition) -> f32 {
    let origin = address.origin();
    let span = address.span_m();
    // Subtraction through f32 would lose meter precision near large planets;
    // unsigned differences also cover cameras at the limits of the i32 domain.
    [
        (origin.x_m, camera.x_m),
        (origin.y_m, camera.y_m),
        (origin.z_m, camera.z_m),
    ]
    .into_iter()
    .map(|(lo, p)| {
        let d = if p < lo {
            lo.abs_diff(p)
        } else if p > lo + span {
            p.abs_diff(lo + span)
        } else {
            0
        } as f32;
        d * d
    })
    .sum()
}

/// Only the global height envelope proves emptiness. Equal corner signs do
/// not: high frequency terrain can cross the box between sampled corners.
fn intersects_shell(address: VoxelChunkAddress, field: &PlanetDesignField) -> bool {
    let origin = address.origin();
    let halo = address.spacing_m();
    let span = address.span_m();
    let mut near_squared = 0.0;
    let mut far_squared = 0.0;
    for value in [origin.x_m, origin.y_m, origin.z_m] {
        let lo = (value - halo) as f32;
        let hi = (value + span + halo) as f32;
        let near = if lo > 0.0 {
            lo
        } else if hi < 0.0 {
            -hi
        } else {
            0.0
        };
        let far = lo.abs().max(hi.abs());
        near_squared += near * near;
        far_squared += far * far;
    }
    // 16 m exceeds accumulated f32 box/radius rounding at the largest root
    // bounds; the independent f64 shell tests cover both grazing boundaries.
    let envelope = field.config().height_limit_m + VOXEL_DENSITY_LIMIT_M + 16.0;
    let inner = field.config().radius_m - envelope;
    let outer = field.config().radius_m + envelope;
    near_squared <= outer * outer && far_squared >= inner * inner
}

pub(crate) fn select_leaves(
    field: &PlanetDesignField,
    camera: VoxelPosition,
    max_leaves: usize,
) -> Vec<VoxelChunkAddress> {
    let root = VoxelChunkAddress::root();
    let mut leaves = BTreeSet::from([root]);
    let mut queue = BinaryHeap::from([Candidate {
        address: root,
        distance_squared: distance_squared(root, camera),
    }]);
    while let Some(candidate) = queue.pop() {
        let address = candidate.address;
        // Refine within two chunk spans, stopping at exact one-meter cells.
        if candidate.distance_squared > (2.0 * address.span_m() as f32).powi(2) {
            continue;
        }
        let Some(children) = address.children() else {
            continue;
        };
        let children: Vec<_> = children
            .into_iter()
            .filter(|&child| intersects_shell(child, field))
            .collect();
        if leaves.len() - 1 + children.len() > max_leaves {
            continue;
        }
        leaves.remove(&address);
        for child in children {
            leaves.insert(child);
            queue.push(Candidate {
                address: child,
                distance_squared: distance_squared(child, camera),
            });
        }
    }
    leaves.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkIndex, PlanetDesignConfig};

    #[test]
    fn bounded_leaves_cover_surface_without_ancestor_overlap_and_revisit_exactly() {
        for radius in [100_000.0, 4_900_000.0, 8_000_000.0] {
            let mut config = PlanetDesignConfig::starter(42);
            config.radius_m = radius;
            let field = config.validate().unwrap();
            let camera = VoxelPosition {
                x_m: radius as i32,
                y_m: 7,
                z_m: -9,
            };
            let leaves = select_leaves(&field, camera, 256);
            assert!(leaves.len() <= 256);
            assert!(
                leaves.iter().any(|a| a.lod() == 0),
                "nearest leaves must reach meter cells"
            );
            let selected: BTreeSet<_> = leaves.iter().copied().collect();
            for &leaf in &leaves {
                for ancestor in std::iter::successors(leaf.parent(), |a| a.parent()) {
                    assert!(!selected.contains(&ancestor));
                }
            }
            // Every actual terrain probe has exactly one selected ancestor.
            for direction in crate::test_support::positions() {
                let direction = direction.normalized();
                let p = direction * (radius + field.height(direction, 0.0));
                let point = VoxelPosition {
                    x_m: p.x.round() as i32,
                    y_m: p.y.round() as i32,
                    z_m: p.z.round() as i32,
                };
                let fine = VoxelChunkAddress::containing(point, 0).unwrap();
                assert_eq!(
                    std::iter::successors(Some(fine), |a| a.parent())
                        .filter(|a| selected.contains(a))
                        .count(),
                    1
                );
            }
            let distant = select_leaves(
                &field,
                VoxelPosition {
                    x_m: i32::MAX,
                    y_m: 0,
                    z_m: 0,
                },
                256,
            );
            assert_eq!(distant, vec![VoxelChunkAddress::root()]);
            assert_eq!(leaves, select_leaves(&field, camera, 256));
        }
    }

    #[test]
    fn shell_culling_keeps_grazing_boxes_and_halos() {
        for radius in [100_000.0, 8_000_000.0] {
            let mut config = PlanetDesignConfig::starter(42);
            config.radius_m = radius;
            let field = config.validate().unwrap();
            for lod in [0, 4, 10, 18] {
                for sign in [-1.0, 1.0] {
                    for offset in [-12_020.0, -12_000.0, 12_000.0, 12_020.0] {
                        let point = VoxelPosition {
                            x_m: (sign * (radius + offset)) as i32,
                            y_m: 0,
                            z_m: 0,
                        };
                        let a = VoxelChunkAddress::containing(point, lod).unwrap();
                        let origin = a.origin();
                        let h = a.spacing_m() as f64;
                        let s = a.span_m() as f64;
                        let mut near = 0.0;
                        let mut far = 0.0;
                        for v in [origin.x_m, origin.y_m, origin.z_m] {
                            let lo = v as f64 - h;
                            let hi = v as f64 + s + h;
                            let d = if lo > 0.0 {
                                lo
                            } else if hi < 0.0 {
                                -hi
                            } else {
                                0.0
                            };
                            near += d * d;
                            far += lo.abs().max(hi.abs()).powi(2);
                        }
                        if near.sqrt() <= radius as f64 + 12_004.0
                            && far.sqrt() >= radius as f64 - 12_004.0
                        {
                            assert!(intersects_shell(a, &field));
                        }
                    }
                }
            }
            assert!(!intersects_shell(
                VoxelChunkAddress::containing(
                    VoxelPosition {
                        x_m: 0,
                        y_m: 0,
                        z_m: 0
                    },
                    0
                )
                .unwrap(),
                &field
            ));
            assert!(!intersects_shell(
                VoxelChunkAddress::new(0, ChunkIndex { x: 0, y: 0, z: 0 }).unwrap(),
                &field
            ));
        }
    }
}
