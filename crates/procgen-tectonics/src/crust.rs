//! Continental crust as a per-cell property of the mesh, independent of the
//! plate partition.
//!
//! `nucleus_count` nuclei are placed farthest-first from one hashed cell, and
//! the partition's shortest-arrival growth spreads them over per-edge integer
//! costs until the settled area reaches `continental_fraction` of the sphere.
//! Growth is bounded by nothing: it crosses plate boundaries freely, so a
//! continent's edge falls wherever it falls relative to them. An edge inside a
//! plate is a passive margin and one on a boundary is an active margin, and
//! both arise without a rule for either.
//!
//! Cells growth reached are continental and every other cell is oceanic. That
//! is the initial condition alone: from step zero onward a cell's crust is
//! read from the step its crust was created, through [`CellCrust`], so a
//! rifting continent grows an oceanic margin and an overridden cell takes the
//! overriding material's class without anything storing a second answer.
//! Plates have no crust class; the readers that want a plate-level number
//! take [`CrustClassification::plate_continental_fraction`].
//!
//! Determinism: the nuclei are hashes and squared chord distances compared by
//! `total_cmp`, the arrival costs are integers, and the area budget is an f64
//! sum compared against an f64 target, so the mask is bit-identical across
//! machines.

use crate::{
    MAX_GROWTH_ROUGHNESS, PlatePartition, StageInputError,
    partition::{GrowthBounds, GrowthCosts, PlateGrowth, farthest_first},
};
use procgen_core::{
    RandomStream,
    random_streams::{CRUST_GROWTH_COST, CRUST_NUCLEUS},
};
use procgen_sphere_mesh::{SphereMesh, connected_components};
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
    /// Target fraction of the sphere's surface covered by continental crust.
    pub continental_fraction: f32,
    /// Continental nuclei growth starts from, the order of Earth's cratonic
    /// assemblies. Must be at least one and at most the mesh's cell count.
    pub nucleus_count: usize,
    /// Maximum percentage that an edge's deterministic traversal cost varies
    /// above or below the baseline, in the partition's units and bounded by
    /// `MAX_GROWTH_ROUGHNESS`. Zero grows round continents.
    pub growth_roughness: u32,
    pub seed: u64,
}

impl CrustClassificationConfig {
    pub const fn new(seed: u64) -> Self {
        Self {
            continental_fraction: 0.3,
            nucleus_count: 8,
            growth_roughness: MAX_GROWTH_ROUGHNESS,
            seed,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CrustClassificationDiagnostics {
    /// Continental share of the sphere's area the growth achieved, which
    /// exceeds the target by at most one cell's area.
    pub continental_fraction: f32,
    /// Connected components of continental crust, which is fewer than
    /// `nucleus_count` when two nuclei grew together.
    pub component_count: usize,
}

/// Per-cell crust class before step zero.
#[derive(Clone, Debug, PartialEq)]
pub struct CrustClassification {
    pub cell_classes: Vec<CrustClass>,
    pub diagnostics: CrustClassificationDiagnostics,
}

impl CrustClassification {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), StageInputError> {
        if self.cell_classes.len() != mesh.cell_count() {
            return Err(StageInputError::CrustClasses);
        }
        Ok(())
    }

    pub fn class(&self, cell: usize) -> CrustClass {
        self.cell_classes[cell]
    }

    /// Area-weighted continental share of each plate, for the readers that
    /// want a plate-level number out of a per-cell fact. Every plate owns a
    /// cell, so every share is defined.
    pub fn plate_continental_fraction(
        &self,
        mesh: &SphereMesh,
        partition: &PlatePartition,
    ) -> Vec<f64> {
        let mut continental = vec![0.0; partition.plate_count];
        for (cell, &plate) in partition.cell_plates.iter().enumerate() {
            if self.cell_classes[cell] == CrustClass::Continental {
                continental[plate] += f64::from(mesh.cell_areas[cell]);
            }
        }
        for (share, area) in continental.iter_mut().zip(partition.plate_areas(mesh)) {
            *share /= area;
        }
        continental
    }
}

/// Per-cell crust class after evolution, read from the step at which each
/// cell's crust was created: crust with a birth step is oceanic, crust with
/// none is original continental crust that has never been re-made.
///
/// This is the only per-cell answer from step zero onward.
/// [`CrustClassification`] is the initial condition it starts from.
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
    InvalidContinentalFraction,
    InvalidNucleusCount,
    InvalidGrowthRoughness,
}

impl fmt::Display for CrustClassificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidContinentalFraction => {
                formatter.write_str("continental fraction must be finite and between 0 and 1")
            }
            Self::InvalidNucleusCount => formatter
                .write_str("nucleus count must be at least one and at most the mesh cell count"),
            Self::InvalidGrowthRoughness => write!(
                formatter,
                "crust growth roughness cannot exceed {MAX_GROWTH_ROUGHNESS}%"
            ),
        }
    }
}

impl std::error::Error for CrustClassificationError {}

/// Grows continental nuclei to a target share of the sphere and calls every
/// cell they reached continental.
///
/// The nuclei are one hashed cell and then the cells farthest from those
/// already chosen, so they spread over the sphere without reference to plates.
/// Growth settles the cheapest arrival until the settled area passes the
/// target, so the achieved area overshoots by at most one cell.
pub fn classify_crust(
    mesh: &SphereMesh,
    config: CrustClassificationConfig,
) -> Result<CrustClassification, CrustClassificationError> {
    if !config.continental_fraction.is_finite()
        || !(0.0..=1.0).contains(&config.continental_fraction)
    {
        return Err(CrustClassificationError::InvalidContinentalFraction);
    }
    if config.nucleus_count == 0 || config.nucleus_count > mesh.cell_count() {
        return Err(CrustClassificationError::InvalidNucleusCount);
    }
    if config.growth_roughness > MAX_GROWTH_ROUGHNESS {
        return Err(CrustClassificationError::InvalidGrowthRoughness);
    }

    let mut growth = PlateGrowth::new(
        mesh,
        GrowthBounds::WholeSphere,
        GrowthCosts {
            roughness: config.growth_roughness,
            stream: RandomStream::new(config.seed, CRUST_GROWTH_COST),
        },
    );
    for (nucleus, &cell) in crust_nuclei(mesh, config).iter().enumerate() {
        growth.seed(cell, nucleus);
    }
    let continental_area =
        growth.grow_to_area(f64::from(config.continental_fraction) * mesh.total_area());

    let cell_classes: Vec<_> = (0..mesh.cell_count())
        .map(|cell| match growth.reached(cell) {
            true => CrustClass::Continental,
            false => CrustClass::Oceanic,
        })
        .collect();
    let component_count = connected_components(
        mesh,
        |cell| cell_classes[cell] == CrustClass::Continental,
        |_, _| true,
    )
    .len();
    Ok(CrustClassification {
        cell_classes,
        diagnostics: CrustClassificationDiagnostics {
            continental_fraction: (continental_area / mesh.total_area()) as f32,
            component_count,
        },
    })
}

/// The cells the continents grow from: one hashed cell, then the cells
/// farthest from those already chosen. `nucleus_count` is at most the cell
/// count, so every nucleus is a distinct cell.
pub(crate) fn crust_nuclei(mesh: &SphereMesh, config: CrustClassificationConfig) -> Vec<usize> {
    let cells: Vec<usize> = (0..mesh.cell_count()).collect();
    let first =
        RandomStream::new(config.seed, CRUST_NUCLEUS).sample_u64(0, 0) % mesh.cell_count() as u64;
    farthest_first(mesh, &cells, first as usize, config.nucleus_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{fingerprint, mesh, reference_partition};

    fn class_fingerprint(crust: &CrustClassification) -> u64 {
        fingerprint(crust.cell_classes.iter().map(|&class| class as u8 as u64))
    }

    #[test]
    fn classification_is_deterministic_and_seeded() {
        let mesh = mesh(512);
        let config = CrustClassificationConfig::new(17);
        let first = classify_crust(&mesh, config).unwrap();

        assert_eq!(first, classify_crust(&mesh, config).unwrap());
        assert_ne!(
            first.cell_classes,
            classify_crust(&mesh, CrustClassificationConfig { seed: 18, ..config })
                .unwrap()
                .cell_classes
        );
    }

    #[test]
    fn reference_classification_has_stable_fingerprint() {
        let mesh = mesh(512);
        let crust = classify_crust(&mesh, CrustClassificationConfig::new(17)).unwrap();

        // Hashes, integer arrival costs, and f64 area sums only, so this value
        // is expected to match on both the macOS and the Windows development
        // machine.
        assert_eq!(class_fingerprint(&crust), 18_098_810_093_538_859_437);
    }

    #[test]
    fn growth_stops_within_one_cell_of_the_target_area() {
        let mesh = mesh(512);
        // One cell's area is about a five-hundredth of the sphere, so the
        // largest cell bounds the overshoot at every fraction.
        let widest_cell =
            f64::from(mesh.cell_areas.iter().copied().fold(f32::MIN, f32::max)) / mesh.total_area();
        for fraction in [0.1, 0.3, 0.6] {
            let crust = classify_crust(
                &mesh,
                CrustClassificationConfig {
                    continental_fraction: fraction,
                    ..CrustClassificationConfig::new(17)
                },
            )
            .unwrap();
            let achieved = f64::from(crust.diagnostics.continental_fraction);
            assert!(
                achieved >= f64::from(fraction) && achieved - f64::from(fraction) <= widest_cell,
                "fraction {fraction} achieved {achieved}"
            );
        }
    }

    #[test]
    fn every_continental_cell_grew_from_a_nucleus() {
        let mesh = mesh(512);
        let config = CrustClassificationConfig::new(17);
        let crust = classify_crust(&mesh, config).unwrap();
        let nuclei = crust_nuclei(&mesh, config);

        let components = connected_components(
            &mesh,
            |cell| crust.class(cell) == CrustClass::Continental,
            |_, _| true,
        );
        assert_eq!(components.len(), crust.diagnostics.component_count);
        assert!(crust.diagnostics.component_count >= 1);
        assert!(crust.diagnostics.component_count <= config.nucleus_count);
        // Every component holds a nucleus and every continental cell is in a
        // component, so each grew from a nucleus through continental crust
        // alone. Two nuclei in one component are two continents that met.
        for component in &components {
            assert!(
                component.iter().any(|cell| nuclei.contains(cell)),
                "a continent with no nucleus"
            );
        }
        assert_eq!(
            components.iter().map(Vec::len).sum::<usize>(),
            crust
                .cell_classes
                .iter()
                .filter(|&&class| class == CrustClass::Continental)
                .count()
        );
        assert!(
            nuclei
                .iter()
                .all(|&cell| crust.class(cell) == CrustClass::Continental)
        );
    }

    #[test]
    fn plate_shares_average_to_the_achieved_fraction() {
        let (mesh, partition) = reference_partition();
        let crust = classify_crust(&mesh, CrustClassificationConfig::new(17)).unwrap();

        let shares = crust.plate_continental_fraction(&mesh, &partition);
        assert_eq!(shares.len(), partition.plate_count);
        assert!(shares.iter().all(|share| (0.0..=1.0).contains(share)));
        // A continent that ends inside a plate leaves that plate partly
        // continental, which is the passive margin the stage exists for.
        assert!(
            shares.iter().any(|&share| share > 0.0 && share < 1.0),
            "no plate carries both crusts"
        );
        let weighted: f64 = shares
            .iter()
            .zip(partition.plate_areas(&mesh))
            .map(|(share, area)| share * area)
            .sum();
        assert!(
            (weighted / mesh.total_area() - f64::from(crust.diagnostics.continental_fraction))
                .abs()
                < 1.0e-6,
            "plate shares do not sum to the achieved fraction"
        );
    }

    #[test]
    fn cell_crust_reads_the_birth_step_and_not_plate_ownership() {
        let mesh = mesh(32);
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
    fn fraction_extremes_classify_every_cell() {
        let mesh = mesh(512);
        let config = CrustClassificationConfig::new(1);
        let continental = classify_crust(
            &mesh,
            CrustClassificationConfig {
                continental_fraction: 1.0,
                ..config
            },
        )
        .unwrap();
        let oceanic = classify_crust(
            &mesh,
            CrustClassificationConfig {
                continental_fraction: 0.0,
                ..config
            },
        )
        .unwrap();

        assert!(
            continental
                .cell_classes
                .iter()
                .all(|&class| class == CrustClass::Continental)
        );
        assert_eq!(continental.diagnostics.component_count, 1);
        // The nuclei themselves are settled before the budget is consulted, so
        // a zero fraction leaves exactly them continental.
        assert_eq!(
            oceanic
                .cell_classes
                .iter()
                .filter(|&&class| class == CrustClass::Continental)
                .count(),
            config.nucleus_count
        );
    }

    #[test]
    fn rejects_invalid_configurations() {
        let mesh = mesh(64);
        let valid = CrustClassificationConfig::new(0);
        for (config, expected) in [
            (
                CrustClassificationConfig {
                    continental_fraction: -0.1,
                    ..valid
                },
                CrustClassificationError::InvalidContinentalFraction,
            ),
            (
                CrustClassificationConfig {
                    continental_fraction: 1.1,
                    ..valid
                },
                CrustClassificationError::InvalidContinentalFraction,
            ),
            (
                CrustClassificationConfig {
                    continental_fraction: f32::NAN,
                    ..valid
                },
                CrustClassificationError::InvalidContinentalFraction,
            ),
            (
                CrustClassificationConfig {
                    nucleus_count: 0,
                    ..valid
                },
                CrustClassificationError::InvalidNucleusCount,
            ),
            (
                CrustClassificationConfig {
                    nucleus_count: mesh.cell_count() + 1,
                    ..valid
                },
                CrustClassificationError::InvalidNucleusCount,
            ),
            (
                CrustClassificationConfig {
                    growth_roughness: MAX_GROWTH_ROUGHNESS + 1,
                    ..valid
                },
                CrustClassificationError::InvalidGrowthRoughness,
            ),
        ] {
            assert_eq!(classify_crust(&mesh, config), Err(expected));
        }

        let crust = classify_crust(&mesh, valid).unwrap();
        assert_eq!(crust.validate(&mesh), Ok(()));
        assert_eq!(
            CrustClassification {
                cell_classes: crust.cell_classes[1..].to_vec(),
                ..crust
            }
            .validate(&mesh),
            Err(StageInputError::CrustClasses)
        );
    }
}
