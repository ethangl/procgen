//! Terrain generation worker; rendering consumes immutable GPU snapshots.
use crate::physical_gpu_bridge::{
    DesignRequest, DisplayedDesign, DrawFrame, GpuBridge, GpuGeneration, GpuOutput, GpuStats,
    TerrainSubmission, local_draw_layout,
};
use bevy::prelude::*;
use procgen_realtime_pilot::{
    LOCAL_GPU_WORLD_CONFIG, MAX_VOXEL_COVERAGE_LEAVES, VOXEL_BAND_MAX_SPACING_M,
    VOXEL_BAND_REQUESTED_LEAVES, VoxelChunkAddress, VoxelCoverage, VoxelGpuEvent, VoxelGpuOutcome,
    VoxelGpuWorld, cap_voxel_coverage, select_height_coverage, select_voxel_coverage,
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
    // The worker walks whole balanced partitions and hands the world the band
    // each one draws, because a coverage step only redivides the volume it
    // already holds. `band` is what the world was last told to hold.
    let mut target = VoxelCoverage::new(vec![VoxelChunkAddress::root()])?;
    let mut target_band = cap_voxel_coverage(&target, VOXEL_BAND_MAX_SPACING_M)?;
    let mut current = target.clone();
    let mut band = target_band.clone();
    // A coverage that will not fit its budget is an ordinary outcome of flying
    // somewhere the band cannot be afforded, not a broken stream. Hold the last
    // good target and band, say so in the panel, and try again on the next
    // camera move instead of ending the generation worker.
    let mut held: Option<String> = None;
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
        // Band selection stays on `elevation_m`. This clearance only sets how
        // far the camera may drift before coverage is reselected; a few meters
        // of volume displacement cannot change which chunks are wanted, and the
        // voxel selection below is measured against the height surface too.
        let clearance = camera.eye.altitude_m(bridge.design.field.config().radius_m) as f32
            - bridge
                .design
                .field
                .elevation_m(camera.eye.direction(), 0.0)?;
        let threshold = (clearance * 0.1).clamp(8.0, 100_000.0);
        if selected_at.is_none_or(|p| camera.eye.relative_to(p).length() > threshold) {
            let start = Instant::now();
            height_target = select_height_coverage(&bridge.design.field, camera.eye.anchor());
            match select_voxel_coverage(
                &bridge.design.field,
                camera.eye.anchor(),
                VOXEL_BAND_REQUESTED_LEAVES,
                MAX_VOXEL_COVERAGE_LEAVES,
            )
            .and_then(|selected| {
                let capped = cap_voxel_coverage(&selected, VOXEL_BAND_MAX_SPACING_M)?;
                Ok((selected, capped))
            }) {
                Ok((selected, capped)) => {
                    target = selected;
                    target_band = capped;
                    held = None;
                }
                Err(error) => held = Some(error.to_string()),
            }
            // Recorded even when the selection was refused, so a camera sitting
            // still does not re-run it on every pass of this loop.
            selected_at = Some(camera.eye.anchor());
            stats.selection_ms = start.elapsed().as_secs_f64() * 1000.0;
        }
        let replacing = bridge.shared.designs.lock().unwrap().request.is_some();
        heights.update(device, bridge, &height_target, !replacing);
        // A failed update defers its events to the next one; the loop is the same.
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
                VoxelGpuEvent::Published(_) => dirty = true,
            }
        }
        if world.settled() {
            if dirty {
                let leases = world.resident_leases();
                let addresses: Vec<_> = leases.iter().map(|l| l.key.address()).collect();
                let layout = local_draw_layout(
                    &addresses,
                    bridge.design.field.config().volume.active_amplitude_m(),
                );
                let origins = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("local voxel origins"),
                    contents: &layout.origins,
                    usage: wgpu::BufferUsages::VERTEX,
                });
                stats.publication_ms = replacement.elapsed().as_secs_f64() * 1000.0;
                stats.resident = leases.len();
                let spacings = addresses.iter().map(|a| a.spacing_m());
                stats.finest_spacing_m = spacings.clone().min();
                stats.coarsest_spacing_m = spacings.max();
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
                    bounds: layout.bounds,
                    blend_m: layout.blend_m,
                    bias_m: layout.bias_m,
                    born,
                }));
                dirty = false;
            }
            // One coverage step per settled world, and one closed replacement
            // group per step that changes the band, as the streaming audit
            // does. Steps that only redivide chunks coarser than the cap leave
            // the band alone and cost the GPU nothing.
            if !replacing && current.leaves() != target.leaves() {
                match current
                    .step_toward(&target, camera.eye.anchor())
                    .and_then(|next| {
                        let capped = cap_voxel_coverage(&next, VOXEL_BAND_MAX_SPACING_M)?;
                        Ok((next, capped))
                    }) {
                    Ok((next, capped)) => {
                        current = next;
                        held = None;
                        if capped.leaves() != band.leaves() {
                            // A refused replacement leaves `band` behind
                            // `current`. The walk still advances, so it ends,
                            // and the world keeps drawing the last coverage it
                            // actually holds.
                            match world.set_coverage(&capped, camera.eye.anchor()) {
                                Ok(()) => {
                                    band = capped;
                                    replacement = Instant::now();
                                }
                                Err(error) => held = Some(error.to_string()),
                            }
                        }
                    }
                    Err(error) => held = Some(error.to_string()),
                }
            }
        }
        // The band refines live afterwards; waiting for the final target would
        // hold the first design behind hundreds of coverage steps.
        if !offered && world.settled() && heights.ready() {
            let publication = DisplayedDesign {
                revision,
                field: Arc::clone(&bridge.design.field),
                output: Arc::clone(&bridge.design.output),
            };
            bridge.shared.designs.lock().unwrap().publish(publication);
            offered = true;
        }
        if report.elapsed() >= Duration::from_millis(16) {
            stats.current = band.leaves().len();
            stats.target = target_band.leaves().len();
            stats.in_flight = world.in_flight();
            stats.retiring = world.retiring();
            stats.height_bytes = heights.allocated_bytes(bridge);
            stats.bytes = world.memory_bytes() + stats.height_bytes;
            stats.resident_bytes = world.resident_bytes();
            stats.retiring_bytes = world.retiring_bytes();
            stats.status = match &held {
                Some(error) => format!("Holding the local voxel band: {error}"),
                None if world.settled() && current.leaves() == target.leaves() => {
                    "GPU height terrain and the local voxel band resident".into()
                }
                None => "Refining the local voxel band".into(),
            };
            let mut output = bridge.design.output.lock().unwrap();
            stats.height_tiles = output.height.as_ref().map_or(0, |h| h.tiles.len());
            stats.height_generated_tiles = output.stats.height_generated_tiles;
            stats.height_reused_tiles = output.stats.height_reused_tiles;
            stats.height_update_ms = output.stats.height_update_ms;
            stats.height_build_ms = output.stats.height_build_ms;
            stats.height_wait_ms = output.stats.height_wait_ms;
            stats.scheduler_ms = output.stats.scheduler_ms;
            stats.draw_ms = output.stats.draw_ms;
            stats.height_drawn_tiles = output.stats.height_drawn_tiles;
            stats.height_tested_tiles = output.stats.height_tested_tiles;
            stats.surface_target_bytes = output.stats.surface_target_bytes;
            stats.drawn = output.stats.drawn;
            output.stats = stats.clone();
            report = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(None)
}
