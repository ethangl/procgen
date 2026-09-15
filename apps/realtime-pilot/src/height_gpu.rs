//! GPU height tiles and canonical CPU audit vertices. No device discovery.
use crate::{HeightTile, PlanetDesignField, VoxelGpuParameters, voxel_density_shader};
use wgpu::util::DeviceExt;

use crate::{HEIGHT_TILE_BYTES, HEIGHT_VERTEX_COUNT};
/// Bounds temporary mesh output and interference with rendering.
pub const HEIGHT_GPU_BATCH_TILES: usize = 32;
pub fn height_shader() -> String {
    use procgen_cubesphere::FaceEdge;
    let edge_constants: String = [
        ("LEFT", FaceEdge::Left),
        ("RIGHT", FaceEdge::Right),
        ("BOTTOM", FaceEdge::Bottom),
        ("TOP", FaceEdge::Top),
    ]
    .into_iter()
    .map(|(name, edge)| {
        format!(
            "const HEIGHT_EDGE_{name}: u32 = {}u;\n",
            crate::height_tile::edge_bit(edge)
        )
    })
    .collect();
    let edge_constants = format!(
        "const HEIGHT_FINEST_LEVEL: u32 = {}u;\n{edge_constants}",
        procgen_cubesphere::MAX_TILE_LEVEL
    );
    format!(
        "{edge_constants}{}\n{}\nconst HEIGHT_VERTEX_COUNT: u32 = {}u;\nconst HEIGHT_QUADS: u32 = {}u;\nconst HEIGHT_SIDE: u32 = {}u;\nconst HEIGHT_DETAIL_RATIO: f32 = {:?};\nconst HEIGHT_FILTER_MIN_M: f32 = {:?};\nconst HEIGHT_NORMAL_MIN_STEP_M: f32 = {:?};\n{}",
        voxel_density_shader(),
        procgen_cubesphere::MAPPING_WGSL_SOURCE,
        HEIGHT_VERTEX_COUNT,
        crate::HEIGHT_QUADS,
        crate::HEIGHT_SIDE,
        crate::HEIGHT_DETAIL_RATIO,
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
    /// One compute pass, followed by device copies into independently reusable
    /// tile buffers. The temporary output is bounded by HEIGHT_GPU_BATCH_TILES.
    pub fn encode_batch(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        tiles: &[HeightTile],
    ) -> Vec<wgpu::Buffer> {
        assert!(
            !tiles.is_empty() && tiles.len() <= HEIGHT_GPU_BATCH_TILES,
            "height batch must contain 1..={HEIGHT_GPU_BATCH_TILES} tiles"
        );
        let addresses: Vec<_> = tiles.iter().map(|t| t.gpu_words()).collect();
        let address = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("physical height address"),
            contents: bytemuck::cast_slice(&addresses),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("physical height vertices"),
            size: HEIGHT_TILE_BYTES * tiles.len() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("physical height tile"),
            layout: &self.pipeline.get_bind_group_layout(1),
            entries: &[
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
        pass.dispatch_workgroups(
            (HEIGHT_VERTEX_COUNT * tiles.len() as u32).div_ceil(64),
            1,
            1,
        );
        drop(pass);
        (0..tiles.len())
            .map(|i| {
                let tile = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("immutable height tile"),
                    size: HEIGHT_TILE_BYTES,
                    usage: wgpu::BufferUsages::VERTEX
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                encoder.copy_buffer_to_buffer(
                    &output,
                    i as u64 * HEIGHT_TILE_BYTES,
                    &tile,
                    0,
                    HEIGHT_TILE_BYTES,
                );
                tile
            })
            .collect()
    }
}

use crate::{
    VoxelGpuMeshConfig, VoxelGpuWorldConfig, VoxelMeshConfig, VoxelStreamConfig,
    VoxelTransitionConfig,
};
/// The near-field band is mixed-LOD, so every slot reserves transition geometry.
/// One slot costs 3,357,000 bytes: 20,000 48-byte vertices, 40,000 regular
/// triangles at 12 bytes of indices, 12,288 transition triangles at three
/// unshared vertices plus three indices each, and 72 bytes of status and draw
/// records. The 1,536 reserved slots are 4.803 GiB of the budget, leaving
/// 1.197 GiB for the eight in-flight working sets, each a few megabytes.
///
/// The slot count and both capacities come from replaying the height audit's
/// travel route on both presets, sampling every resident slot: at most 1,262
/// slots were resident, in flight, or retiring at once, the worst regular mesh
/// held 8,403 vertices and 16,418 triangles, and the worst transition mesh held
/// 16,251 vertices and 5,417 triangles. The transition reservation is larger
/// than that measured worst case because intermediate coverages the sampling
/// skipped overflowed 8,192.
pub const LOCAL_GPU_WORLD_CONFIG: VoxelGpuWorldConfig = VoxelGpuWorldConfig {
    stream: VoxelStreamConfig {
        max_slots: 1_536,
        max_in_flight: 8,
    },
    mesh: VoxelGpuMeshConfig {
        regular: VoxelMeshConfig {
            vertex_capacity: 20_000,
            triangle_capacity: 40_000,
        },
        transition: VoxelTransitionConfig {
            triangle_capacity: 12_288,
        },
    },
    memory_budget_bytes: 6 * 1024 * 1024 * 1024,
};
