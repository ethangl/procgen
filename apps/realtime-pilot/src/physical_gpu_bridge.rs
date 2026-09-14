//! Renderer-owned messages, immutable snapshots, and visual transition settings.
use bevy::prelude::Resource;
use procgen_realtime_pilot::HeightTile;
use procgen_realtime_pilot::{HeightFilter, MeterPosition, PlanetDesignField, VoxelGpuLease};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, mpsc},
    time::Instant,
};
pub const SURFACE_BLEND_SECONDS: f32 = 0.25;
pub const LOCAL_BLEND_M: f32 = 16.0;
pub const LOCAL_SURFACE_BIAS_M: f32 = 1.0;
pub struct HeightFrame {
    pub tiles: BTreeMap<HeightTile, Arc<wgpu::Buffer>>,
    pub born: f32,
    pub filter: HeightFilter,
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
pub struct DrawFrame {
    pub leases: Vec<VoxelGpuLease>,
    pub bounds: [[i32; 3]; 2],
    pub born: f32,
    pub origins: wgpu::Buffer,
}
#[derive(Default, Clone)]
pub struct GpuStats {
    pub status: String,
    pub height_tiles: usize,
    pub height_bytes: u64,
    pub surface_target_bytes: u64,
    pub height_update_ms: f64,
    pub height_build_ms: f64,
    pub height_wait_ms: f64,
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
    pub height: Option<Arc<HeightFrame>>,
    pub previous_height: Option<Arc<HeightFrame>>,
    pub stats: GpuStats,
}
pub enum TerrainSubmission {
    Voxel(procgen_realtime_pilot::VoxelGpuSubmission),
    Height(HeightSubmission),
}
impl TerrainSubmission {
    pub fn submit(self, queue: &wgpu::Queue) {
        match self {
            Self::Voxel(s) => s.submit(queue),
            Self::Height(s) => s.submit(queue),
        }
    }
}
#[derive(Resource, Clone)]
pub struct GpuBridge {
    pub submissions: Arc<Mutex<std::sync::mpsc::Receiver<TerrainSubmission>>>,
    pub submit: std::sync::mpsc::Sender<TerrainSubmission>,
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
pub struct DesignPublication {
    pub design: DisplayedDesign,
    pub collision: Option<crate::physical_jobs::CollisionResult>,
}
#[derive(Default)]
pub struct DesignExchange {
    pub failure: Option<String>,
    revision: u64,
    pub request: Option<DesignRequest>,
    ready: Option<DesignPublication>,
}
impl DesignExchange {
    pub fn invalidate(&mut self, revision: u64) {
        self.revision = revision;
        self.request = None;
        self.ready = None;
    }
    pub fn publish(&mut self, publication: DesignPublication) -> bool {
        if publication.design.revision != self.revision {
            return false;
        }
        self.ready = Some(publication);
        true
    }
    pub fn take_ready(&mut self) -> Option<DesignPublication> {
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
    fn publication(revision: u64) -> DesignPublication {
        DesignPublication {
            design: DisplayedDesign {
                revision,
                field: Arc::new(
                    procgen_realtime_pilot::PlanetDesignConfig::starter(revision)
                        .validate()
                        .unwrap(),
                ),
                output: Arc::new(Mutex::new(GpuOutput::default())),
            },
            collision: None,
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
        assert_eq!(exchange.take_ready().unwrap().design.field.config().seed, 3);
        assert!(exchange.take_ready().is_none());
    }
}
