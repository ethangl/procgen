//! Renderer-independent admission, cancellation, upload readiness, and residency.
use crate::{DetailLevel, DetailSource, RegionMesh, build_region};
use procgen_core::Vec3;
use procgen_cubesphere::CubeFace;
use std::{
    error::Error,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
};

pub const SOURCE_WORK_RESERVATION: usize = 1024 * 1024 * 1024;
pub const MANAGED_MEMORY_LIMIT: usize = 128 * 1024 * 1024;
pub const MAX_ACTIVE_JOBS: usize = 2;
pub const UPLOAD_BYTES_PER_FRAME: usize = 512 * 1024;
pub const INSTALL_MILLIS_PER_FRAME: f64 = 2.0;
pub const REPLACEMENT_SECONDS: f32 = 0.15;
pub const TRIANGLES_PER_PIECE: usize = 1024;
// Position, normal, RGBA color and one u32 index per expanded corner. Indexed
// chunks are no larger. Reserve both main-world and render-world copies.
pub const BYTES_PER_TRIANGLE: usize = 3 * (12 + 12 + 16 + 4);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ticket {
    pub serial: u64,
    pub face: CubeFace,
    pub detail: DetailLevel,
}
#[derive(Clone, Copy, Debug)]
pub struct StreamView {
    pub position: Vec3,
    pub forward: Vec3,
}
/// Walking keeps nearby visual support fine regardless of viewing direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailFocus {
    View,
    NearbyCollision,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct StreamStats {
    pub managed_bytes: usize,
    pub peak_managed_bytes: usize,
    pub active_jobs: usize,
    pub queued_jobs: usize,
    pub peak_active_jobs: usize,
    pub peak_queued_jobs: usize,
    pub cancellations: usize,
    pub rejected_results: usize,
    pub installations: usize,
    pub waiting_frames: usize,
}
#[derive(Debug)]
pub enum StreamError {
    Budget,
    Worker,
}
impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Budget => "stream product exceeds the managed memory budget",
            Self::Worker => "region worker stopped without a result",
        })
    }
}
impl Error for StreamError {}

pub enum StreamEvent {
    Commit(Ticket),
    Reveal,
    Retire(Ticket),
}
pub struct UploadPiece {
    pub ticket: Ticket,
    pub index: usize,
    pub triangles: std::ops::Range<usize>,
    pub mesh: Arc<RegionMesh>,
}
impl UploadPiece {
    pub fn reserved_bytes(&self) -> usize {
        self.triangles.len() * BYTES_PER_TRIANGLE
    }
}
struct Resident {
    ticket: Ticket,
    bytes: usize,
}
struct Upload {
    ticket: Ticket,
    mesh: Arc<RegionMesh>,
    submitted: usize,
    ready: Vec<bool>,
}
impl Upload {
    fn render_bytes(&self) -> usize {
        self.mesh.surface().triangles().len() * BYTES_PER_TRIANGLE * 2
    }
    fn bytes(&self) -> usize {
        self.mesh.allocated_bytes() + self.render_bytes()
    }
}
struct Slot {
    request: Ticket,
    resident: Option<Resident>,
    upload: Option<Upload>,
}
struct Job {
    ticket: Ticket,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Option<RegionMesh>>,
    reservation: usize,
}

pub struct StreamingWorld {
    source: Arc<DetailSource>,
    reserved_bytes: usize,
    slots: [Slot; 6],
    jobs: Vec<Job>,
    retired: Vec<Resident>,
    serial: u64,
    visible: bool,
    events: Vec<StreamEvent>,
    stats: StreamStats,
}
impl StreamingWorld {
    /// Reserve non-streaming products before admitting region work. The caller
    /// owns their lifetime; this reservation lasts for the entire world.
    pub fn new(source: Arc<DetailSource>, reserved_bytes: usize) -> Result<Self, StreamError> {
        if reserved_bytes > MANAGED_MEMORY_LIMIT
            || source.allocated_bytes() > MANAGED_MEMORY_LIMIT - reserved_bytes
        {
            return Err(StreamError::Budget);
        }
        Ok(Self {
            source,
            reserved_bytes,
            slots: std::array::from_fn(|i| Slot {
                request: Ticket {
                    serial: i as u64,
                    face: CubeFace::ALL[i],
                    detail: DetailLevel::Coarse,
                },
                resident: None,
                upload: None,
            }),
            jobs: Vec::new(),
            retired: Vec::new(),
            serial: 6,
            visible: false,
            events: Vec::new(),
            stats: StreamStats::default(),
        })
    }
    pub fn visible(&self) -> bool {
        self.visible
    }
    pub fn stats(&self) -> StreamStats {
        self.stats
    }
    pub fn requested_tickets(&self) -> [Ticket; 6] {
        std::array::from_fn(|i| self.slots[i].request)
    }
    pub fn resident_tickets(&self) -> [Option<Ticket>; 6] {
        std::array::from_fn(|i| self.slots[i].resident.as_ref().map(|r| r.ticket))
    }
    pub fn resident_levels(&self) -> [Option<DetailLevel>; 6] {
        std::array::from_fn(|i| self.slots[i].resident.as_ref().map(|r| r.ticket.detail))
    }
    pub fn events(&mut self) -> Vec<StreamEvent> {
        std::mem::take(&mut self.events)
    }
    fn bytes(&self) -> usize {
        self.reserved_bytes
            + self.source.allocated_bytes()
            + self.jobs.iter().map(|j| j.reservation).sum::<usize>()
            + self
                .slots
                .iter()
                .map(|s| {
                    s.resident.as_ref().map_or(0, |r| r.bytes)
                        + s.upload.as_ref().map_or(0, Upload::bytes)
                })
                .sum::<usize>()
            + self.retired.iter().map(|r| r.bytes).sum::<usize>()
    }
    fn needed(&self, face: usize) -> bool {
        let slot = &self.slots[face];
        slot.resident
            .as_ref()
            .is_none_or(|r| r.ticket.detail != slot.request.detail)
            && !self.retired.iter().any(|r| r.ticket.face.index() == face)
            && slot.upload.is_none()
            && !self.jobs.iter().any(|j| j.ticket.face.index() == face)
    }
    pub fn update(&mut self, view: StreamView, focus: DetailFocus) -> Result<(), StreamError> {
        if self.visible {
            let altitude = view.position.length() - self.source.radius;
            let direction = view.position.normalized();
            for face in CubeFace::ALL {
                let proximity = direction.dot(face.frame().normal);
                let toward = (face.frame().normal * self.source.radius - view.position)
                    .normalized()
                    .dot(view.forward);
                let current = self.slots[face.index()].request.detail;
                // Hysteresis prevents camera noise at a distance boundary from
                // repeatedly cancelling the same region. Coarse coverage remains behind.
                let fine_limit = if current == DetailLevel::Fine {
                    2.5
                } else {
                    2.0
                };
                let medium_limit = if current != DetailLevel::Coarse {
                    8.5
                } else {
                    8.0
                };
                let detail = if altitude < fine_limit
                    && proximity > 0.35
                    && (toward > 0.0 || focus == DetailFocus::NearbyCollision)
                {
                    DetailLevel::Fine
                } else if altitude < medium_limit && proximity > -0.25 {
                    DetailLevel::Medium
                } else {
                    DetailLevel::Coarse
                };
                self.request(face, detail);
            }
        }
        self.poll()?;
        while self.jobs.len() < MAX_ACTIVE_JOBS {
            let next = CubeFace::ALL
                .into_iter()
                .filter(|f| self.needed(f.index()))
                .max_by(|a, b| {
                    let score = |f: CubeFace| {
                        (f.frame().normal * self.source.radius - view.position)
                            .normalized()
                            .dot(view.forward)
                    };
                    score(*a).total_cmp(&score(*b)).then_with(|| b.cmp(a))
                });
            let Some(face) = next else {
                break;
            };
            let reservation = self.source.region_work_bytes(face);
            if reservation
                > MANAGED_MEMORY_LIMIT - self.source.allocated_bytes() - self.reserved_bytes
            {
                return Err(StreamError::Budget);
            }
            if self.bytes() + reservation > MANAGED_MEMORY_LIMIT {
                break;
            }
            let ticket = self.slots[face.index()].request;
            let source = Arc::clone(&self.source);
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_cancel = Arc::clone(&cancel);
            let (sender, receiver) = mpsc::sync_channel(1);
            std::thread::spawn(move || {
                let result = build_region(&source, ticket.face, ticket.detail, &worker_cancel);
                let _ = sender.send(result);
            });
            self.jobs.push(Job {
                ticket,
                cancel,
                receiver,
                reservation,
            });
        }
        self.measure();
        Ok(())
    }
    fn request(&mut self, face: CubeFace, detail: DetailLevel) {
        let slot = &mut self.slots[face.index()];
        if slot.request.detail == detail {
            return;
        }
        slot.request = Ticket {
            serial: self.serial,
            face,
            detail,
        };
        self.serial += 1;
        for job in &self.jobs {
            if job.ticket.face == face && !job.cancel.swap(true, Ordering::Relaxed) {
                self.stats.cancellations += 1;
            }
        }
        if let Some(upload) = slot.upload.take() {
            self.retired.push(Resident {
                ticket: upload.ticket,
                bytes: upload.render_bytes(),
            });
            self.events.push(StreamEvent::Retire(upload.ticket));
            self.stats.cancellations += 1;
        }
    }
    fn poll(&mut self) -> Result<(), StreamError> {
        let mut i = 0;
        while i < self.jobs.len() {
            let result = match self.jobs[i].receiver.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => {
                    i += 1;
                    continue;
                }
                Err(TryRecvError::Disconnected) => return Err(StreamError::Worker),
            };
            let job = self.jobs.remove(i);
            let Some(mesh) = result else {
                self.stats.rejected_results += 1;
                continue;
            };
            if self.slots[job.ticket.face.index()].request != job.ticket {
                self.stats.rejected_results += 1;
                continue;
            }
            let pieces = mesh
                .surface()
                .triangles()
                .len()
                .div_ceil(TRIANGLES_PER_PIECE);
            let upload = Upload {
                ticket: job.ticket,
                mesh: Arc::new(mesh),
                submitted: 0,
                ready: vec![false; pieces],
            };
            if upload.bytes() > job.reservation
                || self.bytes() + upload.bytes() > MANAGED_MEMORY_LIMIT
            {
                return Err(StreamError::Budget);
            }
            self.slots[job.ticket.face.index()].upload = Some(upload);
            if pieces == 0 {
                self.commit(job.ticket);
            }
        }
        Ok(())
    }
    /// Submit at most one fixed-size piece. The caller stops on its byte and
    /// elapsed-time budget, then acknowledges only after GPU preparation.
    pub fn next_upload(&mut self, remaining_bytes: usize) -> Option<UploadPiece> {
        for slot in &mut self.slots {
            let Some(upload) = &mut slot.upload else {
                continue;
            };
            if upload.submitted == upload.ready.len() {
                continue;
            }
            let index = upload.submitted;
            let start = index * TRIANGLES_PER_PIECE;
            let end = (start + TRIANGLES_PER_PIECE).min(upload.mesh.surface().triangles().len());
            if (end - start) * BYTES_PER_TRIANGLE > remaining_bytes {
                continue;
            }
            upload.submitted += 1;
            return Some(UploadPiece {
                ticket: upload.ticket,
                index,
                triangles: start..end,
                mesh: Arc::clone(&upload.mesh),
            });
        }
        None
    }
    pub fn acknowledge_upload(&mut self, ticket: Ticket, index: usize) -> bool {
        let slot = &mut self.slots[ticket.face.index()];
        if slot.request != ticket {
            return false;
        }
        let Some(upload) = &mut slot.upload else {
            return false;
        };
        if upload.ticket != ticket || index >= upload.submitted || upload.ready[index] {
            return false;
        }
        upload.ready[index] = true;
        if upload.submitted == upload.ready.len() && upload.ready.iter().all(|ready| *ready) {
            self.commit(ticket);
        }
        true
    }
    fn commit(&mut self, ticket: Ticket) {
        let slot = &mut self.slots[ticket.face.index()];
        let upload = slot.upload.take().expect("ready upload");
        let resident = Resident {
            ticket,
            bytes: upload.render_bytes(),
        };
        if let Some(old) = slot.resident.replace(resident) {
            self.events.push(StreamEvent::Retire(old.ticket));
            self.retired.push(old);
        }
        self.events.push(StreamEvent::Commit(ticket));
        self.stats.installations += 1;
        if !self.visible && self.slots.iter().all(|s| s.resident.is_some()) {
            self.visible = true;
            self.events.push(StreamEvent::Reveal);
        }
    }
    /// Renderer calls this after retired assets leave RenderAssets and the GPU
    /// queue confirms completion of work that could still reference them.
    pub fn acknowledge_retirement(&mut self, ticket: Ticket) {
        self.retired.retain(|r| r.ticket != ticket);
    }
    fn measure(&mut self) {
        self.stats.managed_bytes = self.bytes();
        self.stats.peak_managed_bytes = self.stats.peak_managed_bytes.max(self.stats.managed_bytes);
        self.stats.active_jobs = self.jobs.len();
        self.stats.peak_active_jobs = self.stats.peak_active_jobs.max(self.jobs.len());
        self.stats.queued_jobs = (0..6).filter(|&i| self.needed(i)).count();
        self.stats.peak_queued_jobs = self.stats.peak_queued_jobs.max(self.stats.queued_jobs);
        if self.slots.iter().any(|s| {
            s.resident
                .as_ref()
                .is_none_or(|r| r.ticket.detail != s.request.detail)
        }) {
            self.stats.waiting_frames += 1;
        }
        debug_assert!(self.stats.managed_bytes <= MANAGED_MEMORY_LIMIT);
    }
}
impl Drop for StreamingWorld {
    fn drop(&mut self) {
        for job in &self.jobs {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PILOT_PLANET, prepare_detail};
    #[test]
    fn replacement_waits_for_every_piece_and_rejects_cancelled_aba_results() {
        let source = Arc::new(
            prepare_detail(&PILOT_PLANET.validate(42).unwrap(), &AtomicBool::new(false))
                .unwrap()
                .unwrap(),
        );
        let mut world = StreamingWorld::new(source, 0).unwrap();
        let view = StreamView {
            position: Vec3::X * 13.0,
            forward: -Vec3::X,
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !world.visible() {
            assert!(std::time::Instant::now() < deadline);
            world.update(view, DetailFocus::View).unwrap();
            if let Some(piece) = world.next_upload(UPLOAD_BYTES_PER_FRAME) {
                assert_eq!(world.resident_levels()[piece.ticket.face.index()], None);
                assert!(world.acknowledge_upload(piece.ticket, piece.index));
            }
            for event in world.events() {
                if let StreamEvent::Retire(ticket) = event {
                    world.acknowledge_retirement(ticket);
                }
            }
            std::thread::yield_now();
        }
        let face = CubeFace::PositiveX;
        world.request(face, DetailLevel::Fine);
        let old = world.slots[face.index()].request;
        world.request(face, DetailLevel::Medium);
        world.request(face, DetailLevel::Fine);
        assert_ne!(world.slots[face.index()].request, old);
        let (sender, receiver) = mpsc::sync_channel(1);
        sender
            .send(build_region(
                &world.source,
                face,
                DetailLevel::Fine,
                &AtomicBool::new(false),
            ))
            .unwrap();
        world.jobs.push(Job {
            ticket: old,
            cancel: Arc::new(AtomicBool::new(true)),
            receiver,
            reservation: world.source.region_work_bytes(face),
        });
        world.poll().unwrap();
        assert_eq!(world.stats.rejected_results, 1);
        assert!(world.slots[face.index()].upload.is_none());
        assert!(!world.acknowledge_upload(old, 0));
        assert_eq!(
            world.resident_levels()[face.index()],
            Some(DetailLevel::Coarse)
        );
        let current = world.slots[face.index()].request;
        let (sender, receiver) = mpsc::sync_channel(1);
        sender
            .send(build_region(
                &world.source,
                face,
                DetailLevel::Fine,
                &AtomicBool::new(false),
            ))
            .unwrap();
        world.jobs.push(Job {
            ticket: current,
            cancel: Arc::new(AtomicBool::new(false)),
            receiver,
            reservation: world.source.region_work_bytes(face),
        });
        world.poll().unwrap();
        let mut pieces = Vec::new();
        while let Some(piece) = world.next_upload(UPLOAD_BYTES_PER_FRAME) {
            pieces.push(piece);
        }
        assert!(pieces.len() > 1);
        let last = pieces.pop().unwrap();
        for piece in pieces {
            assert!(world.acknowledge_upload(piece.ticket, piece.index));
            assert_eq!(
                world.resident_levels()[face.index()],
                Some(DetailLevel::Coarse)
            );
        }
        assert!(world.acknowledge_upload(last.ticket, last.index));
        assert_eq!(
            world.resident_levels()[face.index()],
            Some(DetailLevel::Fine)
        );
        let old_resident = world.retired[0].ticket;
        let before_retirement = world.bytes();
        world.request(face, DetailLevel::Coarse);
        assert!(!world.needed(face.index()));
        world.acknowledge_retirement(old_resident);
        assert!(world.bytes() < before_retirement);
        assert!(world.needed(face.index()));
        assert!(world.stats().peak_managed_bytes <= MANAGED_MEMORY_LIMIT);
        assert!(world.stats().peak_active_jobs <= MAX_ACTIVE_JOBS);
        assert!(world.stats().peak_queued_jobs <= 6);
        let looking_away = StreamView {
            position: Vec3::X * 4.2,
            forward: Vec3::X,
        };
        world
            .update(looking_away, DetailFocus::NearbyCollision)
            .unwrap();
        assert_eq!(
            world.requested_tickets()[face.index()].detail,
            DetailLevel::Fine
        );
        world.update(looking_away, DetailFocus::View).unwrap();
        assert_eq!(
            world.requested_tickets()[face.index()].detail,
            DetailLevel::Medium
        );
    }
}
