//! Bounded persistent GPU slots with asynchronous completion and local publication.
use crate::voxel_gpu_buffers::OVERFLOW_FLAGS_BYTES;
use crate::{
    PlanetDesignField, VoxelCoverage, VoxelGpuInput, VoxelGpuMeshConfig, VoxelGpuMesher,
    VoxelGpuOutcome, VoxelGpuResidency, VoxelGpuSlot, VoxelGpuTicket, VoxelGpuWork, VoxelMeshError,
    VoxelMeshKey, VoxelPosition, VoxelPublication, VoxelStreamConfig, VoxelStreamError,
    VoxelTransitionPlan,
};
use std::{
    error::Error,
    fmt,
    sync::{Arc, mpsc},
    time::Instant,
};

#[derive(Clone, Copy, Debug)]
pub struct VoxelGpuWorldConfig {
    pub stream: VoxelStreamConfig,
    pub mesh: VoxelGpuMeshConfig,
    pub memory_budget_bytes: u64,
}
#[derive(Debug)]
pub enum VoxelGpuError {
    Stream(VoxelStreamError),
    Mesh(VoxelMeshError),
    MemoryBudget,
    DeviceLimits,
    Stopped,
    Poll(wgpu::PollError),
    Map(wgpu::BufferAsyncError),
}
impl From<VoxelStreamError> for VoxelGpuError {
    fn from(e: VoxelStreamError) -> Self {
        Self::Stream(e)
    }
}
impl From<VoxelMeshError> for VoxelGpuError {
    fn from(e: VoxelMeshError) -> Self {
        Self::Mesh(e)
    }
}
impl fmt::Display for VoxelGpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stream(error) => error.fmt(f),
            Self::Mesh(error) => error.fmt(f),
            Self::MemoryBudget => f.write_str("GPU generation exceeds the memory reservation; prior coverage is retained"),
            Self::DeviceLimits => f.write_str("GPU voxel generation requires eight storage bindings and buffers large enough for its outputs"),
            Self::Stopped => f.write_str("GPU stream stopped after a device or mapping failure; create a new stream to resume"),
            Self::Poll(error) => error.fmt(f),
            Self::Map(error) => error.fmt(f),
        }
    }
}
impl Error for VoxelGpuError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Stream(e) => Some(e),
            Self::Mesh(e) => Some(e),
            Self::Poll(e) => Some(e),
            Self::Map(e) => Some(e),
            _ => None,
        }
    }
}
pub enum VoxelGpuEvent {
    Submitted {
        ticket: VoxelGpuTicket,
        preparation_ms: f64,
        encoding_ms: f64,
    },
    Completed {
        ticket: VoxelGpuTicket,
        elapsed_ms: f64,
        outcome: VoxelGpuOutcome,
    },
    Published(VoxelPublication),
}
enum Completion {
    Job(VoxelGpuTicket, Result<(), wgpu::BufferAsyncError>),
    Fence(u64),
}
struct Pending {
    ticket: VoxelGpuTicket,
    work: VoxelGpuWork,
    flags: wgpu::Buffer,
    submission: u64,
    start: Instant,
}

pub struct VoxelGpuWorld {
    device: wgpu::Device,
    queue: wgpu::Queue,
    mesher: VoxelGpuMesher,
    field: Arc<PlanetDesignField>,
    config: VoxelGpuWorldConfig,
    residency: VoxelGpuResidency,
    slots: Vec<Option<VoxelGpuSlot>>,
    jobs: Vec<Pending>,
    sender: mpsc::Sender<Completion>,
    receiver: mpsc::Receiver<Completion>,
    last_submission: u64,
    completed_submission: u64,
    stopped: bool,
}
impl VoxelGpuWorld {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        field: Arc<PlanetDesignField>,
        config: VoxelGpuWorldConfig,
    ) -> Result<Self, VoxelGpuError> {
        config.stream.validate()?;
        config.mesh.validate()?;
        let largest = (config.mesh.regular.vertex_capacity as u64
            * size_of::<crate::VoxelMeshVertex>() as u64)
            .max(config.mesh.regular.triangle_capacity as u64 * 12)
            .max(
                config.mesh.transition.triangle_capacity as u64
                    * 3
                    * size_of::<crate::VoxelMeshVertex>() as u64,
            );
        // A boundary node can request eight source points, each a 16-byte record.
        let largest =
            largest.max(crate::voxel_transition_plan::MAX_TRANSITION_NODES as u64 * 8 * 16);
        let limits = device.limits();
        if limits.max_storage_buffers_per_shader_stage < 8
            || largest > limits.max_storage_buffer_binding_size as u64
            || largest > limits.max_buffer_size
        {
            return Err(VoxelGpuError::DeviceLimits);
        }
        if config.memory_budget_bytes < config.mesh.slot_bytes() {
            return Err(VoxelGpuError::MemoryBudget);
        }
        let (sender, receiver) = mpsc::channel();
        Ok(Self {
            device: device.clone(),
            queue: queue.clone(),
            mesher: VoxelGpuMesher::new(device, queue),
            field,
            config,
            residency: VoxelGpuResidency::new(config.stream)?,
            slots: (0..config.stream.max_slots).map(|_| None).collect(),
            jobs: Vec::new(),
            sender,
            receiver,
            last_submission: 0,
            completed_submission: 0,
            stopped: false,
        })
    }
    pub fn set_coverage(
        &mut self,
        coverage: &VoxelCoverage,
        camera: VoxelPosition,
    ) -> Result<(), VoxelGpuError> {
        self.residency.set_coverage(coverage, camera)?;
        Ok(())
    }
    pub fn settled(&self) -> bool {
        self.residency.settled()
    }
    pub fn in_flight(&self) -> usize {
        self.residency.in_flight()
    }
    pub fn retiring(&self) -> usize {
        self.residency.retiring()
    }
    pub fn failed(&self) -> impl Iterator<Item = &VoxelMeshKey> {
        self.residency.failed()
    }
    pub fn memory_bytes(&self) -> u64 {
        self.slots.iter().filter(|s| s.is_some()).count() as u64 * self.config.mesh.slot_bytes()
            + self
                .jobs
                .iter()
                .map(|j| j.work.allocated_bytes() + j.flags.size())
                .sum::<u64>()
    }
    pub fn resident_slots(
        &self,
    ) -> impl Iterator<Item = (VoxelGpuTicket, &VoxelMeshKey, &VoxelGpuSlot)> {
        self.residency.residents().map(|(ticket, key)| {
            (
                ticket,
                key,
                self.slots[ticket.slot()]
                    .as_ref()
                    .expect("resident allocation"),
            )
        })
    }
    /// Call immediately after submitting commands that use resident draw buffers
    /// on this queue. Retirement then waits for that submission's completion too.
    pub fn track_draw_submission(&mut self) {
        self.last_submission += 1;
        let fence = self.last_submission;
        let sender = self.sender.clone();
        self.queue.on_submitted_work_done(move || {
            let _ = sender.send(Completion::Fence(fence));
        });
    }
    /// Non-blocking device poll. Reads only two overflow flags (eight bytes) per
    /// completed job. Density, vertices, indices, and draw counts remain on the GPU.
    pub fn update(&mut self) -> Result<Vec<VoxelGpuEvent>, VoxelGpuError> {
        if self.stopped {
            return Err(VoxelGpuError::Stopped);
        }
        if let Err(error) = self.device.poll(wgpu::PollType::Poll) {
            self.stopped = true;
            return Err(VoxelGpuError::Poll(error));
        }
        let mut events = Vec::new();
        while let Ok(completion) = self.receiver.try_recv() {
            match completion {
                Completion::Fence(fence) => {
                    self.completed_submission = self.completed_submission.max(fence)
                }
                Completion::Job(ticket, result) => {
                    if let Err(error) = result {
                        self.stopped = true;
                        return Err(VoxelGpuError::Map(error));
                    }
                    let index = self
                        .jobs
                        .iter()
                        .position(|j| j.ticket == ticket)
                        .expect("pending completion");
                    let job = self.jobs.remove(index);
                    let mapped = job.flags.slice(..).get_mapped_range();
                    let flags: &[u32] = bytemuck::cast_slice(&mapped);
                    let outcome = if flags.iter().any(|&flag| flag != 0) {
                        VoxelGpuOutcome::Failed
                    } else {
                        VoxelGpuOutcome::Ready
                    };
                    drop(mapped);
                    job.flags.unmap();
                    self.completed_submission = self.completed_submission.max(job.submission);
                    self.residency.complete(ticket, outcome);
                    events.push(VoxelGpuEvent::Completed {
                        ticket,
                        elapsed_ms: job.start.elapsed().as_secs_f64() * 1000.0,
                        outcome,
                    });
                }
            }
        }
        let publication = self.residency.publish(self.last_submission);
        if !publication.installed.is_empty() || !publication.retired.is_empty() {
            events.push(VoxelGpuEvent::Published(publication));
        }
        self.residency.release_completed(self.completed_submission);
        while let Some(request) = self.residency.admit() {
            let start = Instant::now();
            let plan = VoxelTransitionPlan::new(request.key);
            let allocation = if self.slots[request.ticket.slot()].is_some() {
                0
            } else {
                self.config.mesh.slot_bytes()
            };
            if self.memory_bytes() + allocation + VoxelGpuMesher::work_bytes(&plan)
                > self.config.memory_budget_bytes
            {
                self.residency
                    .complete(request.ticket, VoxelGpuOutcome::Failed);
                return Err(VoxelGpuError::MemoryBudget);
            }
            if self.slots[request.ticket.slot()].is_none() {
                self.slots[request.ticket.slot()] =
                    Some(VoxelGpuSlot::new(&self.device, self.config.mesh)?);
            }
            let slot = self.slots[request.ticket.slot()].as_ref().unwrap();
            let work = self.mesher.prepare(
                &plan,
                VoxelGpuInput::Field(&self.field),
                slot,
                self.config.mesh,
            )?;
            let flags = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("voxel overflow completion"),
                size: OVERFLOW_FLAGS_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let preparation_ms = start.elapsed().as_secs_f64() * 1000.0;
            let encoding = Instant::now();
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.mesher.encode(&mut encoder, &work, slot);
            let flag_offset = std::mem::offset_of!(crate::VoxelMeshStatus, overflow) as u64;
            let flag_bytes = size_of::<u32>() as u64;
            encoder.copy_buffer_to_buffer(&slot.regular.status, flag_offset, &flags, 0, flag_bytes);
            encoder.copy_buffer_to_buffer(
                &slot.transition.status,
                flag_offset,
                &flags,
                flag_bytes,
                flag_bytes,
            );
            self.queue.submit([encoder.finish()]);
            self.last_submission += 1;
            let sender = self.sender.clone();
            let ticket = request.ticket;
            flags
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send(Completion::Job(ticket, result));
                });
            events.push(VoxelGpuEvent::Submitted {
                ticket,
                preparation_ms,
                encoding_ms: encoding.elapsed().as_secs_f64() * 1000.0,
            });
            self.jobs.push(Pending {
                ticket,
                work,
                flags,
                submission: self.last_submission,
                start,
            });
            debug_assert!(self.memory_bytes() <= self.config.memory_budget_bytes);
        }
        Ok(events)
    }
}
