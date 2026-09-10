use crate::{PlatePartition, StageInputError};
use procgen_core::{RandomStream, random_streams::CRUST_PLATE_ORDER};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CrustClass {
    Oceanic,
    Continental,
}

impl CrustClass {
    pub const ALL: [Self; 2] = [Self::Oceanic, Self::Continental];
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
    /// Validates that every stable plate identity has one crust class.
    pub fn validate(&self, partition: &PlatePartition) -> Result<(), StageInputError> {
        if self.plate_classes.len() != partition.plate_count {
            return Err(StageInputError::Plates);
        }
        Ok(())
    }

    pub fn plate_count(&self, class: CrustClass) -> usize {
        self.plate_classes
            .iter()
            .filter(|&&candidate| candidate == class)
            .count()
    }

    /// Derives the current area-weighted ocean fraction from cell ownership.
    pub fn ocean_fraction(&self, mesh: &SphereMesh, partition: &PlatePartition) -> f32 {
        let areas = partition.plate_areas(mesh);
        let ocean_area: f64 = areas
            .iter()
            .zip(&self.plate_classes)
            .filter(|(_, class)| **class == CrustClass::Oceanic)
            .map(|(&area, _)| area)
            .sum();
        (ocean_area / mesh.total_area()) as f32
    }
}

/// Per-cell crust class after evolution, read from the step at which each
/// cell's crust was created: crust with a birth step is oceanic, crust with
/// none is original continental crust that has never been re-made.
///
/// This is the only per-cell answer once evolution has run. A plate's class in
/// [`CrustClassification`] describes the plate, not the cells it currently
/// owns: a continental plate that rifts grows an oceanic margin, and a cell
/// overridden at a subduction zone takes the overriding material's class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellCrust<'a> {
    pub cell_birth: &'a [Option<i32>],
}

impl CellCrust<'_> {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), StageInputError> {
        if self.cell_birth.len() != mesh.cell_count() {
            return Err(StageInputError::CrustBirth);
        }
        Ok(())
    }

    pub fn class(&self, cell: usize) -> CrustClass {
        match self.cell_birth[cell] {
            Some(_) => CrustClass::Oceanic,
            None => CrustClass::Continental,
        }
    }

    /// Cells of each crust class, in [`CrustClass::ALL`] order. Both counts
    /// come from one scan, because a consumer showing either usually shows
    /// both.
    pub fn cell_counts(&self) -> [usize; CrustClass::ALL.len()] {
        let oceanic = self
            .cell_birth
            .iter()
            .filter(|birth| birth.is_some())
            .count();
        [oceanic, self.cell_birth.len() - oceanic]
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

    let plate_count = partition.plate_count;
    let plate_areas = partition.plate_areas(mesh);

    let target_area = mesh.total_area() * f64::from(config.target_ocean_fraction);
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
    use crate::test_support::reference_partition;

    #[test]
    fn classification_is_deterministic_and_seeded() {
        let (mesh, partition) = reference_partition();
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
    fn plate_area_drives_the_achieved_ocean_fraction() {
        let (mesh, partition) = reference_partition();
        let crust = classify_crust(&mesh, &partition, CrustClassificationConfig::new(17)).unwrap();

        assert_eq!(crust.plate_classes.len(), partition.plate_count);
        assert!((crust.ocean_fraction(&mesh, &partition) - 0.7).abs() < 0.1);
    }

    #[test]
    fn cell_crust_reads_the_birth_step_and_not_plate_ownership() {
        let mesh = crate::test_support::mesh(32);
        let cell_birth: Vec<_> = (0..mesh.cell_count())
            .map(|cell| (cell % 3 != 0).then_some(-(cell as i32)))
            .collect();
        let crust = CellCrust {
            cell_birth: &cell_birth,
        };

        assert_eq!(crust.validate(&mesh), Ok(()));
        assert_eq!(crust.class(0), CrustClass::Continental);
        assert_eq!(crust.class(1), CrustClass::Oceanic);
        // Two cells in three are oceanic, so the counts also pin their order.
        let [oceanic, continental] = crust.cell_counts();
        assert_eq!(oceanic + continental, mesh.cell_count());
        assert!(oceanic > continental);
        assert_eq!(
            CellCrust {
                cell_birth: &cell_birth[1..]
            }
            .validate(&mesh),
            Err(StageInputError::CrustBirth)
        );
    }

    #[test]
    fn fraction_extremes_classify_every_plate() {
        let (mesh, partition) = reference_partition();
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
        let (mesh, partition) = reference_partition();
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
