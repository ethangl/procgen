use std::collections::HashSet;

use bevy::prelude::Resource;
use procgen_cubesphere::{CubeFace, TileAddress};

use super::{MAX_RESIDENT_TILES, NEW_TERRAIN_TILES_PER_FRAME};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ResidentTile {
    pub address: TileAddress,
    pub slot: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TileGeneration {
    pub address: TileAddress,
    pub slot: u32,
}

#[derive(Default)]
pub(super) struct ResidencyUpdate {
    pub generated: Vec<TileGeneration>,
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
                generated.push(TileGeneration {
                    address,
                    slot: slot as u32,
                });
            }
        }

        let mut displayed = Vec::new();
        for face in CubeFace::ALL {
            let root = TileAddress::new(face, 0, 0, 0).unwrap();
            self.resolve(root, targets, &mut displayed);
        }
        displayed.sort_by_key(|tile| address_key(tile.address));
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

    pub(super) fn complete(&mut self, generated: &[TileGeneration]) {
        for generation in generated {
            let slot = self.slots[generation.slot as usize]
                .as_mut()
                .expect("pending terrain slot must remain allocated");
            assert_eq!(slot.address, generation.address);
            slot.ready = true;
        }
    }

    fn resolve(
        &self,
        node: TileAddress,
        targets: &[TileAddress],
        output: &mut Vec<ResidentTile>,
    ) -> bool {
        let exact = targets.contains(&node);
        let has_descendants = targets
            .iter()
            .any(|target| is_ancestor(node, *target) && *target != node);
        if !exact && !has_descendants {
            return true;
        }
        if exact {
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
                .filter(|tile| is_ancestor(node, tile.address))
                .collect::<Vec<_>>();
            if !descendants.is_empty() {
                output.extend(descendants);
                return true;
            }
            return false;
        }

        let checkpoint = output.len();
        let mut complete = true;
        for child in children(node) {
            if targets.iter().any(|target| is_ancestor(child, *target)) {
                let child_checkpoint = output.len();
                if !self.resolve(child, targets, output) {
                    output.truncate(child_checkpoint);
                    if let Some(slot) = self.find_ready(child) {
                        output.push(ResidentTile {
                            address: child,
                            slot: slot as u32,
                        });
                    } else {
                        complete = false;
                    }
                }
            }
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
            .min_by_key(|(index, slot)| (slot.last_used, address_key(slot.address), *index))
            .map(|(index, _)| index)
    }
}

pub(super) fn candidate_path(targets: &[TileAddress]) -> Vec<TileAddress> {
    let mut candidates = targets
        .iter()
        .flat_map(|target| std::iter::successors(Some(*target), |tile| tile.parent()))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|address| (address.level(), address_key(*address)));
    candidates.dedup();
    candidates
}

fn children(address: TileAddress) -> [TileAddress; 4] {
    use procgen_cubesphere::TileQuadrant::*;
    [LowerLeft, LowerRight, UpperLeft, UpperRight].map(|quadrant| address.child(quadrant).unwrap())
}

fn is_ancestor(ancestor: TileAddress, mut tile: TileAddress) -> bool {
    while tile.level() > ancestor.level() {
        tile = tile.parent().unwrap();
    }
    tile == ancestor
}

fn address_key(address: TileAddress) -> (usize, u8, u32, u32) {
    (
        address.face().index(),
        address.level(),
        address.y(),
        address.x(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_cubesphere::TileQuadrant;

    fn root() -> TileAddress {
        TileAddress::new(CubeFace::PositiveX, 0, 0, 0).unwrap()
    }

    fn complete(residency: &mut TileResidency, update: &ResidencyUpdate) {
        residency.complete(&update.generated);
    }

    #[test]
    fn split_handoff_keeps_parent_until_every_child_is_ready() {
        let mut residency = TileResidency::default();
        let root = root();
        let root_generation = residency.update(&[root]);
        assert!(root_generation.displayed.is_empty());
        complete(&mut residency, &root_generation);
        assert_eq!(
            residency.update(&[root]).displayed,
            vec![ResidentTile {
                address: root,
                slot: 0
            }]
        );
        let children = children(root);
        let update = residency.update(&children);
        assert_eq!(update.generated.len(), 4);
        assert_eq!(update.displayed[0].address, root);
        complete(&mut residency, &update);
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
        let root = TileAddress::new(CubeFace::NegativeZ, 0, 0, 0).unwrap();
        let update = residency.update(&[root]);
        complete(&mut residency, &update);
        residency.update(&[root]);
        let grandchildren = children(root)
            .into_iter()
            .flat_map(children)
            .collect::<Vec<_>>();
        let mut targets = CubeFace::ALL[..5]
            .iter()
            .map(|&face| TileAddress::new(face, 0, 0, 0).unwrap())
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
        let children = children(root);
        let update = residency.update(&children);
        complete(&mut residency, &update);
        residency.update(&children);
        let slot = residency.find(root).unwrap();
        residency.slots[slot] = None;
        let update = residency.update(&[root]);
        assert_eq!(
            update.generated,
            vec![TileGeneration {
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
        complete(&mut residency, &update);
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
            complete(&mut residency, &update);
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
        complete(&mut reuse, &initial);
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
        let targets = children(root());
        let mut before = TileResidency::default();
        let mut after = TileResidency::default();
        assert_eq!(
            before.update(&targets).generated,
            after.update(&targets).generated
        );
    }
}
