use std::time::Instant;

use procgen_gpu_tests::{readback, request_device_with_limits};
use procgen_realtime_pilot::{
    PlanetDesignField, VOXEL_HALO, VOXEL_MESH_SCAN_BLOCKS, VOXEL_MESH_VERTEX_SLOTS,
    VOXEL_SAMPLE_COUNT, VOXEL_SAMPLE_SIDE, VoxelChunkAddress, VoxelChunkMesh, VoxelGpuChunk,
    VoxelGpuParameters, VoxelMeshConfig, VoxelMeshDraw, VoxelMeshGpuParameters, VoxelMeshStatus,
    VoxelMeshVertex, VoxelSampleIndex, VoxelVolume, voxel_density_shader, voxel_mesh_shader,
};
use wgpu::util::DeviceExt;

pub fn sample_index(i: usize) -> VoxelSampleIndex {
    VoxelSampleIndex {
        x: (i % VOXEL_SAMPLE_SIDE) as i32 - VOXEL_HALO,
        y: (i / VOXEL_SAMPLE_SIDE % VOXEL_SAMPLE_SIDE) as i32 - VOXEL_HALO,
        z: (i / VOXEL_SAMPLE_SIDE.pow(2)) as i32 - VOXEL_HALO,
    }
}

pub enum Source<'a> {
    Samples(&'a VoxelVolume),
    Field(&'a PlanetDesignField, VoxelChunkAddress),
}
pub struct Job {
    address: VoxelChunkAddress,
    pub config: VoxelMeshConfig,
    group: wgpu::BindGroup,
    density_group: Option<wgpu::BindGroup>,
    pub density: wgpu::Buffer,
    pub status: wgpu::Buffer,
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    draw: wgpu::Buffer,
}
pub struct Audit {
    pub status: VoxelMeshStatus,
    pub draw: VoxelMeshDraw,
    pub mesh: Option<VoxelChunkMesh>,
}
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    pipelines: [wgpu::ComputePipeline; 4],
    density_pipeline: wgpu::ComputePipeline,
}
impl Gpu {
    pub fn new() -> Self {
        let backends = if cfg!(target_os = "macos") {
            wgpu::Backends::METAL
        } else {
            wgpu::Backends::VULKAN
        };
        let limits = wgpu::Limits {
            max_storage_buffers_per_shader_stage: 8,
            ..wgpu::Limits::downlevel_defaults()
        };
        let (adapter, device, queue) =
            request_device_with_limits("voxel mesh audit", backends, limits)
                .expect("meshing audit requires the requested GPU backend");
        println!("GPU mesh: {} / {:?}", adapter.name, adapter.backend);
        let started = Instant::now();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("voxel meshing"),
            source: wgpu::ShaderSource::Wgsl(voxel_mesh_shader().into()),
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
                            read_only: binding <= 2,
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniform chunk meshing"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("uniform chunk meshing"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipelines = ["classify", "scan_blocks", "scan_totals", "emit"].map(|entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        });
        let density_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("density"),
            source: wgpu::ShaderSource::Wgsl(voxel_density_shader().into()),
        });
        let density_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("density"),
            layout: None,
            module: &density_module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        println!(
            "mesh + density pipeline creation: {:.2} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
        Self {
            device,
            queue,
            layout,
            pipelines,
            density_pipeline,
        }
    }
    pub fn prepare(&self, source: Source<'_>, config: VoxelMeshConfig) -> Job {
        let address = match source {
            Source::Samples(v) => v.address(),
            Source::Field(_, a) => a,
        };
        let params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh parameters"),
                contents: bytemuck::bytes_of(&VoxelMeshGpuParameters::new(config).unwrap()),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let chunk = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("chunk"),
                contents: bytemuck::bytes_of(&VoxelGpuChunk::new(address)),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let buffer = |label, bytes, usage| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST
                    | usage,
                mapped_at_creation: false,
            })
        };
        let density = buffer(
            "density",
            (VOXEL_SAMPLE_COUNT * 4) as u64,
            wgpu::BufferUsages::empty(),
        );
        let density_group = match source {
            Source::Samples(v) => {
                v.validate().unwrap();
                let data: Vec<_> = (0..VOXEL_SAMPLE_COUNT)
                    .map(|i| v.potential(sample_index(i)))
                    .collect();
                self.queue
                    .write_buffer(&density, 0, bytemuck::cast_slice(&data));
                None
            }
            Source::Field(field, _) => {
                let field_params =
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("field"),
                            contents: bytemuck::bytes_of(&VoxelGpuParameters::new(field)),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("density"),
                    layout: &self.density_pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: field_params.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: chunk.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: density.as_entire_binding(),
                        },
                    ],
                }))
            }
        };
        let scratch = buffer(
            "prefix scratch",
            (VOXEL_MESH_VERTEX_SLOTS * 8) as u64,
            wgpu::BufferUsages::empty(),
        );
        let blocks = buffer(
            "block scratch",
            (VOXEL_MESH_SCAN_BLOCKS * 16) as u64,
            wgpu::BufferUsages::empty(),
        );
        let status = buffer("status", 16, wgpu::BufferUsages::empty());
        let vertices = buffer(
            "vertices",
            config.vertex_capacity as u64 * 32,
            wgpu::BufferUsages::VERTEX,
        );
        let indices = buffer(
            "indices",
            config.triangle_capacity as u64 * 12,
            wgpu::BufferUsages::INDEX,
        );
        let draw = buffer("draw", 20, wgpu::BufferUsages::INDIRECT);
        let buffers = [
            &params, &chunk, &density, &scratch, &blocks, &status, &vertices, &indices, &draw,
        ];
        let entries: Vec<_> = buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.as_entire_binding(),
            })
            .collect();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh chunk"),
            layout: &self.layout,
            entries: &entries,
        });
        Job {
            address,
            config,
            group,
            density_group,
            density,
            status,
            vertices,
            indices,
            draw,
        }
    }
    /// Interleave chunks by pass or finish each chunk in turn. Neither schedule
    /// may change output. All density and extraction passes precede audit readback.
    pub fn dispatch(&self, jobs: &[&Job], interleaved: bool) -> f64 {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for job in jobs {
            if let Some(group) = &job.density_group {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.density_pipeline);
                pass.set_bind_group(0, group, &[]);
                pass.dispatch_workgroups((VOXEL_SAMPLE_COUNT as u32).div_ceil(64), 1, 1);
            }
        }
        let mut encode = |job: &Job, stage: usize| {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.pipelines[stage]);
            pass.set_bind_group(0, &job.group, &[]);
            pass.dispatch_workgroups(
                if stage == 2 {
                    1
                } else {
                    VOXEL_MESH_SCAN_BLOCKS as u32
                },
                1,
                1,
            );
        };
        if interleaved {
            for stage in 0..4 {
                for job in jobs {
                    encode(job, stage);
                }
            }
        } else {
            for job in jobs {
                for stage in 0..4 {
                    encode(job, stage);
                }
            }
        }
        let start = Instant::now();
        self.queue.submit([encoder.finish()]);
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        start.elapsed().as_secs_f64() * 1000.0
    }
    pub fn audit(&self, job: &Job) -> Audit {
        let status = readback::<VoxelMeshStatus>(&self.device, &self.queue, &job.status, 1)[0];
        let draw = readback::<VoxelMeshDraw>(&self.device, &self.queue, &job.draw, 1)[0];
        assert_eq!(
            draw,
            VoxelMeshDraw {
                index_count: if status.overflow == 0 {
                    status.index_count
                } else {
                    0
                },
                instance_count: 1,
                first_index: 0,
                base_vertex: 0,
                first_instance: 0
            }
        );
        let mesh = if status.overflow != 0 {
            None
        } else {
            assert!(
                status.vertex_count <= job.config.vertex_capacity
                    && status.index_count <= job.config.triangle_capacity * 3
            );
            let vertices = if status.vertex_count == 0 {
                Vec::new()
            } else {
                readback::<VoxelMeshVertex>(
                    &self.device,
                    &self.queue,
                    &job.vertices,
                    status.vertex_count as usize,
                )
            };
            let indices = if status.index_count == 0 {
                Vec::new()
            } else {
                readback::<u32>(
                    &self.device,
                    &self.queue,
                    &job.indices,
                    status.index_count as usize,
                )
            };
            Some(VoxelChunkMesh::from_parts(job.address, vertices, indices).unwrap())
        };
        Audit { status, draw, mesh }
    }
    pub fn read_volume(&self, job: &Job) -> VoxelVolume {
        VoxelVolume::from_potentials(
            job.address,
            readback(&self.device, &self.queue, &job.density, VOXEL_SAMPLE_COUNT),
        )
        .unwrap()
    }
}
