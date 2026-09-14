//! wgpu command encoding used by residency and explicit CPU/GPU agreement audits.
use crate::voxel_gpu_buffers::{OVERFLOW_FLAGS_BYTES, storage};
use crate::{
    PlanetDesignField, VOXEL_MESH_SCAN_BLOCKS, VOXEL_MESH_VERTEX_SLOTS, VOXEL_MESH_WORKGROUP_SIZE,
    VOXEL_SAMPLE_COUNT, VoxelGpuChunk, VoxelGpuMeshConfig, VoxelGpuParameters, VoxelGpuSlot,
    VoxelMeshError, VoxelMeshGpuChunk, VoxelMeshGpuParameters, VoxelTransitionGpuParameters,
    VoxelTransitionPlan, VoxelTransitionSamples, VoxelVolume, voxel_density_shader,
    voxel_mesh_shader, voxel_transition_shader,
};
use wgpu::util::DeviceExt;

pub enum VoxelGpuInput<'a> {
    Field(&'a PlanetDesignField),
    Samples {
        volume: &'a VoxelVolume,
        transition: &'a VoxelTransitionSamples,
    },
}
pub struct VoxelGpuWork {
    regular: wgpu::BindGroup,
    transition: Option<wgpu::BindGroup>,
    density_group: Option<wgpu::BindGroup>,
    points_group: Option<wgpu::BindGroup>,
    density: wgpu::Buffer,
    point_values: wgpu::Buffer,
    buffers: Vec<wgpu::Buffer>,
    point_count: usize,
    transition_blocks: u32,
}
impl VoxelGpuWork {
    pub fn density(&self) -> &wgpu::Buffer {
        &self.density
    }
    pub fn point_values(&self) -> &wgpu::Buffer {
        &self.point_values
    }
    pub fn allocated_bytes(&self) -> u64 {
        self.buffers.iter().map(wgpu::Buffer::size).sum::<u64>()
            + self.density.size()
            + self.point_values.size()
    }
}
struct MeshKernels {
    pipelines: [wgpu::ComputePipeline; 4],
    layout: wgpu::BindGroupLayout,
}
impl MeshKernels {
    fn new(device: &wgpu::Device, source: String, read_only: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("voxel mesh"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let entries: Vec<_> = (0..9)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: if binding == 0 {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage {
                            read_only: binding <= read_only,
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("voxel mesh"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("voxel mesh"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipelines = ["classify", "scan_blocks", "scan_totals", "emit"].map(|entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        });
        Self { pipelines, layout }
    }
    fn encode(&self, encoder: &mut wgpu::CommandEncoder, group: &wgpu::BindGroup, blocks: u32) {
        for (i, pipeline) in self.pipelines.iter().enumerate() {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(if i == 2 { 1 } else { blocks }, 1, 1);
        }
    }
}
/// The caller selects and supplies the device/backend. Eight storage bindings
/// are required; no device discovery, feature upgrade, or CPU fallback occurs here.
pub struct VoxelGpuMesher {
    device: wgpu::Device,
    queue: wgpu::Queue,
    regular: MeshKernels,
    transition: MeshKernels,
    density: wgpu::ComputePipeline,
    points: wgpu::ComputePipeline,
}
impl VoxelGpuMesher {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let source = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("voxel density"),
            source: wgpu::ShaderSource::Wgsl(voxel_density_shader().into()),
        });
        let density_pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &source,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            device: device.clone(),
            queue: queue.clone(),
            regular: MeshKernels::new(device, voxel_mesh_shader(), 2),
            transition: MeshKernels::new(device, voxel_transition_shader(), 3),
            density: density_pipeline("main"),
            points: density_pipeline("sample_points"),
        }
    }
    /// Upper bound before GPU allocation, including audit status transfer. Empty
    /// transition arrays use a single inert storage record and are never dispatched.
    pub fn work_bytes(plan: &VoxelTransitionPlan) -> u64 {
        let points = plan.sample_points().len().max(1) as u64;
        let nodes = plan.nodes().len().max(1) as u64;
        let tets = plan.tetrahedra().len() as u64;
        let blocks = tets.div_ceil(VOXEL_MESH_WORKGROUP_SIZE as u64);
        // Regular params/chunk/mask/scratch, density params, point records/values,
        // transition params/nodes/tets/scratch, and eight-byte completion readback.
        16 + 16
            + size_of::<VoxelMeshGpuChunk>() as u64
            + (VOXEL_MESH_VERTEX_SLOTS * 8 + VOXEL_MESH_SCAN_BLOCKS * 16 + VOXEL_SAMPLE_COUNT * 4)
                as u64
            + size_of::<VoxelGpuParameters>() as u64
            + points * 20
            + 16
            + nodes * 64
            + tets.max(1) * 16
            + (tets + blocks).max(1) * 8
            + OVERFLOW_FLAGS_BYTES
    }
    pub fn prepare(
        &self,
        plan: &VoxelTransitionPlan,
        input: VoxelGpuInput<'_>,
        slot: &VoxelGpuSlot,
        config: VoxelGpuMeshConfig,
    ) -> Result<VoxelGpuWork, VoxelMeshError> {
        config.validate()?;
        plan.validate()?;
        let mut buffers = Vec::new();
        let upload = |label, bytes: &[u8], uniform| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytes,
                    usage: if uniform {
                        wgpu::BufferUsages::UNIFORM
                    } else {
                        wgpu::BufferUsages::STORAGE
                    },
                })
        };
        let params = upload(
            "mesh parameters",
            bytemuck::bytes_of(&VoxelMeshGpuParameters::new(config.regular)?),
            true,
        );
        let chunk = upload(
            "density chunk",
            bytemuck::bytes_of(&VoxelGpuChunk::new(plan.key().address())),
            false,
        );
        let mesh_chunk = upload(
            "mesh chunk",
            bytemuck::bytes_of(&VoxelMeshGpuChunk::transition(plan)),
            false,
        );
        let density = storage(
            &self.device,
            "density",
            (VOXEL_SAMPLE_COUNT * 4) as u64,
            wgpu::BufferUsages::empty(),
        );
        let point_values = storage(
            &self.device,
            "transition sample values",
            plan.sample_points().len().max(1) as u64 * 4,
            wgpu::BufferUsages::empty(),
        );
        let scratch = storage(
            &self.device,
            "mesh scan",
            (VOXEL_MESH_VERTEX_SLOTS * 8) as u64,
            wgpu::BufferUsages::empty(),
        );
        let blocks = storage(
            &self.device,
            "mesh blocks",
            (VOXEL_MESH_SCAN_BLOCKS * 16) as u64,
            wgpu::BufferUsages::empty(),
        );
        let regular = bind(
            &self.device,
            &self.regular.layout,
            &[
                &params,
                &mesh_chunk,
                &density,
                &scratch,
                &blocks,
                &slot.regular.status,
                &slot.regular.vertices,
                &slot.regular.indices,
                &slot.regular.draw,
            ],
        );
        let (density_group, points_group) = match input {
            VoxelGpuInput::Field(field) => {
                let field = upload(
                    "field",
                    bytemuck::bytes_of(&VoxelGpuParameters::new(field)),
                    true,
                );
                let group = bind(
                    &self.device,
                    &self.density.get_bind_group_layout(0),
                    &[&field, &chunk, &density],
                );
                let points_group = if plan.sample_points().is_empty() {
                    None
                } else {
                    let points: Vec<_> = plan
                        .sample_points()
                        .iter()
                        .copied()
                        .map(crate::voxel_gpu::VoxelGpuPoint::new)
                        .collect();
                    let points = upload(
                        "transition source points",
                        bytemuck::cast_slice(&points),
                        false,
                    );
                    let group = bind(
                        &self.device,
                        &self.points.get_bind_group_layout(0),
                        &[&field, &points, &point_values],
                    );
                    buffers.push(points);
                    Some(group)
                };
                buffers.push(field);
                (Some(group), points_group)
            }
            VoxelGpuInput::Samples { volume, transition } => {
                volume.validate()?;
                transition.validate(plan)?;
                if volume.address() != plan.key().address() {
                    return Err(VoxelMeshError::TransitionAddress);
                }
                let values: Vec<_> = (0..VOXEL_SAMPLE_COUNT)
                    .map(|i| volume.potential(crate::VoxelSampleIndex::from_linear(i)))
                    .collect();
                self.queue
                    .write_buffer(&density, 0, bytemuck::cast_slice(&values));
                if !transition.values().is_empty() {
                    self.queue.write_buffer(
                        &point_values,
                        0,
                        bytemuck::cast_slice(transition.values()),
                    );
                }
                (None, None)
            }
        };
        let transition = if plan.tetrahedra().is_empty() {
            None
        } else {
            let params = upload(
                "transition parameters",
                bytemuck::bytes_of(&VoxelTransitionGpuParameters::new(config.transition, plan)?),
                true,
            );
            let nodes = upload(
                "transition nodes",
                bytemuck::cast_slice(plan.nodes()),
                false,
            );
            let tets = upload(
                "transition tetrahedra",
                bytemuck::cast_slice(plan.tetrahedra()),
                false,
            );
            let count = plan.tetrahedra().len();
            let scratch = storage(
                &self.device,
                "transition scan",
                ((count + count.div_ceil(VOXEL_MESH_WORKGROUP_SIZE)) * 8) as u64,
                wgpu::BufferUsages::empty(),
            );
            let group = bind(
                &self.device,
                &self.transition.layout,
                &[
                    &params,
                    &nodes,
                    &tets,
                    &point_values,
                    &scratch,
                    &slot.transition.status,
                    &slot.transition.vertices,
                    &slot.transition.indices,
                    &slot.transition.draw,
                ],
            );
            buffers.extend([params, nodes, tets, scratch]);
            Some(group)
        };
        buffers.extend([params, chunk, mesh_chunk, scratch, blocks]);
        let work = VoxelGpuWork {
            regular,
            transition,
            density_group,
            points_group,
            density,
            point_values,
            buffers,
            point_count: plan.sample_points().len(),
            transition_blocks: plan.tetrahedra().len().div_ceil(VOXEL_MESH_WORKGROUP_SIZE) as u32,
        };
        debug_assert!(work.allocated_bytes() + OVERFLOW_FLAGS_BYTES <= Self::work_bytes(plan));
        Ok(work)
    }
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        work: &VoxelGpuWork,
        slot: &VoxelGpuSlot,
    ) {
        encoder.clear_buffer(&slot.transition.status, 0, None);
        encoder.clear_buffer(&slot.transition.draw, 0, None);
        if let Some(group) = &work.density_group {
            encode_density(
                encoder,
                &self.density,
                group,
                (VOXEL_SAMPLE_COUNT as u32).div_ceil(64),
            );
        }
        if let Some(group) = &work.points_group {
            encode_density(
                encoder,
                &self.points,
                group,
                (work.point_count as u32).div_ceil(64),
            );
        }
        self.regular
            .encode(encoder, &work.regular, VOXEL_MESH_SCAN_BLOCKS as u32);
        if let Some(group) = &work.transition {
            self.transition
                .encode(encoder, group, work.transition_blocks);
        }
    }
}
fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffers: &[&wgpu::Buffer],
) -> wgpu::BindGroup {
    let entries: Vec<_> = buffers
        .iter()
        .enumerate()
        .map(|(i, b)| wgpu::BindGroupEntry {
            binding: i as u32,
            resource: b.as_entire_binding(),
        })
        .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("voxel generation"),
        layout,
        entries: &entries,
    })
}
fn encode_density(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::ComputePipeline,
    group: &wgpu::BindGroup,
    blocks: u32,
) {
    let mut pass = encoder.begin_compute_pass(&Default::default());
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.dispatch_workgroups(blocks, 1, 1);
}
