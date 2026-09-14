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
    pub field: Arc<PlanetDesignField>,
    pub camera: Arc<Mutex<GpuCamera>>,
    pub output: Arc<Mutex<GpuOutput>>,
}
impl GpuBridge {
    pub fn new(field: Arc<PlanetDesignField>, camera: GpuCamera) -> Self {
        let (submit, submissions) = std::sync::mpsc::channel();
        Self {
            start: Instant::now(),
            submit,
            submissions: Arc::new(Mutex::new(submissions)),
            field,
            camera: Arc::new(Mutex::new(camera)),
            output: Arc::new(Mutex::new(GpuOutput::default())),
        }
    }
}
