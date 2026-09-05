use crate::{PlatePartition, random_streams::CRUST_PLATE_ORDER};
use procgen_core::RandomStream;
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CrustClass {
    Oceanic,
    Continental,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrustClassificationConfig {
    /// Desired fraction of the sphere's surface covered by oceanic crust.
    pub target_ocean_fraction: f32,
    pub seed: u64,
}

impl CrustClassificationConfig {
    pub const fn new(seed: u64) -> Self {
        Self {
            target_ocean_fraction: 0.7,
            seed,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CrustClassification {
    pub plate_classes: Vec<CrustClass>,
}

impl CrustClassification {
    /// Derives a cell's crust class from its current plate ownership.
    pub fn cell_class(&self, partition: &PlatePartition, cell: usize) -> CrustClass {
        self.plate_classes[partition.cell_plates[cell]]
    }

    pub fn plate_count(&self, class: CrustClass) -> usize {
        self.plate_classes
            .iter()
            .filter(|&&candidate| candidate == class)
            .count()
    }

    /// Derives the current area-weighted ocean fraction from cell ownership.
    pub fn ocean_fraction(&self, mesh: &SphereMesh, partition: &PlatePartition) -> f32 {
        let ocean_area: f64 = partition
            .cell_plates
            .iter()
            .zip(&mesh.cell_areas)
            .filter(|(plate, _)| self.plate_classes[**plate] == CrustClass::Oceanic)
            .map(|(_, &area)| f64::from(area))
            .sum();
        let total_area: f64 = mesh.cell_areas.iter().map(|&area| f64::from(area)).sum();
        (ocean_area / total_area) as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrustClassificationError {
    InvalidOceanFraction,
    CellCountMismatch,
}

impl fmt::Display for CrustClassificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOceanFraction => {
                formatter.write_str("target ocean fraction must be finite and between 0 and 1")
            }
            Self::CellCountMismatch => {
                formatter.write_str("plate assignments must match the mesh cell count")
            }
        }
    }
}

impl std::error::Error for CrustClassificationError {}

/// Assigns one immutable crust class to each plate. Plate candidates are
/// visited in a seeded, deterministic order and selected only when their
/// complete surface area moves the achieved ocean fraction closer to the
/// requested target.
pub fn classify_crust(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    config: CrustClassificationConfig,
) -> Result<CrustClassification, CrustClassificationError> {
    if !config.target_ocean_fraction.is_finite()
        || !(0.0..=1.0).contains(&config.target_ocean_fraction)
    {
        return Err(CrustClassificationError::InvalidOceanFraction);
    }
    if partition.cell_plates.len() != mesh.cell_count() {
        return Err(CrustClassificationError::CellCountMismatch);
    }

    let plate_count = partition.plate_count();
    let mut plate_areas = vec![0.0_f64; plate_count];
    for (&plate, &area) in partition.cell_plates.iter().zip(&mesh.cell_areas) {
        plate_areas[plate] += f64::from(area);
    }

    let total_area: f64 = plate_areas.iter().sum();
    let target_area = total_area * f64::from(config.target_ocean_fraction);
    let random = RandomStream::new(config.seed, CRUST_PLATE_ORDER);
    let mut plate_order: Vec<_> = (0..plate_count).collect();
    plate_order.sort_unstable_by_key(|&plate| (random.sample_u64(plate as u64, 0), plate));

    let mut ocean_area = 0.0_f64;
    let mut plate_classes = vec![CrustClass::Continental; plate_count];
    for plate in plate_order {
        let candidate_area = ocean_area + plate_areas[plate];
        if (candidate_area - target_area).abs() < (ocean_area - target_area).abs() {
            plate_classes[plate] = CrustClass::Oceanic;
            ocean_area = candidate_area;
        }
    }

    Ok(CrustClassification { plate_classes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mesh;
    use crate::{PlatePartitionConfig, partition_plates};

    fn fixture() -> (SphereMesh, PlatePartition) {
        let mesh = mesh(512);
        let partition = partition_plates(
            &mesh,
            PlatePartitionConfig {
                major_plate_count: 5,
                minor_plate_count: 11,
                major_head_start_rounds: 2,
                seed: 7,
            },
        )
        .unwrap();
        (mesh, partition)
    }

    #[test]
    fn classification_is_deterministic_and_seeded() {
        let (mesh, partition) = fixture();
        let config = CrustClassificationConfig::new(17);
        let first = classify_crust(&mesh, &partition, config).unwrap();

        assert_eq!(first, classify_crust(&mesh, &partition, config).unwrap());
        assert_ne!(
            first.plate_classes,
            classify_crust(
                &mesh,
                &partition,
                CrustClassificationConfig { seed: 18, ..config }
            )
            .unwrap()
            .plate_classes
        );
    }

    #[test]
    fn cell_crust_follows_plate_ownership_and_area_drives_fraction() {
        let (mesh, partition) = fixture();
        let crust = classify_crust(&mesh, &partition, CrustClassificationConfig::new(17)).unwrap();

        assert_eq!(crust.plate_classes.len(), partition.plate_count());
        for (cell, &plate) in partition.cell_plates.iter().enumerate() {
            assert_eq!(
                crust.cell_class(&partition, cell),
                crust.plate_classes[plate]
            );
        }

        assert!((crust.ocean_fraction(&mesh, &partition) - 0.7).abs() < 0.1);
    }

    #[test]
    fn derived_cell_crust_tracks_current_plate_ownership() {
        let (mesh, mut partition) = fixture();
        let crust = classify_crust(&mesh, &partition, CrustClassificationConfig::new(17)).unwrap();
        let cell = 0;
        let original_class = crust.cell_class(&partition, cell);
        let replacement_plate = crust
            .plate_classes
            .iter()
            .position(|&class| class != original_class)
            .unwrap();

        partition.cell_plates[cell] = replacement_plate;

        assert_ne!(crust.cell_class(&partition, cell), original_class);
    }

    #[test]
    fn fraction_extremes_classify_every_plate() {
        let (mesh, partition) = fixture();
        let continental = classify_crust(
            &mesh,
            &partition,
            CrustClassificationConfig {
                target_ocean_fraction: 0.0,
                seed: 1,
            },
        )
        .unwrap();
        let oceanic = classify_crust(
            &mesh,
            &partition,
            CrustClassificationConfig {
                target_ocean_fraction: 1.0,
                seed: 1,
            },
        )
        .unwrap();

        assert!(
            continental
                .plate_classes
                .iter()
                .all(|&class| class == CrustClass::Continental)
        );
        assert!(
            oceanic
                .plate_classes
                .iter()
                .all(|&class| class == CrustClass::Oceanic)
        );
        assert_eq!(continental.ocean_fraction(&mesh, &partition), 0.0);
        assert_eq!(oceanic.ocean_fraction(&mesh, &partition), 1.0);
    }

    #[test]
    fn rejects_invalid_inputs() {
        let (mesh, partition) = fixture();
        for target in [-0.1, 1.1, f32::NAN] {
            assert_eq!(
                classify_crust(
                    &mesh,
                    &partition,
                    CrustClassificationConfig {
                        target_ocean_fraction: target,
                        seed: 0,
                    },
                ),
                Err(CrustClassificationError::InvalidOceanFraction)
            );
        }

        let mut short_partition = partition.clone();
        short_partition.cell_plates.pop();
        assert_eq!(
            classify_crust(
                &mesh,
                &short_partition,
                CrustClassificationConfig {
                    target_ocean_fraction: 0.7,
                    seed: 0,
                }
            ),
            Err(CrustClassificationError::CellCountMismatch)
        );
    }
}
