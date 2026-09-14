//! Transition extraction shares G1 source sampling and G2's vertex/draw layouts.
use crate::voxel_mesh_topology::SIMPLEX_RINGS;
use crate::{
    MAX_TRANSITION_TETRAHEDRA, VOXEL_MESH_WORKGROUP_SIZE, VoxelMeshError, VoxelTransitionPlan,
};
use bytemuck::{Pod, Zeroable};
use std::fmt::Write;

#[derive(Clone, Copy, Debug)]
pub struct VoxelTransitionConfig {
    pub triangle_capacity: u32,
}
impl VoxelTransitionConfig {
    pub fn validate(self) -> Result<(), VoxelMeshError> {
        if self.triangle_capacity == 0
            || self.triangle_capacity as usize > MAX_TRANSITION_TETRAHEDRA * 2
        {
            return Err(VoxelMeshError::TransitionCapacity);
        }
        Ok(())
    }
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct VoxelTransitionGpuParameters {
    capacity: u32,
    tetrahedra: u32,
    one: f32,
    zero: f32,
}
impl VoxelTransitionGpuParameters {
    pub fn new(
        config: VoxelTransitionConfig,
        plan: &VoxelTransitionPlan,
    ) -> Result<Self, VoxelMeshError> {
        config.validate()?;
        Ok(Self {
            capacity: config.triangle_capacity * 3,
            tetrahedra: plan.tetrahedra().len() as u32,
            one: 1.0,
            zero: 0.0,
        })
    }
}
/// Bindings: parameters, plan nodes, plan tetrahedra, canonical sample values,
/// scan scratch, status, vertices, indices, draw. Types of the last four match G2.
/// Scratch holds (tetrahedra + ceil(tetrahedra / WORKGROUP_SIZE)) pairs of u32.
/// Dispatch classify, scan_blocks, scan_totals, emit; skip empty plans entirely.
/// Transition output is indexed triangle soup with canonical shared root positions.
pub fn voxel_transition_shader() -> String {
    let mut source = format!(
        "const GROUP_SIZE: u32 = {VOXEL_MESH_WORKGROUP_SIZE}u;\nconst RINGS = array<vec4<u32>,16>(\n"
    );
    for r in SIMPLEX_RINGS.iter() {
        writeln!(
            source,
            "vec4<u32>({}u,{}u,{}u,{}u),",
            r[0], r[1], r[2], r[3]
        )
        .unwrap();
    }
    source.push_str(");\n");
    source.push_str(include_str!(
        "../../../crates/procgen-core/wgsl/arithmetic.wgsl"
    ));
    source.push_str(include_str!("voxel_transition.wgsl"));
    source
}
