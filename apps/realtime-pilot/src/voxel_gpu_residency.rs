//! Backend-neutral leases and atomic local publication. Completion is explicit.
use crate::voxel_neighborhood::touching;
use crate::voxel_selection::distance_squared;
use crate::{VoxelCoverage, VoxelMeshKey, VoxelPosition};
use std::{collections::BTreeSet, error::Error, fmt};

#[derive(Clone, Copy, Debug)]
pub struct VoxelStreamConfig {
    pub max_slots: usize,
    pub max_in_flight: usize,
}
/// Ceiling on the jobs one stream encodes before reading a completion back. Each
/// holds a working set of a few megabytes outside the slot reservation, and the
/// owner of the render queue drains that many submissions per frame, so this
/// bounds both. The viewer runs at eight; the ceiling leaves room for a wider
/// setting without changing validation, which used to hard-code the old value.
const MAX_JOBS_IN_FLIGHT: usize = 32;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoxelStreamError {
    SlotLimit,
    JobLimit { max: usize },
    Capacity,
}
impl fmt::Display for VoxelStreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SlotLimit => write!(
                f,
                "GPU slot limit must be in 1..={}",
                crate::MAX_VOXEL_COVERAGE_LEAVES
            ),
            Self::JobLimit { max } => write!(f, "GPU in-flight limit must be in 1..={max}"),
            Self::Capacity => f.write_str(
                "GPU slot pool cannot retain old coverage through the largest local replacement",
            ),
        }
    }
}
impl Error for VoxelStreamError {}
impl VoxelStreamConfig {
    pub fn validate(self) -> Result<(), VoxelStreamError> {
        if !(1..=crate::MAX_VOXEL_COVERAGE_LEAVES).contains(&self.max_slots) {
            return Err(VoxelStreamError::SlotLimit);
        }
        let max = MAX_JOBS_IN_FLIGHT.min(self.max_slots);
        if !(1..=max).contains(&self.max_in_flight) {
            return Err(VoxelStreamError::JobLimit { max });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct VoxelGpuTicket {
    slot: usize,
    generation: u64,
}
impl VoxelGpuTicket {
    pub fn slot(self) -> usize {
        self.slot
    }
}
#[derive(Clone, Debug)]
pub struct VoxelGpuRequest {
    pub ticket: VoxelGpuTicket,
    pub key: VoxelMeshKey,
}
#[derive(Clone, Copy, Debug)]
pub enum VoxelGpuOutcome {
    Ready,
    Failed,
}
#[derive(Default)]
pub struct VoxelPublication {
    pub installed: Vec<VoxelGpuTicket>,
    pub retired: Vec<VoxelGpuTicket>,
}
struct Slot {
    generation: u64,
    state: State,
}
enum State {
    Free,
    Working(VoxelMeshKey),
    Discarding,
    Ready(VoxelMeshKey),
    Resident(VoxelMeshKey),
    Retiring(u64),
}
struct Group {
    old: Vec<VoxelGpuTicket>,
    new: Vec<VoxelMeshKey>,
}

pub struct VoxelGpuResidency {
    config: VoxelStreamConfig,
    slots: Vec<Slot>,
    desired: BTreeSet<VoxelMeshKey>,
    groups: Vec<Group>,
    failed: BTreeSet<VoxelMeshKey>,
    camera: VoxelPosition,
}
impl VoxelGpuResidency {
    pub fn new(config: VoxelStreamConfig) -> Result<Self, VoxelStreamError> {
        config.validate()?;
        Ok(Self {
            config,
            slots: (0..config.max_slots)
                .map(|_| Slot {
                    generation: 0,
                    state: State::Free,
                })
                .collect(),
            desired: BTreeSet::new(),
            groups: Vec::new(),
            failed: BTreeSet::new(),
            camera: VoxelPosition {
                x_m: 0,
                y_m: 0,
                z_m: 0,
            },
        })
    }
    pub fn residents(&self) -> impl Iterator<Item = (VoxelGpuTicket, &VoxelMeshKey)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            if let State::Resident(key) = &s.state {
                Some((
                    VoxelGpuTicket {
                        slot: i,
                        generation: s.generation,
                    },
                    key,
                ))
            } else {
                None
            }
        })
    }
    pub fn in_flight(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| matches!(s.state, State::Working(_) | State::Discarding))
            .count()
    }
    pub fn retiring(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| matches!(s.state, State::Retiring(_)))
            .count()
    }
    pub fn failed(&self) -> impl Iterator<Item = &VoxelMeshKey> {
        self.failed.iter()
    }
    pub fn settled(&self) -> bool {
        self.groups.is_empty() && self.in_flight() == 0
    }
    /// A failed request stays failed until the caller explicitly retries it.
    pub fn retry_failed(&mut self) {
        self.failed.clear();
    }
    pub fn set_coverage(
        &mut self,
        coverage: &VoxelCoverage,
        camera: VoxelPosition,
    ) -> Result<(), VoxelStreamError> {
        let desired: BTreeSet<_> = coverage.keys().collect();
        self.camera = camera;
        if desired == self.desired {
            return Ok(());
        }
        let groups = replacement_groups(self.residents(), &desired);
        // Reserve headroom for one complete local group. Admissions focus on one
        // group at a time so ready fragments cannot fill the pool and deadlock.
        let largest = groups.iter().map(|g| g.new.len()).max().unwrap_or(0);
        let resident_count = self.residents().count();
        let required = if resident_count == 0 {
            desired.len()
        } else {
            resident_count.max(desired.len()) + largest
        };
        if required > self.config.max_slots {
            return Err(VoxelStreamError::Capacity);
        }
        self.desired = desired;
        self.groups = groups;
        self.failed.retain(|key| self.desired.contains(key));
        // A changed selection can split pending work across several groups.
        // Retain one group; discarded commands keep their slots until completion.
        let active = self
            .next_group()
            .map(|group| group.new.clone())
            .unwrap_or_default();
        for slot in &mut self.slots {
            match &slot.state {
                State::Working(key) if !active.contains(key) => slot.state = State::Discarding,
                State::Ready(key) if !active.contains(key) => slot.state = State::Free,
                _ => {}
            }
        }
        Ok(())
    }
    pub fn admit(&mut self) -> Option<VoxelGpuRequest> {
        if self.in_flight() >= self.config.max_in_flight {
            return None;
        }
        let group = self.next_group()?;
        let key = group
            .new
            .iter()
            .filter(|key| {
                !self.slots.iter().any(|slot| match &slot.state {
                    State::Working(k) | State::Ready(k) | State::Resident(k) => k == *key,
                    _ => false,
                })
            })
            .min_by(|a, b| {
                distance_squared(a.address(), self.camera)
                    .total_cmp(&distance_squared(b.address(), self.camera))
                    .then_with(|| a.cmp(b))
            })?
            .clone();
        let (slot, entry) = self
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, s)| matches!(s.state, State::Free))?;
        entry.generation = entry
            .generation
            .checked_add(1)
            .expect("GPU slot generation exhausted");
        entry.state = State::Working(key.clone());
        Some(VoxelGpuRequest {
            ticket: VoxelGpuTicket {
                slot,
                generation: entry.generation,
            },
            key,
        })
    }
    fn next_group(&self) -> Option<&Group> {
        self.groups
            .iter()
            .filter(|g| !g.new.is_empty() && !g.new.iter().any(|k| self.failed.contains(k)))
            .min_by(|a, b| {
                let started = |g: &Group| {
                    self.slots.iter().any(|slot| match &slot.state {
                        State::Working(k) | State::Ready(k) => g.new.contains(k),
                        _ => false,
                    })
                };
                let distance = |g: &Group| {
                    g.new
                        .iter()
                        .map(|k| distance_squared(k.address(), self.camera))
                        .min_by(f32::total_cmp)
                        .unwrap()
                };
                // Finish admitted work even when the camera changes priority.
                started(b)
                    .cmp(&started(a))
                    .then_with(|| distance(a).total_cmp(&distance(b)))
                    .then_with(|| a.new.cmp(&b.new))
            })
    }
    /// Call only after all commands using this destination have completed. A
    /// cancelled job keeps its slot until this acknowledgement, including ABA revisits.
    pub fn complete(&mut self, ticket: VoxelGpuTicket, outcome: VoxelGpuOutcome) -> bool {
        let Some(slot) = self.slots.get_mut(ticket.slot) else {
            return false;
        };
        if slot.generation != ticket.generation {
            return false;
        }
        let failed = match (&slot.state, outcome) {
            (State::Working(key), VoxelGpuOutcome::Failed) => Some(key.clone()),
            _ => None,
        };
        slot.state = match &slot.state {
            State::Working(key) => match outcome {
                VoxelGpuOutcome::Ready => State::Ready(key.clone()),
                VoxelGpuOutcome::Failed => State::Free,
            },
            State::Discarding => State::Free,
            _ => return false,
        };
        if let Some(key) = failed {
            self.failed.insert(key.clone());
            let group = self
                .groups
                .iter()
                .find(|g| g.new.contains(&key))
                .expect("working replacement group");
            // Release unpublished siblings after a failure; independent local
            // replacements must not be starved by fragments of a failed group.
            for slot in &mut self.slots {
                match &slot.state {
                    State::Ready(k) if group.new.contains(k) => slot.state = State::Free,
                    State::Working(k) if group.new.contains(k) => slot.state = State::Discarding,
                    _ => {}
                }
            }
        }
        true
    }

    /// retire_after must cover the last GPU submission that can use any current
    /// resident draw. Publication changes CPU visibility atomically per local group.
    pub fn publish(&mut self, retire_after: u64) -> VoxelPublication {
        let mut publication = VoxelPublication::default();
        for group in &self.groups {
            let ready: Option<Vec<_>> = group
                .new
                .iter()
                .map(|key| {
                    self.slots
                        .iter()
                        .enumerate()
                        .find_map(|(i, s)| match &s.state {
                            State::Ready(k) if k == key => Some(VoxelGpuTicket {
                                slot: i,
                                generation: s.generation,
                            }),
                            _ => None,
                        })
                })
                .collect();
            let Some(ready) = ready else {
                continue;
            };
            for ticket in ready {
                let slot = &mut self.slots[ticket.slot];
                let State::Ready(key) = std::mem::replace(&mut slot.state, State::Free) else {
                    unreachable!("ready group member")
                };
                slot.state = State::Resident(key);
                publication.installed.push(ticket);
            }
            for &ticket in &group.old {
                self.slots[ticket.slot].state = State::Retiring(retire_after);
                publication.retired.push(ticket);
            }
        }
        if !publication.installed.is_empty() || !publication.retired.is_empty() {
            self.groups = replacement_groups(self.residents(), &self.desired);
        }
        publication
    }
    pub fn release_completed(&mut self, completed_submission: u64) {
        self.release_unleased(completed_submission, |_| true);
    }
    pub(crate) fn release_unleased(
        &mut self,
        completed_submission: u64,
        unleased: impl Fn(usize) -> bool,
    ) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if matches!(slot.state,State::Retiring(until) if until <= completed_submission)
                && unleased(index)
            {
                slot.state = State::Free;
            }
        }
    }
}

fn replacement_groups<'a>(
    residents: impl Iterator<Item = (VoxelGpuTicket, &'a VoxelMeshKey)>,
    desired: &BTreeSet<VoxelMeshKey>,
) -> Vec<Group> {
    let residents: Vec<_> = residents.collect();
    let old: Vec<_> = residents
        .iter()
        .filter(|(_, k)| !desired.contains(*k))
        .map(|(t, k)| (*t, (*k).clone()))
        .collect();
    let new: Vec<_> = desired
        .iter()
        .filter(|k| !residents.iter().any(|(_, r)| r == k))
        .cloned()
        .collect();
    let addresses: Vec<_> = old
        .iter()
        .map(|(_, k)| k.address())
        .chain(new.iter().map(|k| k.address()))
        .collect();
    let mut seen = vec![false; addresses.len()];
    let mut groups = Vec::new();
    for start in 0..addresses.len() {
        if seen[start] {
            continue;
        }
        let mut group = Group {
            old: Vec::new(),
            new: Vec::new(),
        };
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            if i < old.len() {
                group.old.push(old[i].0);
            } else {
                group.new.push(new[i - old.len()].clone());
            }
            for j in 0..addresses.len() {
                if !seen[j] && touching(addresses[i], addresses[j]) {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        group.new.sort();
        groups.push(group);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VoxelChunkAddress;
    fn camera(x: i32) -> VoxelPosition {
        VoxelPosition {
            x_m: x,
            y_m: 0,
            z_m: 0,
        }
    }
    fn chunk(x: i32, lod: u8) -> VoxelChunkAddress {
        VoxelChunkAddress::containing(camera(x), lod).unwrap()
    }
    fn settle(state: &mut VoxelGpuResidency, fence: u64) {
        for _ in 0..100 {
            let mut jobs = Vec::new();
            while let Some(job) = state.admit() {
                jobs.push(job);
            }
            for job in jobs.into_iter().rev() {
                assert!(state.complete(job.ticket, VoxelGpuOutcome::Ready));
            }
            state.publish(fence);
            state.release_completed(fence);
            if state.settled() {
                return;
            }
        }
        panic!("stream did not settle");
    }
    #[test]
    fn replacements_are_local_atomic_and_retirement_waits_for_draws() {
        let near = chunk(0, 1);
        let distant = chunk(1024, 1);
        let initial = VoxelCoverage::new(vec![near, distant]).unwrap();
        let mut stream = VoxelGpuResidency::new(VoxelStreamConfig {
            max_slots: 24,
            max_in_flight: 8,
        })
        .unwrap();
        stream.set_coverage(&initial, camera(0)).unwrap();
        settle(&mut stream, 1);
        let old: Vec<_> = stream.residents().map(|(t, k)| (t, k.clone())).collect();
        let mut leaves = near.children().unwrap().to_vec();
        leaves.push(distant);
        stream
            .set_coverage(&VoxelCoverage::new(leaves).unwrap(), camera(1))
            .unwrap();
        let mut jobs = Vec::new();
        while let Some(j) = stream.admit() {
            jobs.push(j);
        }
        assert_eq!(jobs.len(), 8);
        for j in &jobs[..7] {
            stream.complete(j.ticket, VoxelGpuOutcome::Ready);
        }
        assert!(stream.publish(50).installed.is_empty());
        assert_eq!(stream.residents().count(), 2);
        stream.complete(jobs[7].ticket, VoxelGpuOutcome::Ready);
        let publication = stream.publish(50);
        assert_eq!(publication.installed.len(), 8);
        assert_eq!(publication.retired.len(), 1);
        let distant_ticket = old.iter().find(|(_, k)| k.address() == distant).unwrap().0;
        assert!(
            stream
                .residents()
                .any(|(t, k)| t == distant_ticket && k.address() == distant)
        );
        stream.release_completed(49);
        assert_eq!(stream.retiring(), 1);
        stream.release_completed(50);
        assert_eq!(stream.retiring(), 0);
        let before: Vec<_> = stream.residents().map(|(t, _)| t).collect();
        let unchanged =
            VoxelCoverage::new(stream.residents().map(|(_, k)| k.address()).collect()).unwrap();
        stream.set_coverage(&unchanged, camera(2)).unwrap();
        assert!(stream.admit().is_none());
        assert_eq!(
            before,
            stream.residents().map(|(t, _)| t).collect::<Vec<_>>()
        );
    }
    #[test]
    fn cancelled_and_duplicate_completions_cannot_reuse_or_publish_an_aba_slot() {
        let a = VoxelCoverage::new(vec![chunk(0, 0)]).unwrap();
        let empty = VoxelCoverage::new(vec![]).unwrap();
        let mut stream = VoxelGpuResidency::new(VoxelStreamConfig {
            max_slots: 1,
            max_in_flight: 1,
        })
        .unwrap();
        stream.set_coverage(&a, camera(0)).unwrap();
        let old = stream.admit().unwrap();
        stream.set_coverage(&empty, camera(0)).unwrap();
        stream.set_coverage(&a, camera(0)).unwrap();
        assert!(stream.admit().is_none());
        stream.complete(old.ticket, VoxelGpuOutcome::Ready);
        assert!(stream.publish(1).installed.is_empty());
        let new = stream.admit().unwrap();
        assert_eq!(old.ticket.slot(), new.ticket.slot());
        assert_ne!(old.ticket, new.ticket);
        assert!(!stream.complete(old.ticket, VoxelGpuOutcome::Ready));
        stream.complete(new.ticket, VoxelGpuOutcome::Ready);
        stream.publish(2);
        assert_eq!(stream.residents().next().unwrap().0, new.ticket);
        assert!(!stream.complete(new.ticket, VoxelGpuOutcome::Failed));
    }
    #[test]
    fn failed_groups_retain_old_coverage_and_do_not_block_independent_groups() {
        let a = chunk(0, 1);
        let b = chunk(1024, 1);
        let mut stream = VoxelGpuResidency::new(VoxelStreamConfig {
            max_slots: 40,
            max_in_flight: 8,
        })
        .unwrap();
        stream
            .set_coverage(&VoxelCoverage::new(vec![a, b]).unwrap(), camera(0))
            .unwrap();
        settle(&mut stream, 1);
        let targets = VoxelCoverage::new(
            a.children()
                .unwrap()
                .into_iter()
                .chain(b.children().unwrap())
                .collect(),
        )
        .unwrap();
        stream.set_coverage(&targets, camera(0)).unwrap();
        let mut jobs = Vec::new();
        while let Some(j) = stream.admit() {
            jobs.push(j);
        }
        assert!(jobs.iter().all(|j| j.key.address().parent() == Some(a)));
        stream.complete(jobs[0].ticket, VoxelGpuOutcome::Ready);
        stream.complete(jobs[1].ticket, VoxelGpuOutcome::Failed);
        for j in &jobs[2..] {
            stream.complete(j.ticket, VoxelGpuOutcome::Ready);
        }
        assert!(stream.publish(2).installed.is_empty());
        for _ in 0..8 {
            let j = stream.admit().unwrap();
            assert_eq!(j.key.address().parent(), Some(b));
            stream.complete(j.ticket, VoxelGpuOutcome::Ready);
        }
        let p = stream.publish(3);
        assert_eq!(p.installed.len(), 8);
        assert!(stream.residents().any(|(_, k)| k.address() == a));
        stream.release_completed(3);
        stream.retry_failed();
        settle(&mut stream, 4);
        assert_eq!(stream.residents().count(), 16);
    }
    #[test]
    fn camera_reprioritization_finishes_one_group_with_bounded_headroom() {
        let mut stream = VoxelGpuResidency::new(VoxelStreamConfig {
            max_slots: 6,
            max_in_flight: 1,
        })
        .unwrap();
        let initial =
            VoxelCoverage::new([0, 32, 1024, 1056].map(|x| chunk(x, 0)).to_vec()).unwrap();
        stream.set_coverage(&initial, camera(0)).unwrap();
        settle(&mut stream, 1);
        let target =
            VoxelCoverage::new([64, 96, 1088, 1120].map(|x| chunk(x, 0)).to_vec()).unwrap();
        stream.set_coverage(&target, camera(64)).unwrap();
        let first = stream.admit().unwrap();
        assert_eq!(first.key.address(), chunk(64, 0));
        stream.complete(first.ticket, VoxelGpuOutcome::Ready);
        assert!(stream.publish(2).installed.is_empty());
        stream.set_coverage(&target, camera(1088)).unwrap();
        let second = stream.admit().unwrap();
        assert_eq!(second.key.address(), chunk(96, 0));
        stream.complete(second.ticket, VoxelGpuOutcome::Ready);
        assert_eq!(stream.publish(3).installed.len(), 2);
        stream.release_completed(3);
        settle(&mut stream, 4);
        assert_eq!(stream.residents().count(), 4);
    }
    #[test]
    fn insufficient_replacement_headroom_preserves_selection() {
        let a = chunk(0, 1);
        let initial = VoxelCoverage::new(vec![a]).unwrap();
        let mut stream = VoxelGpuResidency::new(VoxelStreamConfig {
            max_slots: 8,
            max_in_flight: 2,
        })
        .unwrap();
        stream.set_coverage(&initial, camera(0)).unwrap();
        settle(&mut stream, 1);
        let target = VoxelCoverage::new(a.children().unwrap().to_vec()).unwrap();
        assert_eq!(
            stream.set_coverage(&target, camera(0)),
            Err(VoxelStreamError::Capacity)
        );
        assert!(stream.settled());
        assert_eq!(stream.residents().next().unwrap().1.address(), a);
    }
}
