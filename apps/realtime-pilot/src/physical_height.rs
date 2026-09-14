//! Immutable height tile cache with bounded batch overlap and atomic publication.
pub const HEIGHT_BATCHES_IN_FLIGHT: usize = 2;
use crate::physical_gpu_bridge::{
    GpuGeneration, HeightFrame, HeightSubmission, SURFACE_BLEND_SECONDS,
};
use procgen_realtime_pilot::HeightTile;
use procgen_realtime_pilot::{
    HEIGHT_GPU_BATCH_TILES, HEIGHT_TILE_BYTES, HeightFilter, HeightGpuMesher,
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, mpsc},
    time::Instant,
};

struct Pending {
    frame: HeightFrame,
    receipts: Vec<mpsc::Receiver<()>>,
    ready_at: Option<Instant>,
    remaining: VecDeque<HeightTile>,
    start: Instant,
}
impl Pending {
    fn poll(&mut self) {
        self.receipts.retain(|receipt| match receipt.try_recv() {
            Ok(()) => false,
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => panic!("height completion disconnected"),
        });
        if self.remaining.is_empty() && self.receipts.is_empty() {
            self.ready_at.get_or_insert_with(Instant::now);
        }
    }
    fn next_batch(&mut self) -> Option<Vec<HeightTile>> {
        if self.remaining.is_empty() || self.receipts.len() == HEIGHT_BATCHES_IN_FLIGHT {
            return None;
        }
        let count = self.remaining.len().min(HEIGHT_GPU_BATCH_TILES);
        Some(self.remaining.drain(..count).collect())
    }
}
pub struct HeightStream {
    mesher: HeightGpuMesher,
    current: Option<Arc<HeightFrame>>,
    pending: Option<Pending>,
}
impl HeightStream {
    pub fn new(device: &wgpu::Device, bridge: &GpuGeneration) -> Self {
        Self {
            mesher: HeightGpuMesher::new(device, &bridge.design.field),
            current: None,
            pending: None,
        }
    }
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        bridge: &GpuGeneration,
        target: &[HeightTile],
        filter: HeightFilter,
        admit: bool,
    ) {
        let blend_complete = self.current.as_ref().is_none_or(|current| {
            current.born + SURFACE_BLEND_SECONDS <= bridge.shared.start.elapsed().as_secs_f32()
        });
        if blend_complete {
            bridge.design.output.lock().unwrap().previous_height = None;
        }
        if let Some(pending) = &mut self.pending {
            pending.poll();
            if pending.ready_at.is_some() && blend_complete {
                let pending = self.pending.take().unwrap();
                let mut frame = pending.frame;
                frame.born = bridge.shared.start.elapsed().as_secs_f32();
                let frame = Arc::new(frame);
                let mut output = bridge.design.output.lock().unwrap();
                output.previous_height = self.current.replace(Arc::clone(&frame));
                output.height = Some(frame);
                let ready = pending.ready_at.unwrap();
                output.stats.height_build_ms =
                    ready.duration_since(pending.start).as_secs_f64() * 1000.0;
                output.stats.height_wait_ms = ready.elapsed().as_secs_f64() * 1000.0;
                output.stats.height_update_ms = pending.start.elapsed().as_secs_f64() * 1000.0;
            }
        }
        if self.pending.is_none() {
            if !admit {
                return;
            }
            if let Some(current) = &self.current
                && current.filter == filter
                && current.tiles.keys().copied().eq(target.iter().copied())
            {
                return;
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
                receipts: Vec::new(),
                ready_at: None,
                start: Instant::now(),
            });
        }
        let pending = self.pending.as_mut().unwrap();
        // Pipeline two bounded batches through the render owner. Generation may
        // overlap the previous dissolve, but publication must wait for it.
        while let Some(batch) = pending.next_batch() {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("bounded height batch"),
            });
            let buffers =
                self.mesher
                    .encode_batch(device, &mut encoder, &batch, pending.frame.filter);
            pending
                .frame
                .tiles
                .extend(batch.into_iter().zip(buffers.into_iter().map(Arc::new)));
            let (completed, receipt) = mpsc::channel();
            bridge
                .shared
                .submit
                .send(HeightSubmission {
                    commands: encoder.finish(),
                    completed,
                })
                .expect("render queue owner");
            pending.receipts.push(receipt);
        }
    }

    pub fn idle(&self) -> bool {
        self.pending.is_none()
    }

    pub fn ready(&self) -> bool {
        self.current.is_some()
    }

    pub fn allocated_bytes(&self, bridge: &GpuGeneration) -> u64 {
        // Count shared immutable tiles once across current, previous and pending.
        let output = bridge.design.output.lock().unwrap();
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

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_cubesphere::{CubeFace, TileAddress};
    #[test]
    fn batch_window_is_bounded_and_waits_for_out_of_order_completions() {
        let tiles: Vec<_> = (0..70)
            .map(|x| HeightTile::new(TileAddress::new(CubeFace::PositiveX, 7, x, 0).unwrap(), []))
            .collect();
        let mut pending = Pending {
            frame: HeightFrame {
                tiles: BTreeMap::new(),
                born: 0.0,
                filter: HeightFilter {
                    surface_m: [0.0; 3],
                    clearance_m: 0.0,
                },
            },
            remaining: tiles.clone().into(),
            receipts: Vec::new(),
            start: Instant::now(),
            ready_at: None,
        };
        let mut encoded = pending.next_batch().unwrap();
        let (a, ar) = mpsc::channel();
        pending.receipts.push(ar);
        encoded.extend(pending.next_batch().unwrap());
        let (b, br) = mpsc::channel();
        pending.receipts.push(br);
        assert!(
            pending.next_batch().is_none(),
            "full window must stop admission"
        );
        b.send(()).unwrap();
        pending.poll();
        assert!(pending.ready_at.is_none());
        encoded.extend(pending.next_batch().unwrap());
        let (c, cr) = mpsc::channel();
        pending.receipts.push(cr);
        c.send(()).unwrap();
        pending.poll();
        assert!(
            pending.ready_at.is_none(),
            "last batch does not imply all work is complete"
        );
        a.send(()).unwrap();
        pending.poll();
        assert!(pending.ready_at.is_some());
        assert_eq!(encoded, tiles, "every tile encoded once, in order");
        assert!(pending.next_batch().is_none());
    }
}
