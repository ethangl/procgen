//! Bounded GPU mesh layouts and shader composition; consumers own devices and slots.
use std::fmt::Write;

use bytemuck::{Pod, Zeroable};

use crate::voxel_mesh_topology::{
    CELL_COUNT, CELLS, NODES, RINGS, TETS, VOXEL_MESH_MAX_TRIANGLES, VOXEL_MESH_VERTEX_SLOTS,
};
use crate::{
    VOXEL_HALO, VOXEL_SAMPLE_COUNT, VOXEL_SAMPLE_SIDE, VoxelChunkAddress, VoxelGpuChunk,
    VoxelMeshError, VoxelTransitionPlan,
};

/// G2 extraction skips cells owned by the transition plan. Density generation
/// still binds the original 16-byte VoxelGpuChunk record.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct VoxelMeshGpuChunk {
    chunk: VoxelGpuChunk,
    blocked: [u32; crate::voxel_transition_plan::VOXEL_CELL_MASK_WORDS],
}
impl VoxelMeshGpuChunk {
    pub fn uniform(address: VoxelChunkAddress) -> Self {
        Self {
            chunk: VoxelGpuChunk::new(address),
            blocked: [0; crate::voxel_transition_plan::VOXEL_CELL_MASK_WORDS],
        }
    }
    pub fn transition(plan: &VoxelTransitionPlan) -> Self {
        Self {
            chunk: VoxelGpuChunk::new(plan.key().address()),
            blocked: plan.blocked,
        }
    }
}

pub const VOXEL_MESH_WORKGROUP_SIZE: usize = 256;
pub const VOXEL_MESH_SCAN_BLOCKS: usize =
    VOXEL_MESH_VERTEX_SLOTS.div_ceil(VOXEL_MESH_WORKGROUP_SIZE);

#[derive(Clone, Copy, Debug)]
pub struct VoxelMeshConfig {
    pub vertex_capacity: u32,
    pub triangle_capacity: u32,
}
impl VoxelMeshConfig {
    pub fn validate(self) -> Result<(), VoxelMeshError> {
        if self.vertex_capacity == 0
            || self.vertex_capacity as usize > VOXEL_MESH_VERTEX_SLOTS
            || self.triangle_capacity == 0
            || self.triangle_capacity as usize > VOXEL_MESH_MAX_TRIANGLES
        {
            return Err(VoxelMeshError::Capacity);
        }
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct VoxelMeshGpuParameters {
    vertex_capacity: u32,
    index_capacity: u32,
    arithmetic_one: f32,
    arithmetic_zero: f32,
}
impl VoxelMeshGpuParameters {
    pub fn new(config: VoxelMeshConfig) -> Result<Self, VoxelMeshError> {
        config.validate()?;
        Ok(Self {
            vertex_capacity: config.vertex_capacity,
            index_capacity: config.triangle_capacity * 3,
            arithmetic_one: 1.0,
            arithmetic_zero: 0.0,
        })
    }
}

/// Counts are required capacities, including when overflow is one. An overflow
/// slot must not replace resident coverage; its indirect draw is disabled.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct VoxelMeshStatus {
    pub vertex_count: u32,
    pub index_count: u32,
    pub overflow: u32,
    pub reserved: u32,
}

/// Layout accepted by wgpu draw_indexed_indirect. Bind the chunk's vertex/index
/// slices when drawing; offsets are local so each slot can be managed separately.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct VoxelMeshDraw {
    pub index_count: u32,
    pub instance_count: u32,
    pub first_index: u32,
    pub base_vertex: i32,
    pub first_instance: u32,
}

/// One chunk per bind group. Bindings: 0 parameters, 1 VoxelMeshGpuChunk record, 2 potentials,
/// 3 scan scratch (vertex slots * 8 bytes), 4 block scratch (scan blocks * 16),
/// 5 status (16), 6 vertices (capacity * 32), 7 indices (triangle capacity * 12),
/// 8 indirect draw (20). All but 0 are storage; 1/2 are read-only. Outputs also
/// carry VERTEX, INDEX and INDIRECT usage respectively. Density comes directly
/// from G1. Each entry point uses the same explicit layout.
/// Dispatch classify, scan_blocks, scan_totals, emit in order, with x workgroups
/// [SCAN_BLOCKS, SCAN_BLOCKS, 1, SCAN_BLOCKS]. No CPU readback is needed between
/// passes. Scratch and outputs must belong to an unpublished destination slot.
/// The consumer validates device buffer/binding limits before allocating slots.
pub fn voxel_mesh_shader() -> String {
    let mask_words = crate::voxel_transition_plan::VOXEL_CELL_MASK_WORDS;
    let mut source = format!(
        "const MASK_WORDS: u32 = {mask_words}u;\nconst CELLS: u32 = {CELLS}u;\nconst NODES: u32 = {NODES}u;\nconst CELL_COUNT: u32 = {CELL_COUNT}u;\nconst SLOTS: u32 = {VOXEL_MESH_VERTEX_SLOTS}u;\nconst BLOCKS: u32 = {VOXEL_MESH_SCAN_BLOCKS}u;\nconst GROUP_SIZE: u32 = {VOXEL_MESH_WORKGROUP_SIZE}u;\nconst SAMPLE_SIDE: u32 = {VOXEL_SAMPLE_SIDE}u;\nconst SAMPLE_COUNT: u32 = {VOXEL_SAMPLE_COUNT}u;\nconst HALO: u32 = {VOXEL_HALO}u;\n"
    );
    writeln!(source, "const TETS = array<vec4<u32>, 6>(").unwrap();
    for t in TETS {
        writeln!(
            source,
            "vec4<u32>({}u,{}u,{}u,{}u),",
            t[0], t[1], t[2], t[3]
        )
        .unwrap();
    }
    writeln!(source, ");\nconst RINGS = array<vec4<u32>, 96>(").unwrap();
    for r in RINGS.iter() {
        writeln!(
            source,
            "vec4<u32>({}u,{}u,{}u,{}u),",
            r[0], r[1], r[2], r[3]
        )
        .unwrap();
    }
    writeln!(source, ");").unwrap();
    source.push_str(include_str!(
        "../../../crates/procgen-core/wgsl/arithmetic.wgsl"
    ));
    source.push_str(include_str!("voxel_mesh.wgsl"));
    source
}
