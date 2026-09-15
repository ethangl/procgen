//! Renderer-owned messages, immutable snapshots, and visual transition settings.
use bevy::prelude::Resource;
use procgen_realtime_pilot::HeightTile;
use procgen_realtime_pilot::{MeterPosition, PlanetDesignField, VoxelChunkAddress, VoxelGpuLease};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, mpsc},
    time::Instant,
};
pub const SURFACE_BLEND_SECONDS: f32 = 0.25;
/// Width of the band over which local voxel coverage dissolves into height tiles.
pub const LOCAL_BLEND_M: f32 = 16.0;
/// How far the height surface is pushed below the local region it overlaps.
pub const LOCAL_SURFACE_BIAS_M: f32 = 1.0;
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
#[derive(Clone, Copy)]
pub struct GpuCamera {
    pub eye: MeterPosition,
}
/// One immutable local voxel snapshot: leases pinning the resident slots, their
/// per-instance origin records, and the integer box the whole region occupies.
pub struct DrawFrame {
    pub leases: Vec<VoxelGpuLease>,
    pub bounds: [[i32; 3]; 2],
    pub born: f32,
    pub origins: wgpu::Buffer,
}
/// Instance stride of the origin buffer: three integer meters and the LOD word.
pub const LOCAL_ORIGIN_BYTES: usize = 16;
/// The CPU half of a `DrawFrame`: the union of the chunk boxes and the packed
/// instance records. Buffer creation is the only part that needs a device.
pub fn local_draw_layout(addresses: &[VoxelChunkAddress]) -> ([[i32; 3]; 2], Vec<u8>) {
    let mut bounds = [[i32::MAX; 3], [i32::MIN; 3]];
    let mut origins = vec![0u8; addresses.len().max(1) * LOCAL_ORIGIN_BYTES];
    for (i, address) in addresses.iter().enumerate() {
        let p = address.origin();
        for (axis, value) in [p.x_m, p.y_m, p.z_m].into_iter().enumerate() {
            bounds[0][axis] = bounds[0][axis].min(value);
            bounds[1][axis] = bounds[1][axis].max(value + address.span_m());
        }
        origins[i * LOCAL_ORIGIN_BYTES..(i + 1) * LOCAL_ORIGIN_BYTES]
            .copy_from_slice(bytemuck::cast_slice(&[p.x_m, p.y_m, p.z_m, 0]));
    }
    if addresses.is_empty() {
        bounds = [[0; 3]; 2];
    }
    (bounds, origins)
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
    pub resident: usize,
    pub finest_spacing_m: Option<i32>,
    pub drawn: usize,
    pub target: usize,
    pub in_flight: usize,
    pub retiring: usize,
    pub bytes: u64,
    pub resident_bytes: u64,
    pub retiring_bytes: u64,
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
/// Encoded work handed to the owner of the render queue, from either stream.
pub enum TerrainSubmission {
    Voxel(procgen_realtime_pilot::VoxelGpuSubmission),
    Height(HeightSubmission),
}
impl TerrainSubmission {
    pub fn submit(self, queue: &wgpu::Queue) {
        match self {
            Self::Voxel(submission) => submission.submit(queue),
            Self::Height(submission) => submission.submit(queue),
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
    fn draw_frame_bounds_are_the_union_of_chunk_boxes_with_one_record_per_lease() {
        use procgen_realtime_pilot::{VOXEL_CHUNK_CELLS, VoxelPosition};
        let address = |x, y, z| {
            procgen_realtime_pilot::VoxelChunkAddress::containing(
                VoxelPosition {
                    x_m: x,
                    y_m: y,
                    z_m: z,
                },
                0,
            )
            .unwrap()
        };
        let span = VOXEL_CHUNK_CELLS;
        let addresses = [address(0, 0, 0), address(span, -span, 2 * span)];
        let (bounds, origins) = local_draw_layout(&addresses);
        assert_eq!(bounds, [[0, -span, 0], [2 * span, span, 3 * span]]);
        assert_eq!(origins.len(), addresses.len() * LOCAL_ORIGIN_BYTES);
        let records: &[i32] = bytemuck::cast_slice(&origins);
        assert_eq!(records, [0, 0, 0, 0, span, -span, 2 * span, 0]);
        // An empty region must still describe a finite box and a bindable buffer.
        let (bounds, origins) = local_draw_layout(&[]);
        assert_eq!(bounds, [[0; 3]; 2]);
        assert_eq!(origins.len(), LOCAL_ORIGIN_BYTES);
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
