use crate::StageInputError;
use procgen_core::{
    RandomStream,
    random_streams::{FIRST_MAJOR_PLATE_SEED, PLATE_GROWTH_COST},
};
use procgen_sphere_mesh::SphereMesh;
use std::{cmp::Reverse, collections::BinaryHeap, fmt};

const UNASSIGNED_PLATE: usize = usize::MAX;
const BASE_GROWTH_COST: u64 = 100;
pub const MAX_GROWTH_ROUGHNESS: u32 = BASE_GROWTH_COST as u32 - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlatePartitionConfig {
    pub major_plate_count: usize,
    pub minor_plate_count: usize,
    /// Expected major-only growth rounds at the baseline traversal cost.
    /// Roughness means this is not an exact graph-hop count.
    pub major_head_start_rounds: usize,
    /// Maximum percentage that an edge's deterministic traversal cost varies
    /// above or below the baseline. Must not exceed `MAX_GROWTH_ROUGHNESS`.
    pub growth_roughness: u32,
    pub seed: u64,
}

impl PlatePartitionConfig {
    pub const fn new(major_plate_count: usize, minor_plate_count: usize) -> Self {
        Self {
            major_plate_count,
            minor_plate_count,
            major_head_start_rounds: 0,
            growth_roughness: 0,
            seed: 0,
        }
    }

    pub const fn plate_count(self) -> usize {
        self.major_plate_count
            .saturating_add(self.minor_plate_count)
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
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatePartitionError {
    NoMajorPlates,
    TooManyPlates,
    InsufficientUnclaimedCells,
    InvalidGrowthRoughness,
}

impl fmt::Display for PlatePartitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoMajorPlates => formatter.write_str("at least one major plate is required"),
            Self::TooManyPlates => {
                formatter.write_str("plate count cannot exceed the mesh cell count")
            }
            Self::InsufficientUnclaimedCells => {
                formatter.write_str("too few unassigned cells remain to seed the requested plates")
            }
            Self::InvalidGrowthRoughness => write!(
                formatter,
                "plate growth roughness cannot exceed {MAX_GROWTH_ROUGHNESS}%"
            ),
        }
    }
}

impl std::error::Error for PlatePartitionError {}

/// Partitions a sphere mesh using major-plate head-start growth followed by
/// minor-plate seeding and a shared deterministic weighted flood fill.
pub fn partition_plates(
    mesh: &SphereMesh,
    config: PlatePartitionConfig,
) -> Result<PlatePartition, PlatePartitionError> {
    if config.major_plate_count == 0 {
        return Err(PlatePartitionError::NoMajorPlates);
    }
    if config.plate_count() > mesh.cell_count() {
        return Err(PlatePartitionError::TooManyPlates);
    }
    if config.growth_roughness > MAX_GROWTH_ROUGHNESS {
        return Err(PlatePartitionError::InvalidGrowthRoughness);
    }

    let first_seed = (RandomStream::new(config.seed, FIRST_MAJOR_PLATE_SEED).sample_u64(0, 0)
        % mesh.cell_count() as u64) as usize;
    let mut growth = PlateGrowth::new(mesh, config.seed, config.growth_roughness);
    growth.seed(first_seed);
    growth.seed_farthest(config.major_plate_count - 1)?;
    let head_start_cost = (config.major_head_start_rounds as u64).saturating_mul(BASE_GROWTH_COST);
    growth.grow_for(head_start_cost);
    growth.seed_farthest(config.minor_plate_count)?;
    growth.grow_for(u64::MAX);
    Ok(PlatePartition {
        plate_count: growth.plate_seeds.len(),
        cell_plates: growth.cell_plates,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Arrival {
    cost: u64,
    sequence: u64,
    cell: usize,
    plate: usize,
}

struct PlateGrowth<'mesh> {
    mesh: &'mesh SphereMesh,
    cell_plates: Vec<usize>,
    plate_seeds: Vec<usize>,
    seed_distance: Vec<f32>,
    best_arrivals: Vec<u64>,
    arrivals: BinaryHeap<Reverse<Arrival>>,
    next_sequence: u64,
    current_time: u64,
    growth_roughness: u64,
    growth_costs: RandomStream,
}

impl<'mesh> PlateGrowth<'mesh> {
    fn new(mesh: &'mesh SphereMesh, seed: u64, growth_roughness: u32) -> Self {
        Self {
            mesh,
            cell_plates: vec![UNASSIGNED_PLATE; mesh.cell_count()],
            plate_seeds: Vec::new(),
            seed_distance: vec![f32::MAX; mesh.cell_count()],
            best_arrivals: vec![u64::MAX; mesh.cell_count()],
            arrivals: BinaryHeap::new(),
            next_sequence: 0,
            current_time: 0,
            growth_roughness: u64::from(growth_roughness),
            growth_costs: RandomStream::new(seed, PLATE_GROWTH_COST),
        }
    }

    fn seed(&mut self, cell: usize) {
        let plate = self.plate_seeds.len();
        self.plate_seeds.push(cell);

        let seed_position = self.mesh.cell_centers[cell];
        for (candidate, distance) in self.seed_distance.iter_mut().enumerate() {
            *distance =
                distance.min(self.mesh.cell_centers[candidate].distance_squared(seed_position));
        }

        self.settle(cell, plate, self.current_time);
    }

    fn seed_farthest(&mut self, count: usize) -> Result<(), PlatePartitionError> {
        for _ in 0..count {
            let cell = self
                .seed_distance
                .iter()
                .enumerate()
                .filter(|(cell, _)| self.cell_plates[*cell] == UNASSIGNED_PLATE)
                .max_by(|(left_cell, left), (right_cell, right)| {
                    left.total_cmp(right)
                        .then_with(|| right_cell.cmp(left_cell))
                })
                .map(|(cell, _)| cell)
                .ok_or(PlatePartitionError::InsufficientUnclaimedCells)?;
            self.seed(cell);
        }
        Ok(())
    }

    fn grow_for(&mut self, elapsed_cost: u64) {
        self.current_time = self.current_time.saturating_add(elapsed_cost);
        while self
            .arrivals
            .peek()
            .is_some_and(|entry| entry.0.cost <= self.current_time)
        {
            let Reverse(arrival) = self.arrivals.pop().unwrap();
            if self.cell_plates[arrival.cell] != UNASSIGNED_PLATE {
                continue;
            }
            self.settle(arrival.cell, arrival.plate, arrival.cost);
        }
    }

    fn settle(&mut self, cell: usize, plate: usize, cost: u64) {
        self.cell_plates[cell] = plate;
        for corner in self.mesh.cell_corners(cell) {
            if self.cell_plates[corner.neighbor] != UNASSIGNED_PLATE {
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
    use std::collections::VecDeque;

    #[test]
    fn rejects_invalid_plate_counts() {
        let mesh = mesh(32);
        assert_eq!(
            partition_plates(&mesh, PlatePartitionConfig::new(0, 1)),
            Err(PlatePartitionError::NoMajorPlates)
        );
        assert_eq!(
            partition_plates(&mesh, PlatePartitionConfig::new(32, 1)),
            Err(PlatePartitionError::TooManyPlates)
        );
        assert_eq!(
            partition_plates(
                &mesh,
                PlatePartitionConfig {
                    growth_roughness: MAX_GROWTH_ROUGHNESS + 1,
                    ..PlatePartitionConfig::new(4, 4)
                }
            ),
            Err(PlatePartitionError::InvalidGrowthRoughness)
        );
    }

    #[test]
    fn partition_is_deterministic_and_seeded() {
        let mesh = mesh(512);
        let first = partition_plates(&mesh, reference_partition_config()).unwrap();
        assert_eq!(
            first,
            partition_plates(&mesh, reference_partition_config()).unwrap()
        );

        let changed = partition_plates(
            &mesh,
            PlatePartitionConfig {
                seed: 8,
                ..reference_partition_config()
            },
        )
        .unwrap();
        assert_ne!(first, changed);

        let rough_config = PlatePartitionConfig {
            growth_roughness: 35,
            ..reference_partition_config()
        };
        let rough = partition_plates(&mesh, rough_config).unwrap();
        assert_eq!(rough, partition_plates(&mesh, rough_config).unwrap());
        assert_ne!(first, rough);
    }

    #[test]
    fn reference_partition_has_stable_fingerprint() {
        let mesh = mesh(512);
        let partition = partition_plates(&mesh, reference_partition_config()).unwrap();
        let fingerprint = fingerprint(partition.cell_plates.iter().map(|&value| value as u64));

        assert_eq!(fingerprint, 2_459_160_733_919_900_345);
    }

    #[test]
    fn every_plate_is_nonempty_and_connected() {
        let mesh = mesh(512);
        let partition = partition_plates(
            &mesh,
            PlatePartitionConfig {
                growth_roughness: 35,
                ..reference_partition_config()
            },
        )
        .unwrap();

        assert!(
            partition
                .cell_plates
                .iter()
                .all(|&plate| plate < partition.plate_count)
        );
        for plate in 0..partition.plate_count {
            let start = partition
                .cell_plates
                .iter()
                .position(|&cell_plate| cell_plate == plate)
                .expect("every plate must own at least one cell");
            let expected = partition
                .cell_plates
                .iter()
                .filter(|&&cell_plate| cell_plate == plate)
                .count();
            let mut visited = vec![false; mesh.cell_count()];
            visited[start] = true;
            let mut queue = VecDeque::from([start]);
            let mut actual = 0;
            while let Some(cell) = queue.pop_front() {
                actual += 1;
                for corner in mesh.cell_corners(cell) {
                    if !visited[corner.neighbor] && partition.cell_plates[corner.neighbor] == plate
                    {
                        visited[corner.neighbor] = true;
                        queue.push_back(corner.neighbor);
                    }
                }
            }
            assert_eq!(actual, expected, "plate {plate} is disconnected");
        }
    }

    #[test]
    fn excessive_head_start_reports_starved_minor_plates() {
        let mesh = mesh(64);
        let result = partition_plates(
            &mesh,
            PlatePartitionConfig {
                major_plate_count: 2,
                minor_plate_count: 8,
                major_head_start_rounds: 100,
                growth_roughness: 35,
                seed: 7,
            },
        );
        assert_eq!(result, Err(PlatePartitionError::InsufficientUnclaimedCells));
    }
}
