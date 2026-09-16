//! Terrain generation worker; rendering consumes immutable GPU snapshots.
use crate::physical_gpu_bridge::{
    DesignHandoff, DisplayedDesign, DrawFrame, GpuBridge, GpuGeneration, GpuOutput, GpuStats,
    TerrainSubmission, local_draw_layout,
};
use bevy::prelude::*;
use procgen_realtime_pilot::{
    HeightGpuPipeline, LOCAL_GPU_WORLD_CONFIG, MAX_VOXEL_COVERAGE_LEAVES, VOXEL_BAND_MAX_SPACING_M,
    VOXEL_BAND_REQUESTED_LEAVES, VoxelChunkAddress, VoxelCoverage, VoxelGpuEvent, VoxelGpuMesher,
    VoxelGpuOutcome, VoxelGpuWorld, VoxelPosition, cap_voxel_coverage, select_height_coverage,
    select_voxel_coverage,
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
    // Both kernels are composed and compiled once for the whole worker. Only a
    // design's parameters change between revisions, so a revision costs the
    // meshes it generates rather than a second shader compilation.
    let pipelines = Pipelines {
        voxel: Arc::new(VoxelGpuMesher::new(device, queue)),
        height: Arc::new(HeightGpuPipeline::new(device)),
    };
    let mut seed = VoxelCoverage::new(vec![VoxelChunkAddress::root()])?;
    while let Some(handoff) = generate_revision(device, queue, &generation, &pipelines, seed, stop)?
    {
        generation.design = DisplayedDesign {
            revision: handoff.request.revision,
            field: handoff.request.field,
            output: Arc::new(Mutex::new(GpuOutput::default())),
        };
        seed = handoff.coverage;
    }
    Ok(())
}
/// Compiled kernels shared by every revision this worker generates.
struct Pipelines {
    voxel: Arc<VoxelGpuMesher>,
    height: Arc<HeightGpuPipeline>,
}
/// The design publication decision for one worker iteration. A design is ready
/// as soon as its height shell is. The voxel world's settlement is deliberately
/// not part of this: re-meshing a whole band after an edit takes seconds, and
/// the tiles that approximate it do not.
///
/// The tiles are the same octave height the band's density starts from, so they
/// differ from the band's surface only by the volume term, which the tiles do
/// not evaluate and which is strictly inside plus or minus its `amplitude_m`
/// (6 m in the shipped 300 km design), and by the tiles' own spacing filter.
/// That is the same disagreement the band dissolves across at its edge today.
fn should_publish(offered: bool, heights_ready: bool) -> bool {
    !offered && heights_ready
}
/// Start the new world on the coverage the previous revision reached, handed
/// over in one `set_coverage` of its capped band, so the band arrives as one
/// replacement group per closed region rather than one step per chunk.
/// An empty band is the whole coverage above the spacing cap, which the world
/// already holds; a new world has nothing resident.
fn seed_band(
    world: &mut VoxelGpuWorld,
    seed: VoxelCoverage,
    camera: VoxelPosition,
) -> Result<(VoxelCoverage, VoxelCoverage), String> {
    let band = cap_voxel_coverage(&seed, VOXEL_BAND_MAX_SPACING_M).map_err(|e| e.to_string())?;
    if !band.leaves().is_empty() {
        world
            .set_coverage(&band, camera)
            .map_err(|e| e.to_string())?;
    }
    Ok((seed, band))
}
fn generate_revision(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bridge: &GpuGeneration,
    pipelines: &Pipelines,
    seed: VoxelCoverage,
    stop: &AtomicBool,
) -> Result<Option<DesignHandoff>, Box<dyn std::error::Error>> {
    let revision = bridge.design.revision;
    let mut offered = revision == 0;
    let mut world = VoxelGpuWorld::with_mesher(
        device,
        queue,
        Arc::clone(&pipelines.voxel),
        Arc::clone(&bridge.design.field),
        LOCAL_GPU_WORLD_CONFIG,
    )?;
    let mut heights =
        crate::physical_height::HeightStream::new(device, Arc::clone(&pipelines.height), bridge);
    let mut selected_at = None;
    let mut height_target = Vec::new();
    // A coverage that will not fit its budget is an ordinary outcome of flying
    // somewhere the band cannot be afforded, not a broken stream. Hold the last
    // good target and band, say so in the panel, and try again on the next
    // camera move instead of ending the generation worker.
    let mut held: Option<String> = None;
    // The worker walks whole balanced partitions and hands the world the band
    // each one draws, because a coverage step only redivides the volume it
    // already holds. `band` is what the world was last told to hold.
    let (mut current, mut band) = match seed_band(
        &mut world,
        seed,
        bridge.shared.camera.lock().unwrap().eye.anchor(),
    ) {
        Ok(seeded) => seeded,
        Err(error) => {
            held = Some(error);
            let root = VoxelCoverage::new(vec![VoxelChunkAddress::root()])?;
            let band = cap_voxel_coverage(&root, VOXEL_BAND_MAX_SPACING_M)?;
            (root, band)
        }
    };
    // Nothing is stepped until the first selection runs, which happens on the
    // first pass of the loop below.
    let mut target = current.clone();
    let mut target_band = band.clone();
    let mut stats = GpuStats::default();
    let mut replacement = Instant::now();
    let mut report = Instant::now();
    let mut dirty = false;
    while !stop.load(Ordering::Relaxed) {
        // A pending request stops new steps below, so the voxel work in flight
        // is at most one replacement group. Waiting for the world to settle
        // instead would hold the edit behind the rest of the band's walk.
        if world.in_flight() == 0
            && heights.idle()
            && let Some(handoff) = bridge.shared.designs.lock().unwrap().take_request(&current)
        {
            return Ok(Some(handoff));
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
        // Captured once, because the step below makes the world unsettled again
        // and the publication decision must describe the top of this iteration.
        let settled = world.settled();
        if settled {
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
        // Tiles first, band behind: this is the intended order, not a
        // shortcut. The new revision's `GpuOutput.frame` stays `None` until its
        // first publication event, so the renderer draws height tiles alone
        // until the band's first group arrives and then dissolves into them at
        // the band edge, exactly as it does at startup.
        if should_publish(offered, heights.ready()) {
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
                None if settled && current.leaves() == target.leaves() => {
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

#[cfg(test)]
mod tests {
    use super::should_publish;
    #[test]
    fn a_design_publishes_as_soon_as_its_height_tiles_are_ready() {
        // The voxel band is not a precondition. It re-meshes behind the tiles,
        // which is seconds of work the viewer no longer waits through.
        assert!(should_publish(false, true));
        assert!(!should_publish(false, false), "no height shell yet");
        assert!(!should_publish(true, true), "already offered");
    }
}
