use procgen_gpu_tests::{readback, request_device_with_limits};
use procgen_realtime_pilot::{
    PlanetDesignField, VOXEL_HALO, VOXEL_SAMPLE_COUNT, VOXEL_SAMPLE_SIDE, VoxelChunkAddress,
    VoxelChunkMesh, VoxelCoverage, VoxelGpuInput, VoxelGpuMeshConfig, VoxelGpuMesher, VoxelGpuSlot,
    VoxelGpuWork, VoxelMeshConfig, VoxelMeshDraw, VoxelMeshStatus, VoxelMeshVertex,
    VoxelSampleIndex, VoxelTransitionConfig, VoxelTransitionPlan, VoxelTransitionSamples,
    VoxelVolume,
};
use std::time::Instant;

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
    pub slot: VoxelGpuSlot,
    work: VoxelGpuWork,
}
pub struct Audit {
    pub status: VoxelMeshStatus,
    pub draw: VoxelMeshDraw,
    pub mesh: Option<VoxelChunkMesh>,
}
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    mesher: VoxelGpuMesher,
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
        let start = Instant::now();
        let mesher = VoxelGpuMesher::new(&device, &queue);
        println!(
            "mesh + density + transition pipeline creation: {:.2} ms",
            start.elapsed().as_secs_f64() * 1000.0
        );
        Self {
            device,
            queue,
            mesher,
        }
    }
    pub fn prepare(&self, source: Source<'_>, regular: VoxelMeshConfig) -> Job {
        let address = match source {
            Source::Samples(v) => v.address(),
            Source::Field(_, a) => a,
        };
        let coverage = VoxelCoverage::new(vec![address]).unwrap();
        let plan = VoxelTransitionPlan::new(coverage.key(address).unwrap());
        let config = VoxelGpuMeshConfig {
            regular,
            transition: VoxelTransitionConfig {
                triangle_capacity: 1,
            },
        };
        let slot = VoxelGpuSlot::new(&self.device, config).unwrap();
        let empty = VoxelTransitionSamples::new(vec![], &plan).unwrap();
        let input = match source {
            Source::Samples(volume) => VoxelGpuInput::Samples {
                volume,
                transition: &empty,
            },
            Source::Field(field, _) => VoxelGpuInput::Field(field),
        };
        let work = self.mesher.prepare(&plan, input, &slot, config).unwrap();
        Job {
            address,
            config: regular,
            slot,
            work,
        }
    }
    /// Compare a shared submission against one submission per chunk. The production
    /// encoder owns pass ordering; tests no longer duplicate that execution path.
    pub fn dispatch(&self, jobs: &[&Job], batched: bool) -> f64 {
        let start = Instant::now();
        if batched {
            let mut encoder = self.device.create_command_encoder(&Default::default());
            for job in jobs {
                self.mesher.encode(&mut encoder, &job.work, &job.slot);
            }
            self.queue.submit([encoder.finish()]);
        } else {
            for job in jobs {
                let mut encoder = self.device.create_command_encoder(&Default::default());
                self.mesher.encode(&mut encoder, &job.work, &job.slot);
                self.queue.submit([encoder.finish()]);
            }
        }
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        start.elapsed().as_secs_f64() * 1000.0
    }
    pub fn audit(&self, job: &Job) -> Audit {
        let output = &job.slot.regular;
        let status = readback::<VoxelMeshStatus>(&self.device, &self.queue, &output.status, 1)[0];
        let draw = readback::<VoxelMeshDraw>(&self.device, &self.queue, &output.draw, 1)[0];
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
                vec![]
            } else {
                readback::<VoxelMeshVertex>(
                    &self.device,
                    &self.queue,
                    &output.vertices,
                    status.vertex_count as usize,
                )
            };
            let indices = if status.index_count == 0 {
                vec![]
            } else {
                readback::<u32>(
                    &self.device,
                    &self.queue,
                    &output.indices,
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
            readback(
                &self.device,
                &self.queue,
                job.work.density(),
                VOXEL_SAMPLE_COUNT,
            ),
        )
        .unwrap()
    }
}
