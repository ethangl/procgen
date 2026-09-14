//! Latest camera input and immutable GPU draw leases. No render-thread generation.
use bevy::prelude::*;
use procgen_realtime_pilot::{
    MeterPosition, PlanetDesignField, VoxelChunkAddress, VoxelCoverage, VoxelGpuEvent,
    VoxelGpuLease, VoxelGpuMeshConfig, VoxelGpuOutcome, VoxelGpuWorld, VoxelGpuWorldConfig,
    VoxelMeshConfig, VoxelStreamConfig, VoxelTransitionConfig, select_voxel_coverage,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use wgpu::util::DeviceExt;

pub const GPU_WORLD_CONFIG: VoxelGpuWorldConfig = VoxelGpuWorldConfig {
    stream: VoxelStreamConfig {
        max_slots: procgen_realtime_pilot::MAX_VOXEL_COVERAGE_LEAVES,
        max_in_flight: 2,
    },
    mesh: VoxelGpuMeshConfig {
        regular: VoxelMeshConfig {
            vertex_capacity: 20_000,
            triangle_capacity: 40_000,
        },
        transition: VoxelTransitionConfig {
            triangle_capacity: 8_192,
        },
    },
    memory_budget_bytes: 8 * 1024 * 1024 * 1024,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExplorationBackend {
    Gpu,
    Cpu,
}
#[derive(Clone, Copy)]
pub struct GpuCamera {
    pub eye: MeterPosition,
}
pub struct DrawFrame {
    pub leases: Vec<VoxelGpuLease>,
    pub origins: wgpu::Buffer,
}
#[derive(Default, Clone)]
pub struct GpuStats {
    pub status: String,
    pub resident: usize,
    pub finest_spacing_m: Option<i32>,
    pub drawn: usize,
    pub target: usize,
    pub in_flight: usize,
    pub retiring: usize,
    pub bytes: u64,
    pub resident_bytes: u64,
    pub retiring_bytes: u64,
    pub selection_ms: f64,
    pub preparation_ms: f64,
    pub encoding_ms: f64,
    pub completion_ms: f64,
    pub submission_latency_ms: f64,
    pub gpu_times: Option<procgen_realtime_pilot::VoxelGpuTimes>,
    pub publication_ms: f64,
    pub scheduler_ms: f64,
    pub draw_ms: f64,
}
#[derive(Default)]
pub struct GpuOutput {
    pub frame: Option<Arc<DrawFrame>>,
    pub stats: GpuStats,
}
#[derive(Resource, Clone)]
pub struct GpuBridge {
    pub submissions:
        Arc<Mutex<std::sync::mpsc::Receiver<procgen_realtime_pilot::VoxelGpuSubmission>>>,
    submit: std::sync::mpsc::Sender<procgen_realtime_pilot::VoxelGpuSubmission>,
    pub field: Arc<PlanetDesignField>,
    pub camera: Arc<Mutex<GpuCamera>>,
    pub output: Arc<Mutex<GpuOutput>>,
}
impl GpuBridge {
    pub fn new(field: Arc<PlanetDesignField>, camera: GpuCamera) -> Self {
        let (submit, submissions) = std::sync::mpsc::channel();
        Self {
            submit,
            submissions: Arc::new(Mutex::new(submissions)),
            field,
            camera: Arc::new(Mutex::new(camera)),
            output: Arc::new(Mutex::new(GpuOutput::default())),
        }
    }
}
#[derive(Resource)]
pub struct GpuWorker {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Drop for GpuWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl GpuWorker {
    pub fn start(device: wgpu::Device, queue: wgpu::Queue, bridge: GpuBridge) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                generate(&device, &queue, &bridge, &cancel)
            }));
            let error = match result {
                Ok(Ok(())) => return,
                Ok(Err(error)) => error.to_string(),
                Err(_) => "GPU worker panicked; see the terminal diagnostic".into(),
            };
            bridge.output.lock().unwrap().stats.status = format!("GPU stream stopped: {error}");
        });
        Self {
            stop,
            worker: Some(worker),
        }
    }
}
fn generate(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bridge: &GpuBridge,
    stop: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut world = VoxelGpuWorld::new(device, queue, Arc::clone(&bridge.field), GPU_WORLD_CONFIG)?;
    let mut camera = *bridge.camera.lock().unwrap();
    let mut selected_at = camera.eye;
    let mut stats = GpuStats::default();
    let start = Instant::now();
    let mut target = select_voxel_coverage(
        &bridge.field,
        camera.eye.anchor(),
        512,
        procgen_realtime_pilot::MAX_VOXEL_COVERAGE_LEAVES,
    )?;
    stats.selection_ms = start.elapsed().as_secs_f64() * 1000.0;

    let mut current = VoxelCoverage::new(vec![VoxelChunkAddress::root()])?;
    world.set_coverage(&current, camera.eye.anchor())?;
    let mut replacement = Instant::now();
    let mut report = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        camera = *bridge.camera.lock().unwrap();
        let clearance = camera.eye.altitude_m(bridge.field.config().radius_m) as f32
            - bridge.field.elevation_m(camera.eye.direction(), 0.0)?;
        let threshold = (clearance * 0.1).clamp(16.0, 100_000.0);
        if camera.eye.relative_to(selected_at.anchor()).length() > threshold {
            let start = Instant::now();
            target = select_voxel_coverage(
                &bridge.field,
                camera.eye.anchor(),
                512,
                procgen_realtime_pilot::MAX_VOXEL_COVERAGE_LEAVES,
            )?;
            selected_at = camera.eye;
            stats.selection_ms = start.elapsed().as_secs_f64() * 1000.0;
        }
        for event in world.update_with_submit(|submission| {
            let _ = bridge.submit.send(submission);
        })? {
            match event {
                VoxelGpuEvent::Submitted {
                    preparation_ms,
                    encoding_ms,
                    ..
                } => {
                    stats.preparation_ms = preparation_ms;
                    stats.encoding_ms = encoding_ms;
                }
                VoxelGpuEvent::Completed {
                    elapsed_ms,
                    gpu_times,
                    submission_latency_ms,
                    outcome,
                    ..
                } => {
                    stats.completion_ms = elapsed_ms;
                    stats.gpu_times = gpu_times;
                    stats.submission_latency_ms = submission_latency_ms;
                    if matches!(outcome, VoxelGpuOutcome::Failed) {
                        return Err("mesh capacity exceeded; previous terrain retained".into());
                    }
                }
                VoxelGpuEvent::Published(_) => {
                    let leases = world.resident_leases();
                    // Instance origins are renderer metadata, not mesh payload.
                    let stride = 16;
                    let mut origins = vec![0u8; leases.len().max(1) * stride];
                    for (i, lease) in leases.iter().enumerate() {
                        let p = lease.key.address().origin();
                        origins[i * stride..i * stride + 16].copy_from_slice(bytemuck::cast_slice(
                            &[p.x_m, p.y_m, p.z_m, lease.key.address().lod() as i32],
                        ));
                    }
                    let origins = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("voxel draw origins"),
                        contents: &origins,
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                    stats.publication_ms = replacement.elapsed().as_secs_f64() * 1000.0;
                    stats.resident = leases.len();
                    stats.finest_spacing_m = leases
                        .iter()
                        .map(|lease| lease.key.address().spacing_m())
                        .min();
                    bridge.output.lock().unwrap().frame =
                        Some(Arc::new(DrawFrame { leases, origins }));
                }
            }
        }
        if world.settled() && current.leaves() != target.leaves() {
            let next = current.step_toward(&target, camera.eye.anchor())?;
            world.set_coverage(&next, camera.eye.anchor())?;
            current = next;
            replacement = Instant::now();
        }
        if report.elapsed() >= Duration::from_millis(16) {
            stats.target = target.leaves().len();
            stats.in_flight = world.in_flight();
            stats.retiring = world.retiring();
            stats.bytes = world.memory_bytes();
            stats.resident_bytes = world.resident_bytes();
            stats.retiring_bytes = world.retiring_bytes();
            stats.status = if world.settled() && current.leaves() == target.leaves() {
                "GPU terrain resident"
            } else {
                "Refining GPU terrain locally"
            }
            .into();
            let mut output = bridge.output.lock().unwrap();
            stats.scheduler_ms = output.stats.scheduler_ms;
            stats.draw_ms = output.stats.draw_ms;
            stats.drawn = output.stats.drawn;
            output.stats = stats.clone();
            report = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}
