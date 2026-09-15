//! Canonical transition interpolation and extraction from a bounded topology plan.
use crate::voxel_chunk_mesh::{build_regular_mesh, edge_vertex};
use crate::voxel_mesh_topology::{NONE, SIMPLEX_RINGS};
use crate::{VoxelChunkMesh, VoxelMeshError, VoxelMeshVertex, VoxelTransitionPlan, VoxelVolume};
use rayon::prelude::*;

pub struct VoxelTransitionSamples {
    values: Vec<f32>,
}
impl VoxelTransitionSamples {
    pub fn new(values: Vec<f32>, plan: &VoxelTransitionPlan) -> Result<Self, VoxelMeshError> {
        let result = Self { values };
        result.validate(plan)?;
        Ok(result)
    }
    pub fn validate(&self, plan: &VoxelTransitionPlan) -> Result<(), VoxelMeshError> {
        if self.values.len() != plan.sample_points().len()
            || self.values.iter().any(|v| !v.is_finite())
        {
            return Err(VoxelMeshError::TransitionSamples);
        }
        Ok(())
    }
    pub fn values(&self) -> &[f32] {
        &self.values
    }
}
pub fn build_voxel_regular_mesh(
    volume: &VoxelVolume,
    plan: &VoxelTransitionPlan,
) -> Result<VoxelChunkMesh, VoxelMeshError> {
    plan.validate()?;
    if volume.address() != plan.key().address() {
        return Err(VoxelMeshError::TransitionAddress);
    }
    build_regular_mesh(volume, &plan.blocked)
}
pub fn build_voxel_transition_mesh(
    plan: &VoxelTransitionPlan,
    samples: &VoxelTransitionSamples,
) -> Result<VoxelChunkMesh, VoxelMeshError> {
    plan.validate()?;
    samples.validate(plan)?;
    let values: Vec<f32> = plan
        .nodes()
        .iter()
        .map(|node| {
            node.samples[..node.sample_count as usize]
                .iter()
                .map(|&i| samples.values[i as usize] / node.sample_count as f32)
                .sum()
        })
        .collect();
    let polygons: Vec<_> = plan
        .tetrahedra()
        .par_iter()
        .map(|&tet| {
            let case = tet.iter().enumerate().fold(0, |mask, (i, &node)| {
                mask | (usize::from(values[node as usize] >= 0.0) << i)
            });
            let mut roots = [[NONE; 2]; 4];
            let mut count = 0;
            for &edge in &SIMPLEX_RINGS[case] {
                if edge == NONE {
                    break;
                }
                let (a, b) = (tet[(edge / 4) as usize], tet[(edge % 4) as usize]);
                let root = if values[a as usize] == 0.0 {
                    [a, a]
                } else if values[b as usize] == 0.0 {
                    [b, b]
                } else {
                    [a.min(b), a.max(b)]
                };
                if !roots[..count].contains(&root) {
                    roots[count] = root;
                    count += 1;
                }
            }
            let vertex = |root: [u32; 2]| {
                let [a, b] = root.map(|i| plan.nodes()[i as usize].position_m);
                if root[0] == root[1] {
                    VoxelMeshVertex {
                        anchor_m: a,
                        offset_m: [0.0; 4],
                        normal: [0.0; 4],
                    }
                } else {
                    edge_vertex(
                        [a[0], a[1], a[2]],
                        [b[0], b[1], b[2]],
                        values[root[0] as usize],
                        values[root[1] as usize],
                    )
                }
            };
            let mut vertices = Vec::with_capacity(6);
            for i in 1..count.saturating_sub(1) {
                vertices.extend([roots[0], roots[i], roots[i + 1]].map(vertex));
            }
            vertices
        })
        .collect();
    let vertices: Vec<_> = polygons.into_iter().flatten().collect();
    let indices = (0..vertices.len() as u32).collect();
    VoxelChunkMesh::from_parts(plan.key().address(), vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{VoxelChunkAddress, VoxelCoverage, VoxelPosition};
    use std::collections::BTreeMap;

    #[derive(Clone, Copy, PartialEq, Debug)]
    struct Point([f64; 3]);
    impl Eq for Point {}
    impl PartialOrd for Point {
        fn partial_cmp(&self, rhs: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(rhs))
        }
    }
    impl Ord for Point {
        fn cmp(&self, rhs: &Self) -> std::cmp::Ordering {
            self.0
                .iter()
                .zip(rhs.0)
                .map(|(a, b)| a.total_cmp(&b))
                .find(|o| !o.is_eq())
                .unwrap_or(std::cmp::Ordering::Equal)
        }
    }

    fn partition(refine: usize) -> VoxelCoverage {
        let root = VoxelChunkAddress::containing(
            VoxelPosition {
                x_m: 0,
                y_m: 0,
                z_m: 0,
            },
            2,
        )
        .unwrap();
        let mut leaves = root.children().unwrap().to_vec();
        let parent = leaves.remove(refine);
        leaves.extend(parent.children().unwrap());
        VoxelCoverage::new(leaves).unwrap()
    }
    fn check(
        f: impl Fn(VoxelPosition) -> f32 + Copy,
        coverage: &VoxelCoverage,
        closed: bool,
    ) -> (usize, usize) {
        let mut edges = BTreeMap::<[Point; 2], (usize, i32)>::new();
        let mut transitions = 0;
        let mut triangles = 0;
        for key in coverage.keys() {
            let plan = VoxelTransitionPlan::new(key);
            let volume = VoxelVolume::fixture(plan.key().address(), f);
            let samples = VoxelTransitionSamples::new(
                plan.sample_points().iter().copied().map(f).collect(),
                &plan,
            )
            .unwrap();
            let regular = build_voxel_regular_mesh(&volume, &plan).unwrap();
            let transition = build_voxel_transition_mesh(&plan, &samples).unwrap();
            transitions += plan.replaced_cells();
            for mesh in [&regular, &transition] {
                let origin = mesh.address().origin();
                let points: Vec<_> = mesh
                    .vertices()
                    .iter()
                    .map(|v| {
                        Point(std::array::from_fn(|i| {
                            [origin.x_m, origin.y_m, origin.z_m][i] as f64
                                + v.anchor_m[i] as f64
                                + v.offset_m[i] as f64
                        }))
                    })
                    .collect();
                triangles += mesh.indices().len() / 3;
                for t in mesh.indices().chunks_exact(3) {
                    for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                        let (a, b) = (points[a as usize], points[b as usize]);
                        assert_ne!(a, b);
                        let entry = edges.entry([a.min(b), a.max(b)]).or_default();
                        entry.0 += 1;
                        entry.1 += if a < b { 1 } else { -1 };
                    }
                }
            }
        }
        for ([a, b], (count, orientation)) in edges {
            if count == 1 && !closed {
                assert!(
                    (0..3).any(|i| [0.0, 128.0].into_iter().any(|v| a.0[i] == v && b.0[i] == v)),
                    "interior crack {a:?} {b:?}"
                );
            } else {
                assert_eq!((count, orientation), (2, 0), "edge {a:?} {b:?}");
            }
        }
        (triangles, transitions)
    }
    #[test]
    fn transition_faces_edges_corners_and_exact_zero_planes_are_closed() {
        for refine in 0..8 {
            let coverage = partition(refine);
            let (triangles, cells) = check(
                |p| {
                    50.0 - procgen_core::Vec3::new(
                        (p.x_m - 64) as f32,
                        (p.y_m - 64) as f32,
                        (p.z_m - 64) as f32,
                    )
                    .length()
                },
                &coverage,
                true,
            );
            assert!(triangles > 0 && cells > 0);
        }
        for plane in [64.0, 64.125] {
            check(|p| plane - p.x_m as f32, &partition(0), false);
            check(
                |p| plane - (p.x_m + p.y_m + p.z_m) as f32,
                &partition(7),
                false,
            );
        }
    }
    #[test]
    fn balance_retains_finest_sources_and_matches_the_adaptive_baseline() {
        let parent = VoxelChunkAddress::containing(
            VoxelPosition {
                x_m: 0,
                y_m: 0,
                z_m: 0,
            },
            3,
        )
        .unwrap();
        let mut leaves = parent.children().unwrap().to_vec();
        let fine_parent = leaves.remove(0);
        let mut fine = fine_parent.children().unwrap().to_vec();
        let finer = fine.remove(1).children().unwrap();
        fine.extend(finer);
        leaves.extend(fine);
        assert!(matches!(
            VoxelCoverage::new(leaves.clone()),
            Err(crate::VoxelCoverageError::Balance)
        ));
        assert!(VoxelCoverage::balanced(leaves.clone(), leaves.len()).is_err());
        let balanced = VoxelCoverage::balanced(leaves, 256).unwrap();
        assert!(finer.iter().all(|a| balanced.leaves().contains(a)));
        assert!(
            [0, 1, 2]
                .into_iter()
                .all(|lod| balanced.leaves().iter().any(|a| a.lod() == lod))
        );
        check(
            |p| {
                100.0
                    - procgen_core::Vec3::new(
                        (p.x_m - 128) as f32,
                        (p.y_m - 128) as f32,
                        (p.z_m - 128) as f32,
                    )
                    .length()
            },
            &balanced,
            true,
        );

        let coverage = partition(0);
        let f = |p: VoxelPosition| {
            50.0 - procgen_core::Vec3::new(
                (p.x_m - 64) as f32,
                (p.y_m - 64) as f32,
                (p.z_m - 64) as f32,
            )
            .length()
        };
        let center = VoxelPosition {
            x_m: 64,
            y_m: 64,
            z_m: 64,
        };
        let mut transition_error = 0.0f32;
        for key in coverage.keys() {
            let plan = VoxelTransitionPlan::new(key);
            let volume = VoxelVolume::fixture(plan.key().address(), f);
            let samples = VoxelTransitionSamples::new(
                plan.sample_points().iter().copied().map(f).collect(),
                &plan,
            )
            .unwrap();
            for mesh in [
                build_voxel_regular_mesh(&volume, &plan).unwrap(),
                build_voxel_transition_mesh(&plan, &samples).unwrap(),
            ] {
                for vertex in mesh.vertices() {
                    transition_error = transition_error.max(
                        (vertex.position(mesh.address()).relative_to(center).length() - 50.0).abs(),
                    );
                }
            }
        }
        // One sixteenth of a coarse (2 m) voxel bounds curvature interpolation in this fixture.
        assert!(transition_error <= 0.125);
        println!("sphere radial error: bounded {transition_error:.6} m");
        let (triangles, _) = check(f, &coverage, true);
        println!("15-chunk sphere: bounded transitions {triangles} triangles");
    }
}
