//! Terrain generation worker; rendering consumes immutable GPU snapshots.
use crate::physical_gpu_bridge::{
    DesignPublication, DesignRequest, DisplayedDesign, DrawFrame, GpuBridge, GpuGeneration,
    GpuOutput, GpuStats, TerrainSubmission,
};
use bevy::prelude::*;
use procgen_realtime_pilot::{
    LOCAL_GPU_WORLD_CONFIG, VoxelCoverage, VoxelGpuEvent, VoxelGpuOutcome, VoxelGpuWorld,
    local_voxel_coverage, select_height_coverage,
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
            bridge.designs.lock().unwrap().failure = Some(format!(
                "GPU stream stopped: {error}. Restart the viewer to retry."
            ));
            let displayed = bridge.display.lock().unwrap().clone();
            displayed.output.lock().unwrap().stats.status = format!("GPU stream stopped: {error}");
            error!("GPU stream stopped: {error}");
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
    let mut generation = GpuGeneration {
        shared: bridge.clone(),
        design: bridge.display.lock().unwrap().clone(),
    };
    while let Some(request) = generate_revision(device, queue, &generation, stop)? {
        generation.design = DisplayedDesign {
            revision: request.revision,
            field: request.field,
            output: Arc::new(Mutex::new(GpuOutput::default())),
        };
    }
    Ok(())
}
fn generate_revision(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bridge: &GpuGeneration,
    stop: &AtomicBool,
) -> Result<Option<DesignRequest>, Box<dyn std::error::Error>> {
    let revision = bridge.design.revision;
    let mut offered = revision == 0;
    let mut world = VoxelGpuWorld::new(
        device,
        queue,
        Arc::clone(&bridge.design.field),
        LOCAL_GPU_WORLD_CONFIG,
    )?;
    let mut heights = crate::physical_height::HeightStream::new(device, bridge);
    let mut selected_at = None;
    let mut height_target = Vec::new();
    let mut height_filter = procgen_realtime_pilot::HeightFilter::new(
        &bridge.design.field,
        bridge.shared.camera.lock().unwrap().eye.anchor(),
    );
    let mut target = VoxelCoverage::new(Vec::new())?;
    let mut current = target.clone();
    let mut stats = GpuStats::default();
    let mut replacement = Instant::now();
    let mut report = Instant::now();
    let mut dirty = false;
    while !stop.load(Ordering::Relaxed) {
        // Drain the bounded GPU work before replacing its owner. No UI thread joins.
        if world.settled()
            && heights.idle()
            && let Some(request) = bridge.shared.designs.lock().unwrap().request.take()
        {
            return Ok(Some(request));
        }
        let camera = *bridge.shared.camera.lock().unwrap();
        let clearance = camera.eye.altitude_m(bridge.design.field.config().radius_m) as f32
            - bridge
                .design
                .field
                .elevation_m(camera.eye.direction(), 0.0)?;
        let threshold = (clearance * 0.1).clamp(8.0, 100_000.0);
        if selected_at.is_none_or(|p| camera.eye.relative_to(p).length() > threshold) {
            let start = Instant::now();
            height_target = select_height_coverage(&bridge.design.field, camera.eye.anchor());
            height_filter = procgen_realtime_pilot::HeightFilter::new(
                &bridge.design.field,
                camera.eye.anchor(),
            );
            target = local_voxel_coverage(&bridge.design.field, camera.eye.anchor())?;
            selected_at = Some(camera.eye.anchor());
            stats.selection_ms = start.elapsed().as_secs_f64() * 1000.0;
        }
        let replacing = bridge.shared.designs.lock().unwrap().request.is_some();
        heights.update(device, bridge, &height_target, height_filter, !replacing);
        for event in world.update_with_submit(|submission| {
            let _ = bridge
                .shared
                .submit
                .send(TerrainSubmission::Voxel(submission));
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
                    dirty = true;
                }
            }
        }
        if world.settled() {
            if dirty {
                let leases = world.resident_leases();
                let mut bounds = [[i32::MAX; 3], [i32::MIN; 3]];
                let mut origins = vec![0u8; leases.len().max(1) * 16];
                for (i, lease) in leases.iter().enumerate() {
                    let p = lease.key.address().origin();
                    let xyz = [p.x_m, p.y_m, p.z_m];
                    for (axis, &value) in xyz.iter().enumerate() {
                        bounds[0][axis] = bounds[0][axis].min(value);
                        bounds[1][axis] = bounds[1][axis].max(value + lease.key.address().span_m());
                    }
                    origins[i * 16..i * 16 + 16]
                        .copy_from_slice(bytemuck::cast_slice(&[p.x_m, p.y_m, p.z_m, 0]));
                }
                if leases.is_empty() {
                    bounds = [[0; 3]; 2];
                }
                let origins = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("local voxel origins"),
                    contents: &origins,
                    usage: wgpu::BufferUsages::VERTEX,
                });
                stats.publication_ms = replacement.elapsed().as_secs_f64() * 1000.0;
                stats.resident = leases.len();
                stats.finest_spacing_m = leases.first().map(|l| l.key.address().spacing_m());
                let mut output = bridge.design.output.lock().unwrap();
                // Reusing nearby chunks must not restart the whole region's fade.
                let born = match &output.frame {
                    Some(previous) if !previous.leases.is_empty() && !leases.is_empty() => {
                        previous.born
                    }
                    _ => bridge.shared.start.elapsed().as_secs_f32(),
                };
                output.frame = Some(Arc::new(DrawFrame {
                    leases,
                    origins,
                    bounds,
                    born,
                }));
                dirty = false;
            }
            if !replacing && current.leaves() != target.leaves() {
                world.set_coverage(&target, camera.eye.anchor())?;
                current = target.clone();
                replacement = Instant::now();
            }
        }
        if !offered && world.settled() && current.leaves() == target.leaves() && heights.ready() {
            let collision = if clearance < 32.0 {
                let start = Instant::now();
                let patch = procgen_realtime_pilot::VoxelCollision::build(
                    &bridge.design.field,
                    camera.eye.anchor(),
                    stop,
                )?;
                Some(crate::physical_jobs::CollisionResult {
                    patch,
                    seconds: start.elapsed().as_secs_f32(),
                })
            } else {
                None
            };
            let publication = DesignPublication {
                design: DisplayedDesign {
                    revision,
                    field: Arc::clone(&bridge.design.field),
                    output: Arc::clone(&bridge.design.output),
                },
                collision,
            };
            bridge.shared.designs.lock().unwrap().publish(publication);
            offered = true;
        }
        if report.elapsed() >= Duration::from_millis(16) {
            stats.target = target.leaves().len();
            stats.in_flight = world.in_flight();
            stats.retiring = world.retiring();
            stats.height_bytes = heights.allocated_bytes(bridge);
            stats.bytes = world.memory_bytes() + stats.height_bytes;
            stats.resident_bytes = world.resident_bytes();
            stats.retiring_bytes = world.retiring_bytes();
            stats.status = if world.settled() && current.leaves() == target.leaves() {
                "GPU height terrain and local voxels resident"
            } else {
                "Updating local GPU voxels"
            }
            .into();
            let mut output = bridge.design.output.lock().unwrap();
            stats.height_tiles = output.height.as_ref().map_or(0, |h| h.tiles.len());
            stats.height_update_ms = output.stats.height_update_ms;
            stats.height_build_ms = output.stats.height_build_ms;
            stats.height_wait_ms = output.stats.height_wait_ms;
            stats.scheduler_ms = output.stats.scheduler_ms;
            stats.draw_ms = output.stats.draw_ms;
            stats.surface_target_bytes = output.stats.surface_target_bytes;
            stats.drawn = output.stats.drawn;
            output.stats = stats.clone();
            report = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(None)
}
