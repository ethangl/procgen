//! Immutable height tile cache and one asynchronous replacement at a time.
use crate::physical_gpu_bridge::{
    GpuBridge, HeightFrame, HeightSubmission, SURFACE_BLEND_SECONDS, TerrainSubmission,
};
use procgen_cubesphere::TileAddress;
use procgen_realtime_pilot::{HEIGHT_TILE_BYTES, HeightFilter, HeightGpuMesher};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, mpsc},
    time::Instant,
};

struct Pending {
    frame: HeightFrame,
    receipt: Option<mpsc::Receiver<()>>,
    remaining: VecDeque<TileAddress>,
    start: Instant,
}
pub struct HeightStream {
    mesher: HeightGpuMesher,
    current: Option<Arc<HeightFrame>>,
    pending: Option<Pending>,
}
impl HeightStream {
    pub fn new(device: &wgpu::Device, bridge: &GpuBridge) -> Self {
        Self {
            mesher: HeightGpuMesher::new(device, &bridge.field),
            current: None,
            pending: None,
        }
    }
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        bridge: &GpuBridge,
        target: &[TileAddress],
        filter: HeightFilter,
    ) {
        if let Some(pending) = &mut self.pending {
            if let Some(receipt) = &pending.receipt {
                match receipt.try_recv() {
                    Ok(()) => pending.receipt = None,
                    Err(mpsc::TryRecvError::Empty) => return,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        panic!("height completion disconnected")
                    }
                }
            }
            if pending.remaining.is_empty() {
                let pending = self.pending.take().unwrap();
                let mut frame = pending.frame;
                frame.born = bridge.start.elapsed().as_secs_f32();
                let frame = Arc::new(frame);
                let mut output = bridge.output.lock().unwrap();
                output.previous_height = self.current.replace(Arc::clone(&frame));
                output.height = Some(frame);
                output.stats.height_update_ms = pending.start.elapsed().as_secs_f64() * 1000.0;
            }
        }
        if self.pending.is_none() {
            if let Some(current) = &self.current {
                if current.born + SURFACE_BLEND_SECONDS > bridge.start.elapsed().as_secs_f32() {
                    return;
                }
                bridge.output.lock().unwrap().previous_height = None;
                if current.filter == filter
                    && current.tiles.keys().copied().eq(target.iter().copied())
                {
                    return;
                }
            }
            let mut tiles = BTreeMap::new();
            let mut remaining = VecDeque::new();
            for &tile in target {
                if let Some(buffer) = self
                    .current
                    .as_ref()
                    .filter(|c| c.filter == filter)
                    .and_then(|c| c.tiles.get(&tile))
                {
                    tiles.insert(tile, Arc::clone(buffer));
                } else {
                    remaining.push_back(tile);
                }
            }
            self.pending = Some(Pending {
                frame: HeightFrame {
                    tiles,
                    filter,
                    born: 0.0,
                },
                remaining,
                receipt: None,
                start: Instant::now(),
            });
        }
        let pending = self.pending.as_mut().unwrap();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bounded height batch"),
        });
        // One batch in flight; 32 tiles bound GPU interference with rendering.
        for _ in 0..32 {
            let Some(tile) = pending.remaining.pop_front() else {
                break;
            };
            let output = self
                .mesher
                .encode(device, &mut encoder, tile, pending.frame.filter);
            pending.frame.tiles.insert(tile, Arc::new(output));
        }
        let (completed, receipt) = mpsc::channel();
        bridge
            .submit
            .send(TerrainSubmission::Height(HeightSubmission {
                commands: encoder.finish(),
                completed,
            }))
            .expect("render queue owner");
        pending.receipt = Some(receipt);
    }
    pub fn allocated_bytes(&self, bridge: &GpuBridge) -> u64 {
        // Count shared immutable tiles once across current, previous and pending.
        let output = bridge.output.lock().unwrap();
        let mut allocations = std::collections::BTreeSet::new();
        for frame in self
            .current
            .iter()
            .map(AsRef::as_ref)
            .chain(output.previous_height.iter().map(AsRef::as_ref))
            .chain(self.pending.iter().map(|p| &p.frame))
        {
            allocations.extend(frame.tiles.values().map(|b| Arc::as_ptr(b) as usize));
        }
        allocations.len() as u64 * HEIGHT_TILE_BYTES
    }
}
