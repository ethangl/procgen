//! Camera-driven, bounded CPU density residency; no renderer or collision policy.
use crate::voxel_density::sample_voxel_chunk_cancellable;
use crate::voxel_selection::{distance_squared, select_leaves};
use crate::{
    PlanetDesignField, VOXEL_DENSITY_BYTES, VoxelChunkAddress, VoxelPosition, VoxelVolume,
    VoxelVolumeError,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread::JoinHandle,
};

const MAX_LEAVES: usize = 4096;
const MAX_JOBS: usize = 8;
const MAX_DENSITY_REACH_M: f32 = 1_000_000.0;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct VoxelResidencyConfig {
    /// Hard cap on leaf metadata, including distant coverage.
    pub max_leaves: usize,
    /// Cancelled workers continue to occupy slots until they acknowledge exit.
    pub max_jobs: usize,
    /// Only leaves intersecting this camera-centered reach allocate density.
    pub density_reach_m: f32,
}
impl VoxelResidencyConfig {
    pub fn density_budget_bytes(self) -> usize {
        (self.max_leaves + self.max_jobs) * VOXEL_DENSITY_BYTES
    }
    fn validate(self) -> Result<(), VoxelResidencyError> {
        if !(1..=MAX_LEAVES).contains(&self.max_leaves) {
            return Err(VoxelResidencyError::LeafLimit);
        }
        if !(1..=MAX_JOBS).contains(&self.max_jobs) {
            return Err(VoxelResidencyError::JobLimit);
        }
        if !self.density_reach_m.is_finite()
            || self.density_reach_m <= 0.0
            || self.density_reach_m > MAX_DENSITY_REACH_M
        {
            return Err(VoxelResidencyError::Reach);
        }
        Ok(())
    }
}
impl Default for VoxelResidencyConfig {
    fn default() -> Self {
        Self {
            max_leaves: 256,
            max_jobs: 2,
            density_reach_m: 4096.0,
        }
    }
}

#[derive(Debug)]
pub enum VoxelResidencyError {
    LeafLimit,
    JobLimit,
    Reach,
    WorkerStopped,
    WorkerSpawn(std::io::Error),
    Volume(VoxelVolumeError),
}
impl From<VoxelVolumeError> for VoxelResidencyError {
    fn from(value: VoxelVolumeError) -> Self {
        Self::Volume(value)
    }
}
impl fmt::Display for VoxelResidencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LeafLimit => write!(f, "voxel leaf limit must be in 1..={MAX_LEAVES}"),
            Self::JobLimit => write!(f, "voxel worker limit must be in 1..={MAX_JOBS}"),
            Self::Reach => {
                write!(
                    f,
                    "voxel density reach must be finite and in (0, {MAX_DENSITY_REACH_M}] meters"
                )
            }
            Self::WorkerStopped => f.write_str("voxel worker stopped without completing its job"),
            Self::WorkerSpawn(e) => e.fmt(f),
            Self::Volume(e) => e.fmt(f),
        }
    }
}
impl Error for VoxelResidencyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::WorkerSpawn(e) => Some(e),
            Self::Volume(e) => Some(e),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Request {
    address: VoxelChunkAddress,
    serial: u64,
}
struct Job {
    request: Request,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<Option<VoxelVolume>, VoxelVolumeError>>,
    worker: JoinHandle<()>,
}
#[derive(Clone, Copy, Default, Debug, Serialize)]
pub struct VoxelResidencyStats {
    pub leaves: usize,
    pub requested_chunks: usize,
    pub resident_chunks: usize,
    pub active_jobs: usize,
    pub queued_jobs: usize,
    pub density_bytes: usize,
    pub peak_density_bytes: usize,
    pub peak_active_jobs: usize,
    pub installations: usize,
    pub evictions: usize,
    pub cancellations: usize,
    pub rejected_results: usize,
}

pub struct VoxelResidency {
    field: Arc<PlanetDesignField>,
    config: VoxelResidencyConfig,
    camera: Option<VoxelPosition>,
    leaves: Vec<VoxelChunkAddress>,
    requests: BTreeMap<VoxelChunkAddress, u64>,
    residents: BTreeMap<VoxelChunkAddress, VoxelVolume>,
    jobs: Vec<Job>,
    serial: u64,
    history: VoxelResidencyStats,
}
impl VoxelResidency {
    pub fn new(
        field: Arc<PlanetDesignField>,
        config: VoxelResidencyConfig,
    ) -> Result<Self, VoxelResidencyError> {
        config.validate()?;
        Ok(Self {
            field,
            config,
            camera: None,
            leaves: Vec::new(),
            requests: BTreeMap::new(),
            residents: BTreeMap::new(),
            jobs: Vec::new(),
            serial: 0,
            history: VoxelResidencyStats::default(),
        })
    }
    pub fn leaves(&self) -> &[VoxelChunkAddress] {
        &self.leaves
    }
    pub fn residents(&self) -> impl Iterator<Item = &VoxelVolume> {
        self.residents.values()
    }
    pub fn settled(&self) -> bool {
        self.camera.is_some() && self.jobs.is_empty() && self.residents.len() == self.requests.len()
    }
    pub fn stats(&self) -> VoxelResidencyStats {
        VoxelResidencyStats {
            leaves: self.leaves.len(),
            requested_chunks: self.requests.len(),
            resident_chunks: self.residents.len(),
            active_jobs: self.jobs.len(),
            queued_jobs: self.requests.keys().filter(|a| self.needed(**a)).count(),
            density_bytes: self
                .residents
                .values()
                .map(VoxelVolume::allocated_bytes)
                .sum::<usize>()
                + self.jobs.len() * VOXEL_DENSITY_BYTES,
            ..self.history
        }
    }
    fn needed(&self, address: VoxelChunkAddress) -> bool {
        !self.residents.contains_key(&address)
            && !self.jobs.iter().any(|job| job.request.address == address)
    }
    /// Each update performs at most max_jobs completions and admissions. Camera
    /// changes rebuild at most max_leaves leaves; stationary frames skip traversal.
    pub fn update(&mut self, camera: VoxelPosition) -> Result<(), VoxelResidencyError> {
        if self.camera != Some(camera) {
            self.reselect(camera);
        }
        self.poll()?;
        while self.jobs.len() < self.config.max_jobs {
            let next = self
                .requests
                .keys()
                .copied()
                .filter(|a| self.needed(*a))
                .min_by(|a, b| {
                    distance_squared(*a, camera)
                        .total_cmp(&distance_squared(*b, camera))
                        .then_with(|| a.cmp(b))
                });
            let Some(address) = next else {
                break;
            };
            let request = Request {
                address,
                serial: self.requests[&address],
            };
            let field = Arc::clone(&self.field);
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_cancel = Arc::clone(&cancel);
            let (sender, receiver) = mpsc::sync_channel(1);
            let worker = std::thread::Builder::new()
                .name("voxel-density".into())
                .spawn(move || {
                    let result = sample_voxel_chunk_cancellable(&field, address, &worker_cancel);
                    let _ = sender.send(result);
                })
                .map_err(VoxelResidencyError::WorkerSpawn)?;
            self.jobs.push(Job {
                request,
                cancel,
                receiver,
                worker,
            });
        }
        let stats = self.stats();
        self.history.peak_density_bytes = self.history.peak_density_bytes.max(stats.density_bytes);
        self.history.peak_active_jobs = self.history.peak_active_jobs.max(stats.active_jobs);
        debug_assert!(stats.density_bytes <= self.config.density_budget_bytes());
        Ok(())
    }
    fn reselect(&mut self, camera: VoxelPosition) {
        self.camera = Some(camera);
        self.leaves = select_leaves(&self.field, camera, self.config.max_leaves);
        let mut requests = BTreeMap::new();
        for &address in &self.leaves {
            if distance_squared(address, camera) > self.config.density_reach_m.powi(2) {
                continue;
            }
            let serial = match self.requests.get(&address) {
                Some(&serial) => serial,
                None => {
                    self.serial = self
                        .serial
                        .checked_add(1)
                        .expect("request serial exhausted");
                    self.serial
                }
            };
            requests.insert(address, serial);
        }
        self.requests = requests;
        for job in &self.jobs {
            if self.requests.get(&job.request.address) != Some(&job.request.serial)
                && !job.cancel.swap(true, Ordering::Relaxed)
            {
                self.history.cancellations += 1;
            }
        }
        let before = self.residents.len();
        self.residents
            .retain(|address, _| self.requests.contains_key(address));
        self.history.evictions += before - self.residents.len();
    }
    fn poll(&mut self) -> Result<(), VoxelResidencyError> {
        let mut i = 0;
        while i < self.jobs.len() {
            let result = match self.jobs[i].receiver.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => {
                    i += 1;
                    continue;
                }
                Err(TryRecvError::Disconnected) => return Err(VoxelResidencyError::WorkerStopped),
            };
            let job = self.jobs.remove(i);
            job.worker
                .join()
                .map_err(|_| VoxelResidencyError::WorkerStopped)?;
            if self.requests.get(&job.request.address) != Some(&job.request.serial) {
                self.history.rejected_results += 1;
                continue;
            }
            let Some(volume) = result? else {
                // A current request is never cancelled. Revisit requests have a
                // new serial, even when the obsolete address happens to match.
                return Err(VoxelResidencyError::WorkerStopped);
            };
            debug_assert_eq!(volume.address(), job.request.address);
            debug_assert!(volume.allocated_bytes() <= VOXEL_DENSITY_BYTES);
            self.residents.insert(volume.address(), volume);
            self.history.installations += 1;
        }
        Ok(())
    }
}
impl Drop for VoxelResidency {
    fn drop(&mut self) {
        for job in &self.jobs {
            job.cancel.store(true, Ordering::Relaxed);
        }
        // Keep reservations alive through worker exit, including when replacing
        // a design. No detached allocations survive this owner.
        for job in self.jobs.drain(..) {
            let _ = job.worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PlanetDesignConfig, sample_voxel_chunk};
    use std::time::{Duration, Instant};

    fn field() -> Arc<PlanetDesignField> {
        let mut config = PlanetDesignConfig::starter(42);
        config.radius_m = 8_000_000.0;
        config.octaves.truncate(1);
        Arc::new(config.validate().unwrap())
    }
    fn ground(field: &PlanetDesignField) -> VoxelPosition {
        VoxelPosition {
            x_m: (field.config().radius_m + field.height(procgen_core::Vec3::X, 0.0)).round()
                as i32,
            y_m: 0,
            z_m: 0,
        }
    }
    fn settle(world: &mut VoxelResidency, camera: VoxelPosition) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            world.update(camera).unwrap();
            let stats = world.stats();
            assert!(stats.density_bytes <= world.config.density_budget_bytes());
            assert!(stats.active_jobs <= world.config.max_jobs);
            if world.settled() {
                break;
            }
            assert!(Instant::now() < deadline, "stream must make progress");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn stale_completion_cannot_install_after_leaving_and_revisiting_same_address() {
        let field = field();
        let point = ground(&field);
        let mut world =
            VoxelResidency::new(Arc::clone(&field), VoxelResidencyConfig::default()).unwrap();
        world.reselect(point);
        let address = *world.requests.keys().next().unwrap();
        let request = Request {
            address,
            serial: world.requests[&address],
        };
        let volume = sample_voxel_chunk(&field, address).unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            wait.recv().unwrap();
            // Emulate a worker completing just before it observes cancellation.
            sender.send(Ok(Some(volume))).unwrap();
        });
        world.jobs.push(Job {
            request,
            cancel: Arc::new(AtomicBool::new(false)),
            receiver,
            worker,
        });
        world.reselect(VoxelPosition {
            x_m: 100_000_000,
            y_m: 0,
            z_m: 0,
        });
        assert_eq!(world.stats().cancellations, 1);
        assert_eq!(world.stats().density_bytes, VOXEL_DENSITY_BYTES);
        world.reselect(point);
        assert_ne!(world.requests[&address], request.serial);
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !world.jobs.is_empty() {
            world.poll().unwrap();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(world.stats().rejected_results, 1);
        assert_eq!(world.stats().installations, 0);
        assert_eq!(world.stats().density_bytes, 0);
        assert!(world.needed(address));
    }

    #[test]
    fn worker_counts_and_evicted_revisits_preserve_every_density_sample() {
        let field = field();
        let camera = ground(&field);
        let config = VoxelResidencyConfig {
            max_leaves: 160,
            max_jobs: 1,
            density_reach_m: 32.0,
        };
        let mut serial = VoxelResidency::new(Arc::clone(&field), config).unwrap();
        let mut parallel = VoxelResidency::new(
            Arc::clone(&field),
            VoxelResidencyConfig {
                max_jobs: 4,
                ..config
            },
        )
        .unwrap();
        settle(&mut serial, camera);
        settle(&mut parallel, camera);
        assert!(!serial.residents.is_empty());
        let compare = |a: &VoxelResidency, b: &VoxelResidency| {
            assert_eq!(a.leaves(), b.leaves());
            assert_eq!(
                a.residents.keys().collect::<Vec<_>>(),
                b.residents.keys().collect::<Vec<_>>()
            );
            for (address, volume) in &a.residents {
                assert!(volume.samples().eq(b.residents[address].samples()));
            }
        };
        compare(&serial, &parallel);
        let before = parallel.stats().installations;
        settle(
            &mut parallel,
            VoxelPosition {
                x_m: 100_000_000,
                y_m: 0,
                z_m: 0,
            },
        );
        assert!(parallel.residents.is_empty());
        assert_eq!(parallel.stats().density_bytes, 0);
        assert_eq!(parallel.stats().evictions, before);
        assert_eq!(parallel.leaves(), &[VoxelChunkAddress::root()]);
        settle(&mut parallel, camera);
        compare(&serial, &parallel);
        assert!(parallel.stats().installations > before);
    }

    #[test]
    fn completed_jobs_install_in_either_order_and_cancelled_sampling_returns_no_partial_volume() {
        let field = field();
        let camera = ground(&field);
        let config = VoxelResidencyConfig::default();
        let mut a = VoxelResidency::new(Arc::clone(&field), config).unwrap();
        let mut b = VoxelResidency::new(Arc::clone(&field), config).unwrap();
        for world in [&mut a, &mut b] {
            world.reselect(camera);
        }
        let addresses: Vec<_> = a.requests.keys().take(2).copied().collect();
        for (world, reverse) in [(&mut a, false), (&mut b, true)] {
            let mut order = addresses.clone();
            if reverse {
                order.reverse();
            }
            for address in order {
                let volume = sample_voxel_chunk(&field, address).unwrap();
                let (sender, receiver) = mpsc::sync_channel(1);
                sender.send(Ok(Some(volume))).unwrap();
                world.jobs.push(Job {
                    request: Request {
                        address,
                        serial: world.requests[&address],
                    },
                    cancel: Arc::new(AtomicBool::new(false)),
                    receiver,
                    worker: std::thread::spawn(|| {}),
                });
                world.poll().unwrap();
            }
        }
        for (address, volume) in &a.residents {
            assert!(volume.samples().eq(b.residents[address].samples()));
        }
        assert!(
            sample_voxel_chunk_cancellable(&field, addresses[0], &AtomicBool::new(true))
                .unwrap()
                .is_none()
        );
        assert!(
            VoxelResidency::new(
                field,
                VoxelResidencyConfig {
                    max_jobs: 0,
                    ..config
                }
            )
            .is_err()
        );
    }
}
