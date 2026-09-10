//! Oceanic crust age at the two ends of evolution.
//!
//! Before step zero there is no history to read, so a prior stands in for one:
//! hop distance from the initial ridges within the initial owner, negated, is
//! the step at which each oceanic cell's crust would have been created had the
//! run started earlier. Evolution carries that field forward, overriding crust
//! at subduction zones and creating it at ridges, and the final age is simply
//! the steps elapsed since each cell's birth. Continental crust has no age at
//! either end.

use crate::{
    BoundaryClass, BoundaryClassification, CrustClass, CrustClassification, FieldSummary,
    PlateEvolution, PlatePartition, stage::StageInputError,
};
use procgen_sphere_mesh::{SphereMesh, multi_source_distances};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrustBirthPriorConfig {
    /// Hop age before step zero given to every cell on an oceanic plate with
    /// no divergent boundary of its own.
    pub ridge_less_age: usize,
}

impl Default for CrustBirthPriorConfig {
    fn default() -> Self {
        Self { ridge_less_age: 8 }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CrustBirthPriorDiagnostics {
    /// Summary of the hop ages the prior turned into birth steps.
    pub hops: FieldSummary,
    pub oceanic_cell_count: usize,
    pub ridge_cell_count: usize,
    pub ridge_plate_count: usize,
    /// Plates that own oceanic crust but no ridge of their own, whose cells
    /// all take the fallback age.
    pub ridge_less_plate_count: usize,
    pub fallback_cell_count: usize,
}

/// Per-cell crust birth before step zero. Continental cells have no birth.
#[derive(Clone, Debug, PartialEq)]
pub struct CrustBirthPrior {
    pub cell_birth: Vec<Option<i32>>,
    pub diagnostics: CrustBirthPriorDiagnostics,
}

impl CrustBirthPrior {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), StageInputError> {
        if self.cell_birth.len() != mesh.cell_count() {
            return Err(StageInputError::CrustBirth);
        }
        Ok(())
    }
}

/// Derives the crust birth evolution starts from, out of the initial ridges
/// and ownership.
///
/// Oceanic cells touching a divergent edge are born at step zero. A
/// multi-source BFS propagates one step of age per hop, only through oceanic
/// cells with the same initial plate owner, so a cell `h` hops from its
/// nearest ridge was born `h` steps before the run. Oceanic cells whose plate
/// has no ridge of its own receive the configured fallback age uniformly.
///
/// This is the one stage that reads the initial per-cell classification: it
/// turns that mask into the birth field, and from step zero onward
/// [`crate::CellCrust`] over birth is the answer. Continental cells have no
/// birth, so the walk never crosses one and a plate that carries both crusts
/// dates only its ocean.
pub fn derive_crust_birth_prior(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    boundaries: &BoundaryClassification,
    config: CrustBirthPriorConfig,
) -> Result<CrustBirthPrior, StageInputError> {
    partition.validate(mesh)?;
    crust.validate(mesh)?;
    boundaries.validate(mesh)?;

    let oceanic = |cell: usize| crust.class(cell) == CrustClass::Oceanic;
    let mut ridge_plates = vec![false; partition.plate_count];
    let mut ridge_cells = Vec::new();

    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        if boundaries.edge_classes[edge_index] != BoundaryClass::Divergent {
            continue;
        }
        for &cell in &edge.cells {
            if !oceanic(cell) {
                continue;
            }
            ridge_plates[partition.cell_plates[cell]] = true;
            ridge_cells.push(cell);
        }
    }
    let mut cell_hops = multi_source_distances(mesh, &ridge_cells, |cell, neighbor| {
        partition.cell_plates[cell] == partition.cell_plates[neighbor] && oceanic(neighbor)
    });
    let ridge_cell_count = cell_hops.iter().filter(|&&hops| hops == Some(0)).count();

    let mut oceanic_plates = vec![false; partition.plate_count];
    let mut fallback_cell_count = 0;
    for (cell, hops) in cell_hops.iter_mut().enumerate() {
        if !oceanic(cell) {
            continue;
        }
        oceanic_plates[partition.cell_plates[cell]] = true;
        if hops.is_none() {
            *hops = Some(config.ridge_less_age);
            fallback_cell_count += 1;
        }
    }
    let oceanic_hops: Vec<_> = cell_hops
        .iter()
        .flatten()
        .map(|&hops| hops as f32)
        .collect();
    let ridge_plate_count = ridge_plates.iter().filter(|&&has_ridge| has_ridge).count();
    let diagnostics = CrustBirthPriorDiagnostics {
        hops: FieldSummary::from_values(&oceanic_hops),
        oceanic_cell_count: oceanic_hops.len(),
        ridge_cell_count,
        ridge_plate_count,
        ridge_less_plate_count: oceanic_plates.iter().filter(|&&oceanic| oceanic).count()
            - ridge_plate_count,
        fallback_cell_count,
    };

    Ok(CrustBirthPrior {
        cell_birth: cell_hops
            .iter()
            .map(|hops| hops.map(|hops| -(hops as i32)))
            .collect(),
        diagnostics,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeafloorAgeDiagnostics {
    pub summary: FieldSummary,
    pub oceanic_cell_count: usize,
}

/// Per-cell seafloor age measured in evolution steps since the cell's crust
/// was created. Continental cells have no seafloor age.
#[derive(Clone, Debug, PartialEq)]
pub struct SeafloorAge {
    pub cell_ages: Vec<Option<usize>>,
    pub diagnostics: SeafloorAgeDiagnostics,
}

impl SeafloorAge {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), StageInputError> {
        if self.cell_ages.len() != mesh.cell_count() {
            return Err(StageInputError::SeafloorAge);
        }
        Ok(())
    }
}

/// Derives seafloor age from crust birth: the steps that have elapsed since
/// each cell's crust was created.
///
/// `elapsed_steps` is the step count the evolution ran, so a birth after it
/// means the two do not describe the same run and is rejected.
pub fn derive_seafloor_age(
    mesh: &SphereMesh,
    evolution: &PlateEvolution,
    elapsed_steps: usize,
) -> Result<SeafloorAge, StageInputError> {
    evolution.validate(mesh)?;

    let elapsed = elapsed_steps as i32;
    if evolution
        .cell_birth
        .iter()
        .flatten()
        .any(|&birth| birth > elapsed)
    {
        return Err(StageInputError::CrustBirthAfterRun);
    }
    let cell_ages: Vec<_> = evolution
        .cell_birth
        .iter()
        .map(|birth| birth.map(|birth| (elapsed - birth) as usize))
        .collect();
    let oceanic_ages: Vec<_> = cell_ages.iter().flatten().map(|&age| age as f32).collect();

    Ok(SeafloorAge {
        diagnostics: SeafloorAgeDiagnostics {
            summary: FieldSummary::from_values(&oceanic_ages),
            oceanic_cell_count: oceanic_ages.len(),
        },
        cell_ages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        empty_boundaries, evolution_fixture, final_state_fixture, fingerprint, plate_crust,
        reference_evolution_config, reference_partition,
    };

    fn birth_fingerprint(cell_birth: &[Option<i32>]) -> u64 {
        fingerprint(
            cell_birth
                .iter()
                .map(|birth| birth.map_or(u64::MAX, |birth| birth as u32 as u64)),
        )
    }

    #[test]
    fn the_prior_is_deterministic_and_has_stable_aggregates() {
        let (mesh, partition) = reference_partition();
        let fixture = evolution_fixture();
        let config = CrustBirthPriorConfig::default();
        let first = derive_crust_birth_prior(
            &mesh,
            &partition,
            &fixture.crust,
            &fixture.boundaries,
            config,
        )
        .unwrap();

        assert_eq!(
            first,
            derive_crust_birth_prior(
                &mesh,
                &partition,
                &fixture.crust,
                &fixture.boundaries,
                config
            )
            .unwrap()
        );
        assert_eq!(first.cell_birth.len(), mesh.cell_count());
        assert_eq!(
            first.diagnostics,
            CrustBirthPriorDiagnostics {
                hops: FieldSummary {
                    minimum: 0.0,
                    maximum: 9.0,
                    mean: 1.801_775_1,
                },
                oceanic_cell_count: 338,
                ridge_cell_count: 103,
                ridge_plate_count: 29,
                // One plate of the reference world owns oceanic crust with no
                // ridge of its own, so its cells take the fallback age.
                ridge_less_plate_count: 1,
                fallback_cell_count: 7,
            }
        );
        assert_eq!(
            birth_fingerprint(&first.cell_birth),
            7_279_187_007_399_044_302
        );
        assert!(
            first.cell_birth.iter().flatten().all(|&birth| birth <= 0),
            "the prior predates step zero"
        );
    }

    #[test]
    fn ridge_less_oceanic_plates_use_the_configured_age_and_continents_stay_empty() {
        let (mesh, partition) = reference_partition();
        let plate_classes: Vec<_> = (0..partition.plate_count)
            .map(|plate| match plate {
                0 => CrustClass::Continental,
                _ => CrustClass::Oceanic,
            })
            .collect();
        let crust = plate_crust(&partition, &plate_classes);
        let prior = derive_crust_birth_prior(
            &mesh,
            &partition,
            &crust,
            &empty_boundaries(&mesh),
            CrustBirthPriorConfig { ridge_less_age: 13 },
        )
        .unwrap();

        for (cell, &plate) in partition.cell_plates.iter().enumerate() {
            assert_eq!(
                prior.cell_birth[cell],
                (plate != 0).then_some(-13),
                "cell {cell} on plate {plate}"
            );
        }
        assert_eq!(prior.diagnostics.ridge_plate_count, 0);
        assert_eq!(
            prior.diagnostics.ridge_less_plate_count,
            partition.plate_count - 1
        );
        assert_eq!(
            prior.diagnostics.fallback_cell_count,
            prior.diagnostics.oceanic_cell_count
        );
    }

    #[test]
    fn all_continental_world_has_no_prior_or_aggregates() {
        let (mesh, partition) = reference_partition();
        let crust = plate_crust(
            &partition,
            &vec![CrustClass::Continental; partition.plate_count],
        );

        let prior = derive_crust_birth_prior(
            &mesh,
            &partition,
            &crust,
            &empty_boundaries(&mesh),
            CrustBirthPriorConfig::default(),
        )
        .unwrap();

        assert!(prior.cell_birth.iter().all(Option::is_none));
        assert_eq!(prior.diagnostics, CrustBirthPriorDiagnostics::default());
    }

    #[test]
    fn prior_propagation_stays_within_initial_plate_ownership() {
        let (mesh, partition) = reference_partition();
        let mut boundaries = empty_boundaries(&mesh);
        let edge_index = mesh
            .edges
            .iter()
            .position(|edge| {
                partition.cell_plates[edge.cells[0]] != partition.cell_plates[edge.cells[1]]
            })
            .unwrap();
        boundaries.edge_classes[edge_index] = BoundaryClass::Divergent;
        let ridge_edge = mesh.edges[edge_index];
        let ridge_plates = ridge_edge.cells.map(|cell| partition.cell_plates[cell]);
        let crust = plate_crust(
            &partition,
            &vec![CrustClass::Oceanic; partition.plate_count],
        );
        let prior = derive_crust_birth_prior(
            &mesh,
            &partition,
            &crust,
            &boundaries,
            CrustBirthPriorConfig { ridge_less_age: 23 },
        )
        .unwrap();

        assert_eq!(prior.cell_birth[ridge_edge.cells[0]], Some(0));
        assert_eq!(prior.cell_birth[ridge_edge.cells[1]], Some(0));
        let same_plate_neighbor = mesh
            .cell_corners(ridge_edge.cells[0])
            .iter()
            .map(|corner| corner.neighbor)
            .find(|&neighbor| partition.cell_plates[neighbor] == ridge_plates[0])
            .unwrap();
        assert_eq!(prior.cell_birth[same_plate_neighbor], Some(-1));
        for (cell, &plate) in partition.cell_plates.iter().enumerate() {
            if !ridge_plates.contains(&plate) {
                assert_eq!(prior.cell_birth[cell], Some(-23));
            }
        }
        assert_eq!(prior.diagnostics.ridge_plate_count, 2);
        assert_eq!(
            prior.diagnostics.ridge_less_plate_count,
            partition.plate_count - 2
        );
    }

    #[test]
    fn the_prior_rejects_mismatched_inputs() {
        let (mesh, partition) = reference_partition();
        let crust = plate_crust(
            &partition,
            &vec![CrustClass::Oceanic; partition.plate_count],
        );
        let boundaries = empty_boundaries(&mesh);

        let mut wrong_partition = partition.clone();
        wrong_partition.cell_plates.pop();
        assert_eq!(
            derive_crust_birth_prior(
                &mesh,
                &wrong_partition,
                &crust,
                &boundaries,
                Default::default()
            ),
            Err(StageInputError::Cells)
        );

        let wrong_crust = CrustClassification {
            cell_classes: Vec::new(),
            ..crust.clone()
        };
        assert_eq!(
            derive_crust_birth_prior(
                &mesh,
                &partition,
                &wrong_crust,
                &boundaries,
                Default::default()
            ),
            Err(StageInputError::CrustClasses)
        );

        let mut wrong_boundaries = boundaries;
        wrong_boundaries.edge_classes.pop();
        assert_eq!(
            derive_crust_birth_prior(
                &mesh,
                &partition,
                &crust,
                &wrong_boundaries,
                Default::default()
            ),
            Err(StageInputError::Boundaries)
        );
    }

    #[test]
    fn final_age_counts_the_steps_since_each_cell_was_born() {
        let (mesh, _, evolution) = final_state_fixture();
        let elapsed = reference_evolution_config().step_count;
        let first = derive_seafloor_age(&mesh, &evolution, elapsed).unwrap();

        assert_eq!(
            first,
            derive_seafloor_age(&mesh, &evolution, elapsed).unwrap()
        );
        assert_eq!(first.cell_ages.len(), mesh.cell_count());
        for (cell, age) in first.cell_ages.iter().enumerate() {
            assert_eq!(
                age.is_some(),
                evolution.cell_crust().class(cell) == CrustClass::Oceanic
            );
            if let (Some(age), Some(birth)) = (age, evolution.cell_birth[cell]) {
                assert_eq!(*age as i32, elapsed as i32 - birth);
            }
        }
        assert_eq!(
            first.diagnostics.oceanic_cell_count,
            first.cell_ages.iter().flatten().count()
        );
        assert_eq!(
            fingerprint(
                first
                    .cell_ages
                    .iter()
                    .map(|age| age.map_or(u64::MAX, |age| age as u64))
            ),
            18_163_853_424_571_989_242
        );
    }

    #[test]
    fn an_elapsed_count_shorter_than_the_run_is_rejected() {
        let (mesh, _, evolution) = final_state_fixture();
        let latest = evolution
            .cell_birth
            .iter()
            .flatten()
            .max()
            .copied()
            .unwrap();
        assert!(latest > 0, "the reference run creates crust");

        assert_eq!(
            derive_seafloor_age(&mesh, &evolution, latest as usize - 1),
            Err(StageInputError::CrustBirthAfterRun)
        );
        assert!(derive_seafloor_age(&mesh, &evolution, latest as usize).is_ok());
        assert_eq!(
            StageInputError::CrustBirthAfterRun.to_string(),
            "no crust can be born after the run's last step"
        );
    }

    #[test]
    fn validation_reports_misaligned_age_values() {
        let (mesh, _, _) = final_state_fixture();
        let age = SeafloorAge {
            cell_ages: vec![None; mesh.cell_count() - 1],
            diagnostics: Default::default(),
        };

        assert_eq!(age.validate(&mesh), Err(StageInputError::SeafloorAge));
        assert_eq!(
            StageInputError::SeafloorAge.to_string(),
            "seafloor-age values must match the mesh cell count"
        );
    }
}
