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
    let mut growth = PlateGrowth::new(
        mesh,
        GrowthBounds::WithinFaces(&faces.cell_faces),
        GrowthCosts {
            roughness: config.growth_roughness,
            stream: RandomStream::new(config.seed, PLATE_GROWTH_COST),
        },
    );
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

/// Returns `count` seeds among `cells`: `first`, then repeatedly the cell
/// farthest from every seed already chosen. Ties go to the lower cell index.
///
/// The partition passes one face's cells, so a face's minor plates are spread
/// across it; [`crate::crust`] passes every cell, so continental nuclei are
/// spread across the sphere.
pub(crate) fn farthest_first(
    mesh: &SphereMesh,
    cells: &[usize],
    first: usize,
    count: usize,
) -> Vec<usize> {
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
            .expect("at least one cell to choose from");
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

/// Which steps growth may take. The partition confines every plate to the
/// crack face its seed sits in, so a plate never leaves it; continental
/// nuclei grow over the whole sphere, because a continent's edge has nothing
/// to do with a plate boundary.
pub(crate) enum GrowthBounds<'a> {
    WithinFaces(&'a [usize]),
    WholeSphere,
}

impl GrowthBounds<'_> {
    fn passable(&self, cell: usize, neighbor: usize) -> bool {
        match self {
            Self::WithinFaces(cell_faces) => cell_faces[cell] == cell_faces[neighbor],
            Self::WholeSphere => true,
        }
    }
}

/// The per-edge cost field growth crosses: how far a cost may vary from the
/// baseline, and the stream that variation is hashed from.
///
/// The stream is the caller's rather than derived from a seed here, so two
/// growths over the same mesh cross independent cost fields even when their
/// seeds coincide.
pub(crate) struct GrowthCosts {
    pub(crate) roughness: u32,
    pub(crate) stream: RandomStream,
}

/// Multi-source shortest-arrival search over per-edge integer costs, bounded
/// by [`GrowthBounds`].
///
/// The numbered regions it grows are the partition's plates; [`crate::crust`]
/// grows continental nuclei with the same engine and reads only which cells
/// it reached.
pub(crate) struct PlateGrowth<'mesh> {
    mesh: &'mesh SphereMesh,
    bounds: GrowthBounds<'mesh>,
    pub(crate) cell_plates: Vec<usize>,
    best_arrivals: Vec<u64>,
    arrivals: BinaryHeap<Reverse<Arrival>>,
    next_sequence: u64,
    costs: GrowthCosts,
}

impl<'mesh> PlateGrowth<'mesh> {
    pub(crate) fn new(
        mesh: &'mesh SphereMesh,
        bounds: GrowthBounds<'mesh>,
        costs: GrowthCosts,
    ) -> Self {
        Self {
            mesh,
            bounds,
            cell_plates: vec![UNASSIGNED_PLATE; mesh.cell_count()],
            best_arrivals: vec![u64::MAX; mesh.cell_count()],
            arrivals: BinaryHeap::new(),
            next_sequence: 0,
            costs,
        }
    }

    /// Plants a region's first cell, which arrives at zero cost.
    pub(crate) fn seed(&mut self, cell: usize, region: usize) {
        self.settle(cell, region, 0);
    }

    /// Settles every cell the seeds can reach.
    pub(crate) fn grow(&mut self) {
        while self.settle_next().is_some() {}
    }

    /// Settles cells until their total area reaches `target_area`, and returns
    /// the area settled.
    ///
    /// The budget is checked before each settle, so growth ends at the first
    /// settled cell that carries the total past the target and the achieved
    /// area exceeds it by at most one cell's. A target the seeds already meet
    /// grows nothing, and one beyond the reachable area settles everything.
    pub(crate) fn grow_to_area(&mut self, target_area: f64) -> f64 {
        let mut area = self.settled_area();
        while area < target_area {
            let Some(cell) = self.settle_next() else {
                break;
            };
            area += f64::from(self.mesh.cell_areas[cell]);
        }
        area
    }

    /// Whether growth has reached `cell`.
    pub(crate) fn reached(&self, cell: usize) -> bool {
        self.cell_plates[cell] != UNASSIGNED_PLATE
    }

    fn settled_area(&self) -> f64 {
        self.cell_plates
            .iter()
            .zip(&self.mesh.cell_areas)
            .filter(|&(&plate, _)| plate != UNASSIGNED_PLATE)
            .map(|(_, &area)| f64::from(area))
            .sum()
    }

    /// Settles the cheapest arrival still outstanding and returns its cell, or
    /// `None` once the frontier holds nothing new.
    fn settle_next(&mut self) -> Option<usize> {
        while let Some(Reverse(arrival)) = self.arrivals.pop() {
            if self.cell_plates[arrival.cell] != UNASSIGNED_PLATE {
                continue;
            }
            self.settle(arrival.cell, arrival.plate, arrival.cost);
            return Some(arrival.cell);
        }
        None
    }

    fn settle(&mut self, cell: usize, plate: usize, cost: u64) {
        self.cell_plates[cell] = plate;
        for corner in self.mesh.cell_corners(cell) {
            if !self.bounds.passable(cell, corner.neighbor)
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
        let roughness = u64::from(self.costs.roughness);
        let offset = self.costs.stream.sample_u64(edge as u64, 0) % (roughness * 2 + 1);
        BASE_GROWTH_COST - roughness + offset
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
