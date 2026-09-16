//! Renderer-owned messages, immutable snapshots, and visual transition settings.
use bevy::prelude::Resource;
use procgen_realtime_pilot::HeightTile;
use procgen_realtime_pilot::{
    MeterPosition, PlanetDesignField, VoxelChunkAddress, VoxelCoverage, VoxelGpuLease,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, mpsc},
    time::Instant,
};
pub const SURFACE_BLEND_SECONDS: f32 = 0.25;
/// Narrowest band over which local voxel coverage dissolves into height tiles.
/// A wider band follows the coarsest resident chunk; see `LocalDrawLayout`.
pub const LOCAL_BLEND_M: f32 = 16.0;
/// Smallest distance the height surface is pushed below the region it overlaps.
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
    /// Dissolve width and height-surface bias for this band's mixed spacings.
    pub blend_m: f32,
    pub bias_m: f32,
}
/// Instance stride of the origin buffer: three integer meters and the LOD word.
pub const LOCAL_ORIGIN_BYTES: usize = 16;
/// The CPU half of a `DrawFrame`: the union of the chunk boxes, the packed
/// instance records, and the two band-dependent surface-join distances.
/// Buffer creation is the only part that needs a device.
pub struct LocalDrawLayout {
    pub bounds: [[i32; 3]; 2],
    pub origins: Vec<u8>,
    pub blend_m: f32,
    pub bias_m: f32,
}
/// The dissolve follows the coarsest chunk in the band, so the edge fades over
/// a whole coarse chunk rather than a fraction of one. The height surface drops
/// by that chunk's cell spacing plus the volume term's amplitude. Both keep
/// their old fixed values as floors, and `volume_amplitude_m` is zero when the
/// term is disabled, which restores the old rule exactly.
///
/// The amplitude part is now larger than it needs to be, and is kept unchanged
/// on purpose. Height tiles evaluate the projected surface, so the two surfaces
/// no longer differ by the whole amplitude: they differ by the projection's
/// first-order error, measured at 2.375 m worst case for the 300 km design's
/// 6 m term (`the_projected_far_field_surface_tracks_the_band_crossing` in
/// voxel_density.rs). That measured number, not the amplitude, is the value a
/// later tune should use. The spacing part remains provisional for its own
/// reason: one coarse cell is a guess at how far a 16 m extraction can stand
/// above the height surface, not a measured bound, and it will need route
/// evidence or a per-chunk bias instead.
pub fn local_draw_layout(
    addresses: &[VoxelChunkAddress],
    volume_amplitude_m: f32,
) -> LocalDrawLayout {
    let mut bounds = [[i32::MAX; 3], [i32::MIN; 3]];
    let mut origins = vec![0u8; addresses.len().max(1) * LOCAL_ORIGIN_BYTES];
    let mut coarsest = None::<VoxelChunkAddress>;
    for (i, address) in addresses.iter().enumerate() {
        let p = address.origin();
        for (axis, value) in [p.x_m, p.y_m, p.z_m].into_iter().enumerate() {
            bounds[0][axis] = bounds[0][axis].min(value);
            bounds[1][axis] = bounds[1][axis].max(value + address.span_m());
        }
        origins[i * LOCAL_ORIGIN_BYTES..(i + 1) * LOCAL_ORIGIN_BYTES].copy_from_slice(
            bytemuck::cast_slice(&[p.x_m, p.y_m, p.z_m, i32::from(address.lod())]),
        );
        if coarsest.is_none_or(|c| c.lod() < address.lod()) {
            coarsest = Some(*address);
        }
    }
    if addresses.is_empty() {
        bounds = [[0; 3]; 2];
    }
    LocalDrawLayout {
        bounds,
        origins,
        blend_m: coarsest.map_or(LOCAL_BLEND_M, |c| (c.span_m() as f32).max(LOCAL_BLEND_M)),
        bias_m: coarsest.map_or(LOCAL_SURFACE_BIAS_M, |c| {
            (c.spacing_m() as f32 + volume_amplitude_m).max(LOCAL_SURFACE_BIAS_M)
        }),
    }
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
    pub coarsest_spacing_m: Option<i32>,
    pub drawn: usize,
    /// Leaves in the coverage the world is currently settling on, and in the
    /// band selected for the camera. They differ while the band steps.
    pub current: usize,
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
/// What a finishing revision hands its successor: the request that replaces it
/// and the voxel coverage its band had reached. The next world starts its band
/// from that coverage rather than from the root chunk, because the band's
/// selection follows the camera and the height shell, which a design edit does
/// not move.
pub struct DesignHandoff {
    pub request: DesignRequest,
    pub coverage: VoxelCoverage,
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
    /// Take the queued request, if any, together with the coverage the
    /// finishing revision reached. The coverage is cloned only when a request
    /// is actually taken; the worker polls this every millisecond.
    pub fn take_request(&mut self, coverage: &VoxelCoverage) -> Option<DesignHandoff> {
        let request = self.request.take()?;
        Some(DesignHandoff {
            request,
            coverage: coverage.clone(),
        })
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
    fn draw_frame_records_carry_levels_and_derive_the_band_blend_and_bias() {
        use procgen_realtime_pilot::{VOXEL_CHUNK_CELLS, VoxelPosition};
        let address = |x, y, z, lod| {
            procgen_realtime_pilot::VoxelChunkAddress::containing(
                VoxelPosition {
                    x_m: x,
                    y_m: y,
                    z_m: z,
                },
                lod,
            )
            .unwrap()
        };
        let span = VOXEL_CHUNK_CELLS;
        let addresses = [address(0, 0, 0, 0), address(span, -span, 2 * span, 0)];
        let layout = local_draw_layout(&addresses, 0.0);
        assert_eq!(layout.bounds, [[0, -span, 0], [2 * span, span, 3 * span]]);
        assert_eq!(layout.origins.len(), addresses.len() * LOCAL_ORIGIN_BYTES);
        let records: &[i32] = bytemuck::cast_slice(&layout.origins);
        assert_eq!(records, [0, 0, 0, 0, span, -span, 2 * span, 0]);
        // One-meter chunks span 32 m, past the 16 m blend floor; their 1 m cells
        // sit exactly on the bias floor.
        assert_eq!(
            (layout.blend_m, layout.bias_m),
            (32.0, LOCAL_SURFACE_BIAS_M)
        );

        // A mixed band takes both distances from its coarsest lease: a level
        // three chunk spans 256 m with 8 m cells, past both floors.
        let mixed = [
            address(0, 0, 0, 0),
            address(8 * span, 0, 0, 2),
            address(32 * span, 0, 0, 3),
        ];
        let layout = local_draw_layout(&mixed, 0.0);
        let records: &[i32] = bytemuck::cast_slice(&layout.origins);
        assert_eq!(
            records.chunks_exact(4).map(|r| r[3]).collect::<Vec<_>>(),
            [0, 2, 3],
            "the fourth word of each record is the chunk level"
        );
        assert_eq!((layout.blend_m, layout.bias_m), (256.0, 8.0));

        // The volume term's amplitude adds to the bias, because the voxel
        // surface can sit that far below the height surface; the dissolve
        // width does not change.
        let layout = local_draw_layout(&mixed, 6.0);
        assert_eq!((layout.blend_m, layout.bias_m), (256.0, 14.0));
        let layout = local_draw_layout(&addresses, 6.0);
        assert_eq!((layout.blend_m, layout.bias_m), (32.0, 7.0));

        // An empty region must still describe a finite box, a bindable buffer,
        // and the two floors.
        let layout = local_draw_layout(&[], 6.0);
        assert_eq!(layout.bounds, [[0; 3]; 2]);
        assert_eq!(layout.origins.len(), LOCAL_ORIGIN_BYTES);
        assert_eq!(
            (layout.blend_m, layout.bias_m),
            (LOCAL_BLEND_M, LOCAL_SURFACE_BIAS_M)
        );
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
    #[test]
    fn a_taken_request_carries_the_finishing_revisions_coverage_forward() {
        use procgen_realtime_pilot::{VoxelChunkAddress, VoxelPosition};
        let reached = VoxelCoverage::new(
            VoxelChunkAddress::containing(
                VoxelPosition {
                    x_m: 4_903_255,
                    y_m: 16,
                    z_m: 16,
                },
                3,
            )
            .unwrap()
            .children()
            .unwrap()
            .to_vec(),
        )
        .unwrap();
        let mut exchange = DesignExchange::default();
        assert!(
            exchange.take_request(&reached).is_none(),
            "no request, no hand-off"
        );
        exchange.invalidate(1);
        exchange.request = Some(DesignRequest {
            revision: 1,
            field: publication(1).field,
        });
        let handoff = exchange.take_request(&reached).unwrap();
        assert_eq!(handoff.request.revision, 1);
        assert_eq!(
            handoff.coverage.leaves(),
            reached.leaves(),
            "the next world starts its band where this one left it"
        );
        assert!(exchange.take_request(&reached).is_none());
    }
}
