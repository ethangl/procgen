//! Terrain generation worker; rendering consumes immutable GPU snapshots.
use crate::physical_gpu_bridge::{
    DesignRequest, DisplayedDesign, GpuBridge, GpuGeneration, GpuOutput, GpuStats,
};
use bevy::prelude::*;
use procgen_realtime_pilot::select_height_coverage;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

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
    pub fn start(device: wgpu::Device, bridge: GpuBridge) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                generate(&device, &bridge, &cancel)
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
    bridge: &GpuBridge,
    stop: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut generation = GpuGeneration {
        shared: bridge.clone(),
        design: bridge.display.lock().unwrap().clone(),
    };
    while let Some(request) = generate_revision(device, &generation, stop)? {
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
    bridge: &GpuGeneration,
    stop: &AtomicBool,
) -> Result<Option<DesignRequest>, Box<dyn std::error::Error>> {
    let revision = bridge.design.revision;
    let mut offered = revision == 0;
    let mut heights = crate::physical_height::HeightStream::new(device, bridge);
    let mut selected_at = None;
    let mut height_target = Vec::new();
    let mut stats = GpuStats::default();
    let mut report = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        // Drain the bounded GPU work before replacing its owner. No UI thread joins.
        if heights.idle()
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
            selected_at = Some(camera.eye.anchor());
            stats.selection_ms = start.elapsed().as_secs_f64() * 1000.0;
        }
        let replacing = bridge.shared.designs.lock().unwrap().request.is_some();
        heights.update(device, bridge, &height_target, !replacing);
        if !offered && heights.ready() {
            let publication = DisplayedDesign {
                revision,
                field: Arc::clone(&bridge.design.field),
                output: Arc::clone(&bridge.design.output),
            };
            bridge.shared.designs.lock().unwrap().publish(publication);
            offered = true;
        }
        if report.elapsed() >= Duration::from_millis(16) {
            stats.height_bytes = heights.allocated_bytes(bridge);
            stats.status = "GPU height terrain resident".into();
            let mut output = bridge.design.output.lock().unwrap();
            stats.height_tiles = output.height.as_ref().map_or(0, |h| h.tiles.len());
            stats.height_generated_tiles = output.stats.height_generated_tiles;
            stats.height_reused_tiles = output.stats.height_reused_tiles;
            stats.height_update_ms = output.stats.height_update_ms;
            stats.height_build_ms = output.stats.height_build_ms;
            stats.height_wait_ms = output.stats.height_wait_ms;
            stats.scheduler_ms = output.stats.scheduler_ms;
            stats.draw_ms = output.stats.draw_ms;
            stats.surface_target_bytes = output.stats.surface_target_bytes;
            output.stats = stats.clone();
            report = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(None)
}
