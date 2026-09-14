//! Advance toward a target partition through small, closed replacement groups.
use crate::voxel_neighborhood::{bounds, contains};
use crate::voxel_selection::distance_squared;
use crate::{VoxelChunkAddress, VoxelCoverage, VoxelCoverageError, VoxelPosition};
use std::collections::BTreeSet;

impl VoxelCoverage {
    /// Perform one split or coarsening, including the balance required around it.
    /// The caller publishes this coverage before requesting the next step.
    pub fn step_toward(
        &self,
        target: &Self,
        camera: VoxelPosition,
    ) -> Result<Self, VoxelCoverageError> {
        let targets: BTreeSet<_> = target.leaves().iter().copied().collect();
        let ancestors: BTreeSet<_> = target
            .leaves()
            .iter()
            .flat_map(|a| std::iter::successors(a.parent(), |a| a.parent()))
            .collect();
        let split = self
            .leaves()
            .iter()
            .copied()
            .filter(|a| ancestors.contains(a))
            .min_by(|a, b| {
                distance_squared(*a, camera)
                    .total_cmp(&distance_squared(*b, camera))
                    .then_with(|| a.cmp(b))
            });
        if let Some(a) = split {
            let mut leaves: Vec<_> = self.leaves().iter().copied().filter(|&b| b != a).collect();
            leaves.extend(
                a.children()
                    .expect("ancestor has children")
                    .into_iter()
                    .filter(|&child| overlaps_target(child, &targets, &ancestors)),
            );
            let balanced = Self::balanced(leaves, crate::MAX_VOXEL_COVERAGE_LEAVES)?;
            return Self::new(
                balanced
                    .leaves()
                    .iter()
                    .copied()
                    .filter(|&a| overlaps_target(a, &targets, &ancestors))
                    .collect(),
            );
        }
        let mut coarse: Vec<_> = self
            .leaves()
            .iter()
            .filter_map(|a| a.parent())
            .filter(|a| {
                std::iter::successors(Some(*a), |a| a.parent()).any(|a| targets.contains(&a))
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        // Collapse only a sibling level whose outside neighbors permit 2:1.
        // Testing metadata avoids repeatedly rebuilding an inadmissible octree.
        coarse.sort_by(|a, b| {
            a.lod()
                .cmp(&b.lod())
                .then_with(|| distance_squared(*b, camera).total_cmp(&distance_squared(*a, camera)))
                .then_with(|| a.cmp(b))
        });
        for a in coarse {
            if self.leaves().iter().any(|&b| {
                (encloses(a, b) && b.lod() + 1 != a.lod())
                    || (!encloses(a, b)
                        && crate::voxel_neighborhood::touching(a, b)
                        && b.lod() + 1 < a.lod())
            }) {
                continue;
            }
            let mut leaves: Vec<_> = self
                .leaves()
                .iter()
                .copied()
                .filter(|&b| !encloses(a, b))
                .collect();
            leaves.push(a);
            return Self::new(leaves);
        }
        // A target may cull empty leaves without replacing their volume.
        Self::new(
            self.leaves()
                .iter()
                .copied()
                .filter(|a| target.leaves().contains(a))
                .collect(),
        )
    }
}
fn overlaps_target(
    a: VoxelChunkAddress,
    targets: &BTreeSet<VoxelChunkAddress>,
    ancestors: &BTreeSet<VoxelChunkAddress>,
) -> bool {
    ancestors.contains(&a)
        || std::iter::successors(Some(a), |a| a.parent()).any(|a| targets.contains(&a))
}
fn encloses(a: VoxelChunkAddress, b: VoxelChunkAddress) -> bool {
    let (lo, hi) = bounds(b);
    contains(a, lo) && contains(a, hi)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn planet_route_reaches_meter_detail_without_repeating_a_partition() {
        let field = crate::PlanetDesignConfig::starter(42).validate().unwrap();
        let ground = VoxelPosition {
            x_m: (field.config().radius_m + field.elevation_m(procgen_core::Vec3::X, 0.0).unwrap())
                as i32,
            y_m: 0,
            z_m: 0,
        };
        let mut current = VoxelCoverage::new(vec![VoxelChunkAddress::root()]).unwrap();
        for camera in [
            ground,
            VoxelPosition { y_m: 64, ..ground },
            VoxelPosition {
                x_m: field.config().radius_m as i32 * 3,
                y_m: 0,
                z_m: 0,
            },
        ] {
            let target = crate::select_voxel_coverage(&field, camera, 512, 4096).unwrap();
            let mut seen = BTreeSet::new();
            while current.leaves() != target.leaves() {
                assert!(
                    seen.insert(current.leaves().to_vec()),
                    "coverage step repeated a partition after {} steps, {} leaves vs target {}",
                    seen.len(),
                    current.leaves().len(),
                    target.leaves().len()
                );
                current = current.step_toward(&target, camera).unwrap();
                assert!(seen.len() < 4096);
            }
            println!(
                "coverage route: {} steps to {} leaves",
                seen.len(),
                current.leaves().len()
            );
        }
    }
    #[test]
    fn progressive_refinement_and_coarsening_reach_closed_target() {
        let camera = VoxelPosition {
            x_m: 3,
            y_m: 5,
            z_m: 7,
        };
        let root = VoxelChunkAddress::containing(camera, 3).unwrap();
        let initial = VoxelCoverage::new(vec![root]).unwrap();
        let mut leaves = root.children().unwrap().to_vec();
        let refined = leaves.remove(0).children().unwrap();
        leaves.extend(refined);
        let target = VoxelCoverage::balanced(leaves, 256).unwrap();
        let mut current = initial.clone();
        for goal in [&target, &initial] {
            for _ in 0..100 {
                if current.leaves() == goal.leaves() {
                    break;
                }
                let next = current.step_toward(goal, camera).unwrap();
                assert_ne!(next.leaves(), current.leaves());
                next.validate().unwrap();
                current = next;
            }
            assert_eq!(current.leaves(), goal.leaves());
        }
    }
}
