//! Plate partitioning as a crack pattern whose faces growth subdivides.
//!
//! Stage one is the crack pattern in [`crate::cracks`]: arcs walk the mesh and
//! the cells between their walls become faces. Stage two seeds each face —
//! once for a face left whole, farthest-first for a face selected to split —
//! and runs one shortest-arrival growth over per-edge integer costs that never
//! crosses a face boundary. Independent per-edge costs alone produce convex
//! blobs, so the cracks are what give the partition its plate-like outlines.
//!
//! Plate ids follow seeding order, and the crack walk is libm-free, so the
//! whole partition is reproducible; [`crate::cracks`] records exactly what
//! that rests on.

use crate::{
    StageInputError,
    cracks::{CrackFaces, crack_faces},
};
use procgen_core::{
    RandomStream,
    random_streams::{PLATE_FACE_SUBDIVISION, PLATE_GROWTH_COST},
};
use procgen_sphere_mesh::SphereMesh;
use std::{cmp::Reverse, collections::BinaryHeap, fmt};

const UNASSIGNED_PLATE: usize = usize::MAX;
const BASE_GROWTH_COST: u64 = 100;
pub const MAX_GROWTH_ROUGHNESS: u32 = BASE_GROWTH_COST as u32 - 1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlatePartitionConfig {
    /// Crack arcs attempted. Arcs that start on an existing wall are skipped,
    /// so this bounds rather than fixes the number of primary boundaries.
    pub arc_count: usize,
    /// Heading change in radians per radian travelled. Zero walks great circles.
    pub curvature: f32,
    /// Fraction of faces that growth splits into minor plates.
    pub subdivided_fraction: f32,
    /// Target minor-plate area as a fraction of the sphere.
    pub piece_fraction: f32,
    /// Maximum percentage that an edge's deterministic traversal cost varies
    /// above or below the baseline. Must not exceed `MAX_GROWTH_ROUGHNESS`.
    pub growth_roughness: u32,
    pub seed: u64,
}

impl Default for PlatePartitionConfig {
    fn default() -> Self {
        Self {
            arc_count: 16,
            curvature: 8.0,
            subdivided_fraction: 0.8,
            piece_fraction: 1.0 / 120.0,
            growth_roughness: MAX_GROWTH_ROUGHNESS,
            seed: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatePartition {
    pub cell_plates: Vec<usize>,
    /// Number of stable plate identities addressable by `cell_plates`.
    pub plate_count: usize,
}

impl PlatePartition {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), StageInputError> {
        if self.cell_plates.len() != mesh.cell_count() {
            return Err(StageInputError::Cells);
        }
        if self
            .cell_plates
            .iter()
            .any(|&plate| plate >= self.plate_count)
        {
            return Err(StageInputError::PlateOwnership);
        }
        // Suturing empties the absorbed id, and evolution compacts the ids it
        // empties before it returns, so no plate identity a consumer can see
        // is a hole.
        let mut owned = vec![false; self.plate_count];
        for &plate in &self.cell_plates {
            owned[plate] = true;
        }
        if owned.iter().any(|&owned| !owned) {
            return Err(StageInputError::EmptyPlate);
        }
        Ok(())
    }

    /// Total surface area each plate currently owns.
    pub fn plate_areas(&self, mesh: &SphereMesh) -> Vec<f64> {
        let mut areas = vec![0.0; self.plate_count];
        for (&plate, &area) in self.cell_plates.iter().zip(&mesh.cell_areas) {
            areas[plate] += f64::from(area);
        }
        areas
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatePartitionError {
    NoArcs,
    InvalidCurvature,
    InvalidSubdividedFraction,
    InvalidPieceFraction,
    InvalidGrowthRoughness,
}

impl fmt::Display for PlatePartitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoArcs => formatter.write_str("at least one crack arc is required"),
            Self::InvalidCurvature => {
                formatter.write_str("crack curvature must be finite and nonnegative")
            }
            Self::InvalidSubdividedFraction => {
                formatter.write_str("subdivided fraction must lie in [0, 1]")
            }
            Self::InvalidPieceFraction => formatter.write_str("piece fraction must lie in (0, 1]"),
            Self::InvalidGrowthRoughness => write!(
                formatter,
                "plate growth roughness cannot exceed {MAX_GROWTH_ROUGHNESS}%"
            ),
        }
    }
}

impl std::error::Error for PlatePartitionError {}

/// Partitions a sphere mesh into plates: crack faces first, then one
/// shortest-arrival growth from the seeds those faces place.
pub fn partition_plates(
    mesh: &SphereMesh,
    config: PlatePartitionConfig,
) -> Result<PlatePartition, PlatePartitionError> {
    if config.arc_count == 0 {
        return Err(PlatePartitionError::NoArcs);
    }
    if !config.curvature.is_finite() || config.curvature < 0.0 {
        return Err(PlatePartitionError::InvalidCurvature);
    }
    if !(0.0..=1.0).contains(&config.subdivided_fraction) {
        return Err(PlatePartitionError::InvalidSubdividedFraction);
    }
    if !(config.piece_fraction > 0.0 && config.piece_fraction <= 1.0) {
        return Err(PlatePartitionError::InvalidPieceFraction);
    }
    if config.growth_roughness > MAX_GROWTH_ROUGHNESS {
        return Err(PlatePartitionError::InvalidGrowthRoughness);
    }

    let faces = crack_faces(mesh, config.seed, config.arc_count, config.curvature);
    let seeds = plate_seeds(mesh, &faces, config);
    let mut growth = PlateGrowth::new(mesh, &faces.cell_faces, config);
    for (plate, &cell) in seeds.iter().enumerate() {
        growth.seed(cell, plate);
    }
    growth.grow();
    Ok(PlatePartition {
        cell_plates: growth.cell_plates,
        plate_count: seeds.len(),
    })
}

/// Places one seed per face left whole and several inside each face the hash
/// selects for splitting, face by face, so plate ids come out sequential
/// without a remap.
fn plate_seeds(mesh: &SphereMesh, faces: &CrackFaces, config: PlatePartitionConfig) -> Vec<usize> {
    let mut face_cells = vec![Vec::new(); faces.face_count];
    for (cell, &face) in faces.cell_faces.iter().enumerate() {
        face_cells[face].push(cell);
    }
    let target_area = f64::from(config.piece_fraction) * mesh.total_area();
    let subdivision = RandomStream::new(config.seed, PLATE_FACE_SUBDIVISION);

    let mut seeds = Vec::with_capacity(faces.face_count);
    for (face, cells) in face_cells.iter().enumerate() {
        let item = face as u64;
        let first = cells[(subdivision.sample_u64(item, 1) % cells.len() as u64) as usize];
        if subdivision.unit_f32(item, 0) >= config.subdivided_fraction {
            seeds.push(first);
            continue;
        }
        let area: f64 = cells
            .iter()
            .map(|&cell| f64::from(mesh.cell_areas[cell]))
            .sum();
        // At most one piece per cell, so a single-cell face cannot split.
        let pieces = ((area / target_area).round() as usize)
            .max(2)
            .min(cells.len());
        seeds.extend(farthest_first(mesh, cells, first, pieces));
    }
    seeds
}

/// Returns `count` seeds within one face: `first`, then repeatedly the cell
/// farthest from every seed already chosen. Ties go to the lower cell index.
fn farthest_first(mesh: &SphereMesh, cells: &[usize], first: usize, count: usize) -> Vec<usize> {
    let mut seeds = vec![first];
    let mut chosen = first;
    // A chosen cell's own distance falls to zero, so it never wins again.
    let mut distances = vec![f32::MAX; cells.len()];
    while seeds.len() < count {
        let position = mesh.cell_centers[chosen];
        for (distance, &cell) in distances.iter_mut().zip(cells) {
            *distance = distance.min(mesh.cell_centers[cell].distance_squared(position));
        }
        let (index, _) = distances
            .iter()
            .enumerate()
            .max_by(|(left_index, left), (right_index, right)| {
                left.total_cmp(right)
                    .then_with(|| right_index.cmp(left_index))
            })
            .expect("a face holds at least one cell");
        chosen = cells[index];
        seeds.push(chosen);
    }
    seeds
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival {
    cost: u64,
    sequence: u64,
    cell: usize,
    plate: usize,
}

/// Multi-source shortest-arrival search over per-edge integer costs. A step
/// is passable only within one face, so a plate never leaves the face its
/// seed sits in.
struct PlateGrowth<'mesh> {
    mesh: &'mesh SphereMesh,
    cell_faces: &'mesh [usize],
    cell_plates: Vec<usize>,
    best_arrivals: Vec<u64>,
    arrivals: BinaryHeap<Reverse<Arrival>>,
    next_sequence: u64,
    growth_roughness: u64,
    growth_costs: RandomStream,
}

impl<'mesh> PlateGrowth<'mesh> {
    fn new(
        mesh: &'mesh SphereMesh,
        cell_faces: &'mesh [usize],
        config: PlatePartitionConfig,
    ) -> Self {
        Self {
            mesh,
            cell_faces,
            cell_plates: vec![UNASSIGNED_PLATE; mesh.cell_count()],
            best_arrivals: vec![u64::MAX; mesh.cell_count()],
            arrivals: BinaryHeap::new(),
            next_sequence: 0,
            growth_roughness: u64::from(config.growth_roughness),
            growth_costs: RandomStream::new(config.seed, PLATE_GROWTH_COST),
        }
    }

    /// Plants a plate's first cell, which arrives at zero cost.
    fn seed(&mut self, cell: usize, plate: usize) {
        self.settle(cell, plate, 0);
    }

    fn grow(&mut self) {
        while let Some(Reverse(arrival)) = self.arrivals.pop() {
            if self.cell_plates[arrival.cell] != UNASSIGNED_PLATE {
                continue;
            }
            self.settle(arrival.cell, arrival.plate, arrival.cost);
        }
    }

    fn settle(&mut self, cell: usize, plate: usize, cost: u64) {
        self.cell_plates[cell] = plate;
        let face = self.cell_faces[cell];
        for corner in self.mesh.cell_corners(cell) {
            if self.cell_faces[corner.neighbor] != face
                || self.cell_plates[corner.neighbor] != UNASSIGNED_PLATE
            {
                continue;
            }
            let candidate_cost = cost.saturating_add(self.edge_cost(corner.edge));
            if candidate_cost < self.best_arrivals[corner.neighbor] {
                self.best_arrivals[corner.neighbor] = candidate_cost;
                self.next_sequence = self.next_sequence.wrapping_add(1);
                self.arrivals.push(Reverse(Arrival {
                    cost: candidate_cost,
                    sequence: self.next_sequence,
                    cell: corner.neighbor,
                    plate,
                }));
            }
        }
    }

    fn edge_cost(&self, edge: usize) -> u64 {
        let offset = self.growth_costs.sample_u64(edge as u64, 0) % (self.growth_roughness * 2 + 1);
        BASE_GROWTH_COST - self.growth_roughness + offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{fingerprint, mesh, reference_partition_config};
    use procgen_sphere_mesh::connected_components;

    /// A denser crack pattern than the shared reference, so the invariant
    /// tests cover many faces.
    fn dense_config() -> PlatePartitionConfig {
        PlatePartitionConfig {
            arc_count: 24,
            piece_fraction: 32.0 / 1_024.0,
            seed: 7,
            ..PlatePartitionConfig::default()
        }
    }

    #[test]
    fn rejects_invalid_configurations() {
        let mesh = mesh(64);
        let valid = reference_partition_config();
        for (config, expected) in [
            (
                PlatePartitionConfig {
                    arc_count: 0,
                    ..valid
                },
                PlatePartitionError::NoArcs,
            ),
            (
                PlatePartitionConfig {
                    curvature: -1.0,
                    ..valid
                },
                PlatePartitionError::InvalidCurvature,
            ),
            (
                PlatePartitionConfig {
                    curvature: f32::NAN,
                    ..valid
                },
                PlatePartitionError::InvalidCurvature,
            ),
            (
                PlatePartitionConfig {
                    subdivided_fraction: 1.5,
                    ..valid
                },
                PlatePartitionError::InvalidSubdividedFraction,
            ),
            (
                PlatePartitionConfig {
                    piece_fraction: 0.0,
                    ..valid
                },
                PlatePartitionError::InvalidPieceFraction,
            ),
            (
                PlatePartitionConfig {
                    growth_roughness: MAX_GROWTH_ROUGHNESS + 1,
                    ..valid
                },
                PlatePartitionError::InvalidGrowthRoughness,
            ),
        ] {
            assert_eq!(partition_plates(&mesh, config), Err(expected));
        }
    }

    #[test]
    fn every_cell_belongs_to_one_connected_plate() {
        let mesh = mesh(1_024);
        let partition = partition_plates(&mesh, dense_config()).unwrap();

        assert_eq!(partition.cell_plates.len(), mesh.cell_count());
        assert!(
            partition
                .cell_plates
                .iter()
                .all(|&plate| plate < partition.plate_count)
        );
        for plate in 0..partition.plate_count {
            assert_eq!(
                connected_components(
                    &mesh,
                    |cell| partition.cell_plates[cell] == plate,
                    |_, _| true
                )
                .len(),
                1,
                "plate {plate} is empty or disconnected"
            );
        }
    }

    #[test]
    fn subdivision_splits_only_the_selected_faces() {
        let mesh = mesh(1_024);
        let whole = partition_plates(
            &mesh,
            PlatePartitionConfig {
                subdivided_fraction: 0.0,
                ..dense_config()
            },
        )
        .unwrap();
        let split = partition_plates(&mesh, dense_config()).unwrap();
        let every_face = partition_plates(
            &mesh,
            PlatePartitionConfig {
                subdivided_fraction: 1.0,
                ..dense_config()
            },
        )
        .unwrap();

        assert!(whole.plate_count < split.plate_count);
        assert!(split.plate_count < every_face.plate_count);
        // Growth never crosses a face, so every split plate stays inside the
        // one plate the same mesh has when no face splits.
        let mut plate_faces = vec![None; split.plate_count];
        for (cell, &plate) in split.cell_plates.iter().enumerate() {
            let face = whole.cell_plates[cell];
            assert_eq!(
                *plate_faces[plate].get_or_insert(face),
                face,
                "plate {plate} spans two faces"
            );
        }
    }

    #[test]
    fn partition_is_deterministic_and_seeded() {
        let mesh = mesh(512);
        let first = partition_plates(&mesh, reference_partition_config()).unwrap();

        assert_eq!(
            first,
            partition_plates(&mesh, reference_partition_config()).unwrap()
        );
        assert_ne!(
            first,
            partition_plates(
                &mesh,
                PlatePartitionConfig {
                    seed: 8,
                    ..reference_partition_config()
                }
            )
            .unwrap()
        );
    }

    #[test]
    fn reference_partition_has_stable_fingerprint() {
        let mesh = mesh(512);
        let partition = partition_plates(&mesh, reference_partition_config()).unwrap();
        let fingerprint = fingerprint(partition.cell_plates.iter().map(|&value| value as u64));

        // The crack walk is libm-free, so this value is expected to match on
        // both the macOS and the Windows development machine.
        assert_eq!(fingerprint, 1_312_040_099_017_365_644);
    }
}
