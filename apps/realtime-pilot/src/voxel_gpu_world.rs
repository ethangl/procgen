//! Bounded persistent GPU slots with asynchronous completion and local publication.
use crate::voxel_gpu_buffers::OVERFLOW_FLAGS_BYTES;
use crate::voxel_gpu_timing::{GpuTiming, TIMING_BYTES};
use crate::{
    PlanetDesignField, VoxelCoverage, VoxelGpuInput, VoxelGpuMeshConfig, VoxelGpuMesher,
    VoxelGpuOutcome, VoxelGpuRequest, VoxelGpuResidency, VoxelGpuSlot, VoxelGpuTicket,
    VoxelGpuWork, VoxelMeshError, VoxelMeshKey, VoxelPosition, VoxelPublication, VoxelStreamConfig,
    VoxelStreamError, VoxelTransitionPlan,
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
        submission_latency_ms: f64,
        gpu_times: Option<crate::VoxelGpuTimes>,
        outcome: VoxelGpuOutcome,
    },
    Published(VoxelPublication),
}
enum Completion {
    Job {
        ticket: VoxelGpuTicket,
        result: Result<(), wgpu::BufferAsyncError>,
        submission_latency_ms: f64,
    },
    Fence(u64),
}
/// Encoded work transferred to the owner of the render queue. Mapping is armed
/// on that thread only after its submission has been registered.
pub struct VoxelGpuSubmission {
    commands: wgpu::CommandBuffer,
    flags: wgpu::Buffer,
    ticket: VoxelGpuTicket,
    sender: mpsc::Sender<Completion>,
}
impl VoxelGpuSubmission {
    pub fn submit(self, queue: &wgpu::Queue) {
        let submitted = Instant::now();
        queue.submit([self.commands]);
        self.flags
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = self.sender.send(Completion::Job {
                    ticket: self.ticket,
                    result,
                    submission_latency_ms: submitted.elapsed().as_secs_f64() * 1000.0,
                });
            });
    }
}
struct Pending {
    ticket: VoxelGpuTicket,
    work: VoxelGpuWork,
    flags: wgpu::Buffer,
    submission: u64,
    start: Instant,
}

#[derive(Clone)]
pub struct VoxelGpuLease {
    pub ticket: VoxelGpuTicket,
    pub key: VoxelMeshKey,
    pub slot: Arc<VoxelGpuSlot>,
}

pub struct VoxelGpuWorld {
    device: wgpu::Device,
    queue: wgpu::Queue,
    mesher: VoxelGpuMesher,
    field: Arc<PlanetDesignField>,
    config: VoxelGpuWorldConfig,
    residency: VoxelGpuResidency,
    slots: Vec<Option<Arc<VoxelGpuSlot>>>,
    jobs: Vec<Pending>,
    sender: mpsc::Sender<Completion>,
    receiver: mpsc::Receiver<Completion>,
    last_submission: u64,
    completed_submission: u64,
    stopped: bool,
    /// Events gathered by an update that then failed. A caller that observes the
    /// error still observes every publication and completion that preceded it.
    deferred: Vec<VoxelGpuEvent>,
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
            deferred: Vec::new(),
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
    pub fn resident_bytes(&self) -> u64 {
        self.residency.residents().count() as u64 * self.config.mesh.slot_bytes()
    }
    pub fn retiring_bytes(&self) -> u64 {
        self.residency.retiring() as u64 * self.config.mesh.slot_bytes()
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
                    .expect("resident allocation")
                    .as_ref(),
            )
        })
    }
    /// A draw lease pins the allocation until the consumer and its submitted
    /// commands have released it. Keep the lease through GPU completion.
    pub fn resident_leases(&self) -> Vec<VoxelGpuLease> {
        self.residency
            .residents()
            .map(|(ticket, key)| VoxelGpuLease {
                ticket,
                key: key.clone(),
                slot: Arc::clone(
                    self.slots[ticket.slot()]
                        .as_ref()
                        .expect("resident allocation"),
                ),
            })
            .collect()
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
    /// Non-blocking device poll. Reads two overflow flags and optional stage
    /// timestamps. Density, vertices, indices, and draw counts remain on the GPU.
    pub fn update(&mut self) -> Result<Vec<VoxelGpuEvent>, VoxelGpuError> {
        let queue = self.queue.clone();
        self.update_with_submit(|submission| submission.submit(&queue))
    }
    /// Preparation and encoding run here; the caller owns command submission.
    /// Events survive a failed update: they are returned by the next one.
    pub fn update_with_submit(
        &mut self,
        submit: impl FnMut(VoxelGpuSubmission),
    ) -> Result<Vec<VoxelGpuEvent>, VoxelGpuError> {
        let mut events = std::mem::take(&mut self.deferred);
        match self.step(&mut events, submit) {
            Ok(()) => Ok(events),
            Err(error) => {
                self.deferred = events;
                Err(error)
            }
        }
    }
    fn step<F: FnMut(VoxelGpuSubmission)>(
        &mut self,
        events: &mut Vec<VoxelGpuEvent>,
        mut submit: F,
    ) -> Result<(), VoxelGpuError> {
        if self.stopped {
            return Err(VoxelGpuError::Stopped);
        }
        if let Err(error) = self.device.poll(wgpu::PollType::Poll) {
            self.stopped = true;
            return Err(VoxelGpuError::Poll(error));
        }
        while let Ok(completion) = self.receiver.try_recv() {
            match completion {
                Completion::Fence(fence) => {
                    self.completed_submission = self.completed_submission.max(fence)
                }
                Completion::Job {
                    ticket,
                    result,
                    submission_latency_ms,
                } => {
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
                    let flags: &[u32] =
                        bytemuck::cast_slice(&mapped[..OVERFLOW_FLAGS_BYTES as usize]);
                    let gpu_times = job.work.timing.as_ref().map(|_| {
                        GpuTiming::times(
                            &mapped[OVERFLOW_FLAGS_BYTES as usize..],
                            self.queue.get_timestamp_period(),
                        )
                    });
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
                        gpu_times,
                        submission_latency_ms,
                    });
                }
            }
        }
        let publication = self.residency.publish(self.last_submission);
        if !publication.installed.is_empty() || !publication.retired.is_empty() {
            events.push(VoxelGpuEvent::Published(publication));
        }
        self.residency
            .release_unleased(self.completed_submission, |index| {
                self.slots[index]
                    .as_ref()
                    .is_none_or(|slot| Arc::strong_count(slot) == 1)
            });
        while let Some(request) = self.residency.admit() {
            let ticket = request.ticket;
            // An admitted slot is Working until it is completed. Completing every
            // failure here keeps a new `?` in `encode_request` from leaking the slot.
            if let Err(error) = self.encode_request(request, events, &mut submit) {
                self.residency.complete(ticket, VoxelGpuOutcome::Failed);
                return Err(error);
            }
            debug_assert!(self.memory_bytes() <= self.config.memory_budget_bytes);
        }
        Ok(())
    }
    /// The caller completes `request.ticket` as failed on every error path.
    fn encode_request<F: FnMut(VoxelGpuSubmission)>(
        &mut self,
        request: VoxelGpuRequest,
        events: &mut Vec<VoxelGpuEvent>,
        submit: &mut F,
    ) -> Result<(), VoxelGpuError> {
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
            return Err(VoxelGpuError::MemoryBudget);
        }
        if self.slots[request.ticket.slot()].is_none() {
            self.slots[request.ticket.slot()] =
                Some(Arc::new(VoxelGpuSlot::new(&self.device, self.config.mesh)?));
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
            size: OVERFLOW_FLAGS_BYTES + work.timing.as_ref().map_or(0, |_| TIMING_BYTES),
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
        if let Some(timing) = &work.timing {
            encoder.copy_buffer_to_buffer(
                &timing.resolved,
                0,
                &flags,
                OVERFLOW_FLAGS_BYTES,
                TIMING_BYTES,
            );
        }
        self.last_submission += 1;
        let ticket = request.ticket;
        submit(VoxelGpuSubmission {
            commands: encoder.finish(),
            flags: flags.clone(),
            ticket,
            sender: self.sender.clone(),
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
        Ok(())
    }
}
