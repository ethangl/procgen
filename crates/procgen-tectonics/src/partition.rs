use crate::random_streams::FIRST_MAJOR_SEED;
use procgen_core::RandomStream;
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

const UNASSIGNED_PLATE: usize = usize::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlatePartitionConfig {
    pub major_plate_count: usize,
    pub minor_plate_count: usize,
    pub major_head_start_rounds: usize,
    pub seed: u64,
}

impl PlatePartitionConfig {
    pub const fn new(major_plate_count: usize, minor_plate_count: usize) -> Self {
        Self {
            major_plate_count,
            minor_plate_count,
            major_head_start_rounds: 0,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatePartitionError {
    NoMajorPlates,
    TooManyPlates,
    InsufficientUnclaimedCells,
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
        }
    }
}

impl std::error::Error for PlatePartitionError {}

/// Partitions a sphere mesh using major-plate head-start growth followed by
/// minor-plate seeding and a shared deterministic flood fill.
pub fn partition_plates(
    mesh: &SphereMesh,
    config: PlatePartitionConfig,
) -> Result<PlatePartition, PlatePartitionError> {
    let growth = grow_plates(mesh, config)?;
    Ok(PlatePartition {
        plate_count: growth.plate_seeds.len(),
        cell_plates: growth.cell_plates,
    })
}

fn grow_plates(
    mesh: &SphereMesh,
    config: PlatePartitionConfig,
) -> Result<PlateGrowth<'_>, PlatePartitionError> {
    if config.major_plate_count == 0 {
        return Err(PlatePartitionError::NoMajorPlates);
    }
    if config.plate_count() > mesh.cell_count() {
        return Err(PlatePartitionError::TooManyPlates);
    }

    let first_seed = (RandomStream::new(config.seed, FIRST_MAJOR_SEED).sample_u64(0, 0)
        % mesh.cell_count() as u64) as usize;
    let mut growth = PlateGrowth::new(mesh);
    growth.seed(first_seed);
    growth.seed_farthest(config.major_plate_count - 1)?;
    growth.grow(config.major_head_start_rounds);
    growth.seed_farthest(config.minor_plate_count)?;
    growth.grow(usize::MAX);
    Ok(growth)
}

struct PlateGrowth<'mesh> {
    mesh: &'mesh SphereMesh,
    cell_plates: Vec<usize>,
    plate_seeds: Vec<usize>,
    seed_distance: Vec<f32>,
    frontier: Vec<usize>,
}

impl<'mesh> PlateGrowth<'mesh> {
    fn new(mesh: &'mesh SphereMesh) -> Self {
        Self {
            mesh,
            cell_plates: vec![UNASSIGNED_PLATE; mesh.cell_count()],
            plate_seeds: Vec::new(),
            seed_distance: vec![f32::MAX; mesh.cell_count()],
            frontier: Vec::new(),
        }
    }

    fn seed(&mut self, cell: usize) {
        let plate = self.plate_seeds.len();
        self.cell_plates[cell] = plate;
        self.plate_seeds.push(cell);
        self.frontier.push(cell);

        let seed_position = self.mesh.cell_centers[cell];
        for (candidate, distance) in self.seed_distance.iter_mut().enumerate() {
            *distance =
                distance.min(self.mesh.cell_centers[candidate].distance_squared(seed_position));
        }
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

    fn grow(&mut self, max_rounds: usize) {
        let mut rounds = 0;
        while !self.frontier.is_empty() && rounds < max_rounds {
            let current_frontier = std::mem::take(&mut self.frontier);
            for cell in current_frontier {
                let plate = self.cell_plates[cell];
                for corner in self.mesh.cell_corners(cell) {
                    if self.cell_plates[corner.neighbor] == UNASSIGNED_PLATE {
                        self.cell_plates[corner.neighbor] = plate;
                        self.frontier.push(corner.neighbor);
                    }
                }
            }
            rounds += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{mesh, reference_partition_config};
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
    }

    #[test]
    fn reference_partition_has_stable_fingerprint() {
        let mesh = mesh(512);
        let growth = grow_plates(&mesh, reference_partition_config()).unwrap();
        let fingerprint = growth
            .cell_plates
            .iter()
            .chain(&growth.plate_seeds)
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, &value| {
                (hash ^ value as u64).wrapping_mul(0x0000_0100_0000_01b3)
            });

        assert_eq!(fingerprint, 3_406_652_772_411_950_386);
    }

    #[test]
    fn every_plate_is_nonempty_connected_and_owns_its_seed() {
        let mesh = mesh(512);
        let growth = grow_plates(&mesh, reference_partition_config()).unwrap();

        assert!(
            growth
                .cell_plates
                .iter()
                .all(|&plate| plate < growth.plate_seeds.len())
        );
        for (plate, &seed) in growth.plate_seeds.iter().enumerate() {
            assert_eq!(growth.cell_plates[seed], plate);

            let expected = growth
                .cell_plates
                .iter()
                .filter(|&&cell_plate| cell_plate == plate)
                .count();
            let mut visited = vec![false; mesh.cell_count()];
            visited[seed] = true;
            let mut queue = VecDeque::from([seed]);
            let mut actual = 0;
            while let Some(cell) = queue.pop_front() {
                actual += 1;
                for corner in mesh.cell_corners(cell) {
                    if !visited[corner.neighbor] && growth.cell_plates[corner.neighbor] == plate {
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
                seed: 7,
            },
        );
        assert_eq!(result, Err(PlatePartitionError::InsufficientUnclaimedCells));
    }
}
