//! Explicit CPU visual audit worker. The GPU viewer does not start this worker.
use crate::physical_render::{Coloring, PackedSurface};
use procgen_realtime_pilot::{
    DesignPreviewConfig, PhysicalTerrain, PlanetDesignField, PreviewArea, PreviewBands,
    VoxelPosition, generate_design_preview,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::JoinHandle,
    time::Instant,
};

#[derive(Clone, Copy)]
pub struct TerrainRequest {
    pub camera: VoxelPosition,
    pub coloring: Coloring,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TerrainKind {
    Overview,
    Voxels(Coloring),
}
pub struct TerrainResult {
    pub kind: TerrainKind,
    pub packed: PackedSurface,
    pub generation_seconds: f32,
    pub source_bytes: usize,
    pub mesh_bytes: usize,
}
pub struct Jobs {
    pub terrain: SyncSender<TerrainRequest>,
    pub surfaces: Receiver<Result<TerrainResult, String>>,
    cancel: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
}
impl Jobs {
    pub fn start(field: Arc<PlanetDesignField>) -> Self {
        let (terrain, requests) = mpsc::sync_channel::<TerrainRequest>(1);
        let (results, surfaces) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let source = Arc::clone(&field);
        let stop = Arc::clone(&cancel);
        let render = std::thread::spawn(move || {
            let started = Instant::now();
            let overview = generate_design_preview(
                source.config(),
                DesignPreviewConfig {
                    area: PreviewArea::Planet,
                    bands: PreviewBands::Combined,
                    quads: 64,
                },
            )
            .map(|p| TerrainResult {
                kind: TerrainKind::Overview,
                packed: PackedSurface::overview(&p, source.config()),
                generation_seconds: started.elapsed().as_secs_f32(),
                source_bytes: 0,
                mesh_bytes: 0,
            })
            .map_err(|e| e.to_string());
            if results.send(overview).is_err() {
                return;
            }
            let mut terrain =
                PhysicalTerrain::new(Arc::clone(&source)).expect("fixed viewer residency config");
            while !stop.load(Ordering::Relaxed) {
                let request = match requests.recv_timeout(std::time::Duration::from_millis(50)) {
                    Ok(r) => r,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                let result = terrain
                    .build(request.camera, &stop)
                    .map(|frame| {
                        let packed =
                            PackedSurface::voxel(&frame.surface, request.coloring, source.config());
                        TerrainResult {
                            kind: TerrainKind::Voxels(request.coloring),
                            packed,
                            generation_seconds: frame.generation_seconds,
                            source_bytes: frame.source_bytes,
                            mesh_bytes: frame.surface.payload_bytes(),
                        }
                    })
                    .map_err(|e| e.to_string());
                if stop.load(Ordering::Relaxed) || results.send(result).is_err() {
                    break;
                }
            }
        });
        Self {
            terrain,
            surfaces,
            cancel,
            workers: vec![render],
        }
    }
}
impl Drop for Jobs {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        // Drain completed products before joining a sender that may be blocked.
        while self.workers.iter().any(|w| !w.is_finished()) {
            while self.surfaces.try_recv().is_ok() {}
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}
