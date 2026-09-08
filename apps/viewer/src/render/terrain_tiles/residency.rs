use std::collections::HashSet;

use bevy::prelude::Resource;
use procgen_cubesphere::{CubeFace, TileAddress};

use super::{MAX_RESIDENT_TILES, NEW_TERRAIN_TILES_PER_FRAME};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct ResidentTile {
    pub address: TileAddress,
    pub slot: u32,
}

#[derive(Default)]
pub(super) struct ResidencyUpdate {
    pub generated: Vec<ResidentTile>,
    pub displayed: Vec<ResidentTile>,
}

#[derive(Resource)]
pub(super) struct TileResidency {
    slots: Vec<Option<Slot>>,
    displayed: Vec<ResidentTile>,
    frame: u64,
}

#[derive(Clone, Copy)]
struct Slot {
    address: TileAddress,
    last_used: u64,
    ready: bool,
}

impl Default for TileResidency {
    fn default() -> Self {
        Self {
            slots: vec![None; MAX_RESIDENT_TILES],
            displayed: Vec::new(),
            frame: 0,
        }
    }
}

impl TileResidency {
    pub(super) fn update(&mut self, targets: &[TileAddress]) -> ResidencyUpdate {
        self.frame = self.frame.wrapping_add(1);
        let candidates = candidate_path(targets);
        let target_set = targets.iter().copied().collect::<HashSet<_>>();
        let relevant = candidates.iter().copied().collect::<HashSet<_>>();
        for address in &candidates {
            if let Some(slot) = self.find(*address) {
                self.slots[slot].as_mut().unwrap().last_used = self.frame;
            }
        }

        let mut protected = self
            .displayed
            .iter()
            .map(|tile| tile.address)
            .collect::<HashSet<_>>();
        protected.extend(
            candidates
                .iter()
                .copied()
                .filter(|address| self.find(*address).is_some()),
        );
        let mut generated = Vec::new();
        if !self.slots.iter().flatten().any(|slot| !slot.ready) {
            for address in candidates {
                if generated.len() == NEW_TERRAIN_TILES_PER_FRAME {
                    break;
                }
                if self.find(address).is_some() {
                    continue;
                }
                let Some(slot) = self.allocate(&protected) else {
                    break;
                };
                self.slots[slot] = Some(Slot {
                    address,
                    last_used: self.frame,
                    ready: false,
                });
                protected.insert(address);
                generated.push(ResidentTile {
                    address,
                    slot: slot as u32,
                });
            }
        }

        let mut displayed = Vec::new();
        for face in CubeFace::ALL {
            self.resolve(
                TileAddress::root(face),
                &target_set,
                &relevant,
                &mut displayed,
            );
        }
        displayed.sort_by_key(|tile| tile.address);
        displayed.dedup_by_key(|tile| tile.address);
        self.displayed.clone_from(&displayed);
        debug_assert!(self.resident_count() <= MAX_RESIDENT_TILES);
        ResidencyUpdate {
            generated,
            displayed,
        }
    }

    pub(super) fn resident_count(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    pub(super) fn complete_in_flight(&mut self) {
        for slot in self.slots.iter_mut().flatten() {
            slot.ready = true;
        }
    }

    fn resolve(
        &self,
        node: TileAddress,
        targets: &HashSet<TileAddress>,
        relevant: &HashSet<TileAddress>,
        output: &mut Vec<ResidentTile>,
    ) -> bool {
        if targets.contains(&node) {
            if let Some(slot) = self.find_ready(node) {
                output.push(ResidentTile {
                    address: node,
                    slot: slot as u32,
                });
                return true;
            }
            let descendants = self
                .displayed
                .iter()
                .copied()
                .filter(|tile| node.is_ancestor_of(tile.address));
            let before = output.len();
            output.extend(descendants);
            return output.len() > before;
        }
        if !relevant.contains(&node) {
            return true;
        }

        let checkpoint = output.len();
        let mut complete = true;
        for child in node.children().unwrap() {
            complete &= self.resolve(child, targets, relevant, output);
        }
        if complete {
            return true;
        }
        output.truncate(checkpoint);
        if let Some(slot) = self.find_ready(node) {
            output.push(ResidentTile {
                address: node,
                slot: slot as u32,
            });
            true
        } else {
            false
        }
    }

    fn find(&self, address: TileAddress) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| slot.is_some_and(|slot| slot.address == address))
    }

    fn find_ready(&self, address: TileAddress) -> Option<usize> {
        self.find(address)
            .filter(|&slot| self.slots[slot].unwrap().ready)
    }

    fn allocate(&mut self, protected: &HashSet<TileAddress>) -> Option<usize> {
        if let Some(slot) = self.slots.iter().position(Option::is_none) {
            return Some(slot);
        }
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.map(|slot| (index, slot)))
            .filter(|(_, slot)| !protected.contains(&slot.address))
            .min_by_key(|(index, slot)| (slot.last_used, slot.address, *index))
            .map(|(index, _)| index)
    }
}

pub(super) fn candidate_path(targets: &[TileAddress]) -> Vec<TileAddress> {
    let mut candidates = targets
        .iter()
        .flat_map(|target| std::iter::successors(Some(*target), |tile| tile.parent()))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|address| (address.level(), *address));
    candidates.dedup();
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_cubesphere::TileQuadrant;

    fn root() -> TileAddress {
        TileAddress::root(CubeFace::PositiveX)
    }

    fn complete(residency: &mut TileResidency) {
        residency.complete_in_flight();
    }

    #[test]
    fn split_handoff_keeps_parent_until_every_child_is_ready() {
        let mut residency = TileResidency::default();
        let root = root();
        let root_generation = residency.update(&[root]);
        assert!(root_generation.displayed.is_empty());
        complete(&mut residency);
        assert_eq!(
            residency.update(&[root]).displayed,
            vec![ResidentTile {
                address: root,
                slot: 0
            }]
        );
        let mut children = root.children().unwrap();
        children.sort();
        let update = residency.update(&children);
        assert_eq!(update.generated.len(), 4);
        assert_eq!(update.displayed[0].address, root);
        complete(&mut residency);
        let update = residency.update(&children);
        assert_eq!(
            update
                .displayed
                .iter()
                .map(|tile| tile.address)
                .collect::<Vec<_>>(),
            children
        );
    }

    #[test]
    fn incomplete_split_keeps_the_parent() {
        let mut residency = TileResidency::default();
        let root = TileAddress::root(CubeFace::NegativeZ);
        residency.update(&[root]);
        complete(&mut residency);
        residency.update(&[root]);
        let grandchildren = root
            .children()
            .unwrap()
            .into_iter()
            .flat_map(|child| child.children().unwrap())
            .collect::<Vec<_>>();
        let mut targets = CubeFace::ALL[..5]
            .iter()
            .copied()
            .map(TileAddress::root)
            .collect::<Vec<_>>();
        targets.extend(grandchildren);
        let update = residency.update(&targets);
        assert!(update.displayed.iter().any(|tile| tile.address == root));
        assert!(
            !update
                .displayed
                .iter()
                .any(|tile| tile.address.parent() == Some(root))
        );
    }

    #[test]
    fn merge_handoff_retains_children_until_parent_is_ready() {
        let mut residency = TileResidency::default();
        let root = root();
        let mut children = root.children().unwrap();
        children.sort();
        residency.update(&children);
        complete(&mut residency);
        residency.update(&children);
        let slot = residency.find(root).unwrap();
        residency.slots[slot] = None;
        let update = residency.update(&[root]);
        assert_eq!(
            update.generated,
            vec![ResidentTile {
                address: root,
                slot: slot as u32
            }]
        );
        assert_eq!(
            update.displayed,
            children
                .into_iter()
                .map(|address| ResidentTile {
                    address,
                    slot: residency.find(address).unwrap() as u32,
                })
                .collect::<Vec<_>>()
        );
        complete(&mut residency);
        assert_eq!(residency.update(&[root]).displayed[0].address, root);
    }

    #[test]
    fn generation_budget_and_capacity_are_enforced() {
        let mut residency = TileResidency::default();
        let targets = CubeFace::ALL
            .into_iter()
            .flat_map(|face| {
                (0..16).flat_map(move |y| {
                    (0..16).map(move |x| TileAddress::new(face, 4, x, y).unwrap())
                })
            })
            .collect::<Vec<_>>();
        let mut maximum_generated = 0;
        for _ in 0..100 {
            let update = residency.update(&targets);
            maximum_generated = maximum_generated.max(update.generated.len());
            assert!(update.generated.len() <= NEW_TERRAIN_TILES_PER_FRAME);
            assert!(residency.resident_count() <= MAX_RESIDENT_TILES);
            complete(&mut residency);
        }
        assert_eq!(maximum_generated, NEW_TERRAIN_TILES_PER_FRAME);
        assert_eq!(residency.resident_count(), MAX_RESIDENT_TILES);
    }

    #[test]
    fn resident_tiles_reuse_slots_and_eviction_is_deterministic() {
        let mut reuse = TileResidency::default();
        let address = root().child(TileQuadrant::UpperRight).unwrap();
        let initial = reuse.update(&[address]);
        assert!(!initial.generated.is_empty());
        complete(&mut reuse);
        assert!(reuse.update(&[address]).generated.is_empty());

        let mut first = TileResidency::default();
        let mut second = TileResidency::default();
        let fill = (0..MAX_RESIDENT_TILES)
            .map(|index| {
                let face = [CubeFace::PositiveZ, CubeFace::NegativeZ][index / 256];
                let face_index = index % 256;
                TileAddress::new(face, 4, (face_index % 16) as u32, (face_index / 16) as u32)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for (index, &address) in fill.iter().enumerate() {
            let slot = Slot {
                address,
                last_used: index as u64,
                ready: true,
            };
            first.slots[index] = Some(slot);
            second.slots[index] = Some(slot);
        }
        let replacement = TileAddress::new(CubeFace::NegativeY, 4, 15, 15).unwrap();
        let first_update = first.update(&[replacement]);
        let second_update = second.update(&[replacement]);
        assert_eq!(first_update.generated, second_update.generated);
        assert_eq!(
            first_update
                .generated
                .iter()
                .map(|generation| generation.slot)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4]
        );
    }

    #[test]
    fn a_fresh_residency_regenerates_the_same_world_coverage() {
        let targets = root().children().unwrap();
        let mut before = TileResidency::default();
        let mut after = TileResidency::default();
        assert_eq!(
            before.update(&targets).generated,
            after.update(&targets).generated
        );
    }
}
