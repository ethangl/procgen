//! GPU height tiles and canonical CPU audit vertices. No device discovery.
use crate::{PlanetDesignField, VoxelGpuParameters, voxel_density_shader};
use procgen_cubesphere::TileAddress;
use wgpu::util::DeviceExt;

use crate::{HEIGHT_TILE_BYTES, HEIGHT_VERTEX_COUNT};
pub fn height_shader() -> String {
    format!(
        "{}\n{}\nconst HEIGHT_VERTEX_COUNT: u32 = {}u;\nconst HEIGHT_QUADS: u32 = {}u;\nconst HEIGHT_SIDE: u32 = {}u;\nconst HEIGHT_FILTER_DISTANCE_RATIO: f32 = {:?};\nconst HEIGHT_FILTER_MIN_M: f32 = {:?};\nconst HEIGHT_NORMAL_MIN_STEP_M: f32 = {:?};\n{}",
        voxel_density_shader(),
        procgen_cubesphere::MAPPING_WGSL_SOURCE,
        HEIGHT_VERTEX_COUNT,
        crate::HEIGHT_QUADS,
        crate::HEIGHT_SIDE,
        crate::HEIGHT_FILTER_DISTANCE_RATIO,
        crate::HEIGHT_FILTER_MIN_M,
        crate::height_mesh::HEIGHT_NORMAL_MIN_STEP_M,
        include_str!("height_gpu.wgsl")
    )
}
pub struct HeightGpuMesher {
    pipeline: wgpu::ComputePipeline,
    field: wgpu::BindGroup,
}
impl HeightGpuMesher {
    pub fn new(device: &wgpu::Device, field: &PlanetDesignField) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("physical height mesh"),
            source: wgpu::ShaderSource::Wgsl(height_shader().into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("physical height mesh"),
            layout: None,
            module: &shader,
            entry_point: Some("height_mesh"),
            compilation_options: Default::default(),
            cache: None,
        });
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("physical height parameters"),
            contents: bytemuck::bytes_of(&VoxelGpuParameters::new(field)),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let field = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("physical height field"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            }],
        });
        Self { pipeline, field }
    }
    /// Encode one immutable tile. The caller retains output through draw completion.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        tile: TileAddress,
        filter: crate::HeightFilter,
    ) -> wgpu::Buffer {
        let address = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("physical height address"),
            contents: bytemuck::cast_slice(&tile.gpu_words()),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("physical height vertices"),
            size: HEIGHT_TILE_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let filter = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("height filter"),
            contents: bytemuck::bytes_of(&filter),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("physical height tile"),
            layout: &self.pipeline.get_bind_group_layout(1),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: filter.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: address.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("physical height tile"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.field, &[]);
        pass.set_bind_group(1, &group, &[]);
        pass.dispatch_workgroups(HEIGHT_VERTEX_COUNT.div_ceil(64), 1, 1);
        drop(pass);
        output
    }
}

use crate::{
    VoxelGpuMeshConfig, VoxelGpuWorldConfig, VoxelMeshConfig, VoxelStreamConfig,
    VoxelTransitionConfig,
};
/// Local slots have no mixed-LOD transitions; reserve one inert transition triangle.
pub const LOCAL_GPU_WORLD_CONFIG: VoxelGpuWorldConfig = VoxelGpuWorldConfig {
    stream: VoxelStreamConfig {
        max_slots: 300,
        max_in_flight: 8,
    },
    mesh: VoxelGpuMeshConfig {
        regular: VoxelMeshConfig {
            vertex_capacity: 20_000,
            triangle_capacity: 40_000,
        },
        transition: VoxelTransitionConfig {
            triangle_capacity: 1,
        },
    },
    memory_budget_bytes: 512 * 1024 * 1024,
};
