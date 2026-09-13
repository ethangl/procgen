//! Stable bounded candidates; distant searches and local instances share results.
use crate::{TerrainQueries, WALKABLE_COSINE, noise::normalized_noise};
use procgen_core::{Vec3, hash_u32};
use procgen_cubesphere::{CubeFace, FaceCoordinates, face_to_direction};
use procgen_noise::fold_seed_u64_to_u32;

pub const POPULATION_REACH: f32 = 1.8;
pub const LANDMARK_SEARCH_REACH: f32 = 3.0;
const ROCK_GRID: u32 = 8;
const LANDMARK_GRID: u32 = 2;
pub const MAX_POPULATION: usize =
    6 * (ROCK_GRID * ROCK_GRID + LANDMARK_GRID * LANDMARK_GRID) as usize;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlacementKind {
    Rock,
    Landmark,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PlacementId {
    pub kind: PlacementKind,
    pub face: CubeFace,
    pub x: u32,
    pub y: u32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub id: PlacementId,
    pub position: Vec3,
    pub normal: Vec3,
    pub scale: f32,
}

pub struct Population {
    placements: Vec<Placement>,
}
impl Population {
    /// CPU-only catalog for the fixed six-face experiment. At most 408 candidates.
    pub fn prepare(seed: u64, terrain: &TerrainQueries) -> Self {
        let root = fold_seed_u64_to_u32(seed);
        let mut placements = Vec::with_capacity(MAX_POPULATION);
        for kind in [PlacementKind::Rock, PlacementKind::Landmark] {
            let key = hash_u32(
                root,
                match kind {
                    PlacementKind::Rock => 0x524f434b,
                    PlacementKind::Landmark => 0x4c414e44,
                },
                0,
                0,
            );
            let n = match kind {
                PlacementKind::Rock => ROCK_GRID,
                PlacementKind::Landmark => LANDMARK_GRID,
            };
            for face in CubeFace::ALL {
                for y in 0..n {
                    for x in 0..n {
                        let id = PlacementId { kind, face, x, y };
                        let h = hash_u32(key, face.index() as u32, x, y);
                        let unit = |tag| (hash_u32(h, tag, 0, 0) & 0xffff) as f32 / 65536.0;
                        // Jitter stays within the middle half of the candidate cell.
                        let u = 2.0 * (x as f32 + 0.25 + unit(0) * 0.5) / n as f32 - 1.0;
                        let v = 2.0 * (y as f32 + 0.25 + unit(1) * 0.5) / n as f32 - 1.0;
                        let direction = face_to_direction(FaceCoordinates { face, u, v })
                            .expect("candidate inside face");
                        if kind == PlacementKind::Rock
                            && normalized_noise(key, direction * 3.0).value < 0.0
                        {
                            continue;
                        }
                        let Some(hit) = terrain.exterior(direction) else {
                            continue;
                        };
                        if hit.normal.dot(direction) < WALKABLE_COSINE {
                            continue;
                        }
                        placements.push(Placement {
                            id,
                            position: hit.position,
                            normal: hit.normal,
                            scale: match kind {
                                PlacementKind::Rock => 0.018 + unit(2) * 0.022,
                                PlacementKind::Landmark => 0.13,
                            },
                        });
                    }
                }
            }
        }
        Self { placements }
    }
    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }
    pub fn nearby(&self, position: Vec3) -> impl Iterator<Item = &Placement> {
        self.placements
            .iter()
            .filter(move |p| p.position.distance_squared(position) <= POPULATION_REACH.powi(2))
    }
    /// Bounded chord-distance search, not a global nearest-landmark promise.
    pub fn landmark(&self, position: Vec3) -> Option<&Placement> {
        self.placements
            .iter()
            .filter(|p| {
                p.id.kind == PlacementKind::Landmark
                    && p.position.distance_squared(position) <= LANDMARK_SEARCH_REACH.powi(2)
            })
            .min_by(|a, b| {
                a.position
                    .distance_squared(position)
                    .total_cmp(&b.position.distance_squared(position))
                    .then(a.id.cmp(&b.id))
            })
    }
}
