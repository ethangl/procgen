//! Persistent output slots. Source and scan storage belong to in-flight work.
pub(crate) const OVERFLOW_FLAGS_BYTES: u64 = 2 * size_of::<u32>() as u64;

use crate::{VoxelMeshConfig, VoxelMeshError, VoxelTransitionConfig};

#[derive(Clone, Copy, Debug)]
pub struct VoxelGpuMeshConfig {
    pub regular: VoxelMeshConfig,
    pub transition: VoxelTransitionConfig,
}
impl VoxelGpuMeshConfig {
    pub fn validate(self) -> Result<(), VoxelMeshError> {
        self.regular.validate()?;
        self.transition.validate()
    }
    pub fn slot_bytes(self) -> u64 {
        self.regular.vertex_capacity as u64 * size_of::<crate::VoxelMeshVertex>() as u64
            + self.regular.triangle_capacity as u64 * 12
            + self.transition.triangle_capacity as u64
                * 3
                * (size_of::<crate::VoxelMeshVertex>() + size_of::<u32>()) as u64
            + 72
    }
}
pub struct VoxelGpuMeshBuffers {
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub status: wgpu::Buffer,
    pub draw: wgpu::Buffer,
}
impl VoxelGpuMeshBuffers {
    fn new(device: &wgpu::Device, vertices: u32, triangles: u32) -> Self {
        Self {
            vertices: storage(
                device,
                "vertices",
                vertices as u64 * size_of::<crate::VoxelMeshVertex>() as u64,
                wgpu::BufferUsages::VERTEX,
            ),
            indices: storage(
                device,
                "indices",
                triangles as u64 * 12,
                wgpu::BufferUsages::INDEX,
            ),
            status: storage(device, "status", 16, wgpu::BufferUsages::empty()),
            draw: storage(device, "draw", 20, wgpu::BufferUsages::INDIRECT),
        }
    }
}
pub struct VoxelGpuSlot {
    pub regular: VoxelGpuMeshBuffers,
    pub transition: VoxelGpuMeshBuffers,
}
impl VoxelGpuSlot {
    pub fn new(device: &wgpu::Device, config: VoxelGpuMeshConfig) -> Result<Self, VoxelMeshError> {
        config.validate()?;
        Ok(Self {
            regular: VoxelGpuMeshBuffers::new(
                device,
                config.regular.vertex_capacity,
                config.regular.triangle_capacity,
            ),
            transition: VoxelGpuMeshBuffers::new(
                device,
                config.transition.triangle_capacity * 3,
                config.transition.triangle_capacity,
            ),
        })
    }
}
pub(crate) fn storage(
    device: &wgpu::Device,
    label: &str,
    bytes: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST
            | usage,
        mapped_at_creation: false,
    })
}
