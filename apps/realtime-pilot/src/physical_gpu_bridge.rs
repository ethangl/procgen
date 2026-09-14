//! Renderer-owned messages, immutable snapshots, and visual transition settings.
use bevy::prelude::Resource;
use procgen_realtime_pilot::HeightTile;
use procgen_realtime_pilot::{MeterPosition, PlanetDesignField};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, mpsc},
    time::Instant,
};
pub const SURFACE_BLEND_SECONDS: f32 = 0.25;
pub struct ResidentHeightTile {
    pub buffer: wgpu::Buffer,
    pub bounds: crate::physical_visibility::HeightBounds,
}
pub struct HeightFrame {
    pub tiles: BTreeMap<HeightTile, Arc<ResidentHeightTile>>,
    pub born: f32,
}
pub struct HeightSubmission {
    pub commands: wgpu::CommandBuffer,
    pub completed: mpsc::Sender<()>,
}
impl HeightSubmission {
    pub fn submit(self, queue: &wgpu::Queue) {
        queue.submit([self.commands]);
        queue.on_submitted_work_done(move || {
            let _ = self.completed.send(());
        });
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExplorationBackend {
    Gpu,
    Cpu,
}
#[derive(Clone, Copy)]
pub struct GpuCamera {
    pub eye: MeterPosition,
}
#[derive(Default, Clone)]
pub struct GpuStats {
    pub status: String,
    pub height_tiles: usize,
    pub height_drawn_tiles: usize,
    pub height_tested_tiles: usize,
    pub height_generated_tiles: u64,
    pub height_reused_tiles: u64,
    pub height_bytes: u64,
    pub surface_target_bytes: u64,
    pub height_update_ms: f64,
    pub height_build_ms: f64,
    pub height_wait_ms: f64,
    pub selection_ms: f64,
    pub scheduler_ms: f64,
    pub draw_ms: f64,
}
#[derive(Default)]
pub struct GpuOutput {
    pub height: Option<Arc<HeightFrame>>,
    pub previous_height: Option<Arc<HeightFrame>>,
    pub stats: GpuStats,
}
#[derive(Resource, Clone)]
pub struct GpuBridge {
    pub submissions: Arc<Mutex<std::sync::mpsc::Receiver<HeightSubmission>>>,
    pub submit: std::sync::mpsc::Sender<HeightSubmission>,
    pub start: Instant,
    pub display: Arc<Mutex<DisplayedDesign>>,
    pub designs: Arc<Mutex<DesignExchange>>,
    pub camera: Arc<Mutex<GpuCamera>>,
}
impl GpuBridge {
    pub fn new(field: Arc<PlanetDesignField>, camera: GpuCamera) -> Self {
        let (submit, submissions) = std::sync::mpsc::channel();
        let output = Arc::new(Mutex::new(GpuOutput::default()));
        Self {
            display: Arc::new(Mutex::new(DisplayedDesign {
                revision: 0,
                field: Arc::clone(&field),
                output: Arc::clone(&output),
            })),
            designs: Arc::new(Mutex::new(DesignExchange::default())),
            start: Instant::now(),
            submit,
            submissions: Arc::new(Mutex::new(submissions)),
            camera: Arc::new(Mutex::new(camera)),
        }
    }
}

#[derive(Clone)]
pub struct DisplayedDesign {
    pub revision: u64,
    pub field: Arc<PlanetDesignField>,
    pub output: Arc<Mutex<GpuOutput>>,
}
pub struct DesignRequest {
    pub revision: u64,
    pub field: Arc<PlanetDesignField>,
}
#[derive(Default)]
pub struct DesignExchange {
    pub failure: Option<String>,
    revision: u64,
    pub request: Option<DesignRequest>,
    ready: Option<DisplayedDesign>,
}
impl DesignExchange {
    pub fn invalidate(&mut self, revision: u64) {
        self.revision = revision;
        self.request = None;
        self.ready = None;
    }
    pub fn publish(&mut self, publication: DisplayedDesign) -> bool {
        if publication.revision != self.revision {
            return false;
        }
        self.ready = Some(publication);
        true
    }
    pub fn take_ready(&mut self) -> Option<DisplayedDesign> {
        self.ready.take()
    }
}

/// One worker-owned generation. Shared renderer state never retains retired designs.
pub struct GpuGeneration {
    pub shared: GpuBridge,
    pub design: DisplayedDesign,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn publication(revision: u64) -> DisplayedDesign {
        DisplayedDesign {
            revision,
            field: Arc::new(
                procgen_realtime_pilot::PlanetDesignConfig::starter(revision)
                    .validate()
                    .unwrap(),
            ),
            output: Arc::new(Mutex::new(GpuOutput::default())),
        }
    }
    #[test]
    fn edits_discard_completed_and_in_flight_generations_even_after_reverting() {
        let mut exchange = DesignExchange::default();
        exchange.invalidate(1);
        assert!(exchange.publish(publication(1)));
        exchange.invalidate(2);
        assert!(exchange.take_ready().is_none());
        assert!(!exchange.publish(publication(1)));
        exchange.invalidate(3);
        assert!(!exchange.publish(publication(2)));
        assert!(exchange.publish(publication(3)));
        assert_eq!(exchange.take_ready().unwrap().field.config().seed, 3);
        assert!(exchange.take_ready().is_none());
    }
}
