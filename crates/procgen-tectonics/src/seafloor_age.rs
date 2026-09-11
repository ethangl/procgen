//! Oceanic crust age at the two ends of evolution.
//!
//! Before step zero there is no history to read, so a prior stands in for one:
//! hop distance from the initial ridges within the initial owner, turned into
//! time and negated, is the model time at which each oceanic cell's crust
//! would have been created had the run started earlier. Evolution carries that
//! field forward, overriding crust at subduction zones and creating it at
//! ridges, and the final age is simply the model time elapsed since each
//! cell's birth. Continental crust has no age at either end.
//!
//! Everything here is model time rather than a count of steps or of hops. A
//! step and a mesh are how a run is sliced; an age is a fact about the crust,
//! and it must not change when the slicing does.

use crate::{
    BoundaryClass, BoundaryClassification, CrustClass, CrustClassification, FieldSummary,
    PlateEvolution, PlateKinematicsConfig, PlatePartition, field::mean_cell_width,
    stage::StageInputError,
};
use procgen_sphere_mesh::{SphereMesh, multi_source_distances};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrustBirthPriorConfig {
    /// Hop age before step zero given to every cell on an oceanic plate with
    /// no divergent boundary of its own.
    ///
    /// It stays a hop count because that is what it is: the walk it stands in
    /// for measures hops, and the prior turns hops into model time once, for
    /// this age and for every other, by the time the fastest plate takes to
    /// cross one cell.
    pub ridge_less_age: usize,
}

impl Default for CrustBirthPriorConfig {
    fn default() -> Self {
        Self { ridge_less_age: 8 }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CrustBirthPriorDiagnostics {
    /// Summary of the hop ages the prior turned into birth times.
    pub hops: FieldSummary,
    pub oceanic_cell_count: usize,
    pub ridge_cell_count: usize,
    pub ridge_plate_count: usize,
    /// Plates that own oceanic crust but no ridge of their own, whose cells
    /// all take the fallback age.
    pub ridge_less_plate_count: usize,
    pub fallback_cell_count: usize,
}

/// Per-cell crust birth before step zero, as model time. Every value is at or
/// below zero, because the prior describes crust the run did not make.
/// Continental cells have no birth.
#[derive(Clone, Debug, PartialEq)]
pub struct CrustBirthPrior {
    pub cell_birth: Vec<Option<f32>>,
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

/// The model time one hop of the prior's walk stands for: what a plate at the
/// configured maximum angular speed takes to cross one mean cell width.
///
/// The walk measures hops, and a hop is a length; the field it produces is a
/// time. This is the one place the two are related, and it is exactly the
/// relation [`crate::DEFAULT_STEP_DURATION`] was chosen by, so the prior dates
/// crust the same way on any mesh and at any step.
///
/// It reads the configured maximum rather than the fitted plates. The fit is
/// what the run starts from, not what made the crust the prior stands in for,
/// and a world whose fit happened to come out slow would otherwise re-date its
/// whole ocean. The maximum is validated positive where it is configured, so
/// this always has a speed to divide by.
fn hop_duration(mesh: &SphereMesh, kinematics: PlateKinematicsConfig) -> f32 {
    mean_cell_width(mesh.radius, mesh.cell_count())
        / (kinematics.maximum_angular_speed * mesh.radius)
}

/// Derives the crust birth evolution starts from, out of the initial ridges,
/// ownership, and the configured plate speed.
///
/// Oceanic cells touching a divergent edge are born at time zero. A
/// multi-source BFS propagates one hop of age per hop, only through oceanic
/// cells with the same initial plate owner, so a cell `h` hops from its
/// nearest ridge was born `h * hop_duration` before the run. Oceanic cells
/// whose plate has no ridge of its own receive the configured fallback age
/// uniformly. The motion config is read for that one number and nothing else,
/// so the prior does not depend on how the plates were fitted.
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
    kinematics: PlateKinematicsConfig,
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

    let hop = hop_duration(mesh, kinematics);
    Ok(CrustBirthPrior {
        cell_birth: cell_hops
            .iter()
            .map(|hops| hops.map(|hops| -(hops as f32) * hop))
            .collect(),
        diagnostics,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeafloorAgeDiagnostics {
    pub summary: FieldSummary,
    pub oceanic_cell_count: usize,
}

/// Per-cell seafloor age as the model time elapsed since the cell's crust was
/// created. Continental cells have no seafloor age.
#[derive(Clone, Debug, PartialEq)]
pub struct SeafloorAge {
    pub cell_ages: Vec<Option<f32>>,
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

/// Derives seafloor age from crust birth: the model time that has elapsed
/// since each cell's crust was created.
///
/// The run carries how long it lasted, so nothing outside has to keep a step
/// count in agreement with it; a birth after the run's end means the evolution
/// is not self-consistent and is rejected.
pub fn derive_seafloor_age(
    mesh: &SphereMesh,
    evolution: &PlateEvolution,
) -> Result<SeafloorAge, StageInputError> {
    evolution.validate(mesh)?;

    let elapsed = evolution.elapsed_time;
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
        .map(|birth| birth.map(|birth| elapsed - birth))
        .collect();
    let oceanic_ages: Vec<_> = cell_ages.iter().flatten().copied().collect();

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
        NO_LIFECYCLE, NO_POLE_DRIFT, birth_fingerprint, empty_boundaries, evolution_fixture,
        final_state_fixture, plate_crust, reference_base_elevation_config,
        reference_evolution_config, reference_flow_field, reference_partition, still_world_fixture,
    };
    use crate::{PlateEvolutionConfig, derive_base_elevation};

    /// A motion config whose fastest plate turns at unit speed, so that the hop
    /// duration is the mesh's own cell width and a test can state a birth in
    /// hops and read the field in model time.
    fn unit_speed_motion() -> PlateKinematicsConfig {
        PlateKinematicsConfig {
            maximum_angular_speed: 1.0,
            ..PlateKinematicsConfig::new(7)
        }
    }

    fn unit_hop(mesh: &SphereMesh) -> f32 {
        mean_cell_width(mesh.radius, mesh.cell_count()) / mesh.radius
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
            unit_speed_motion(),
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
                unit_speed_motion(),
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
            7_845_085_305_370_493_663
        );
        assert!(
            first.cell_birth.iter().flatten().all(|&birth| birth <= 0.0),
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
            unit_speed_motion(),
            &empty_boundaries(&mesh),
            CrustBirthPriorConfig { ridge_less_age: 13 },
        )
        .unwrap();

        for (cell, &plate) in partition.cell_plates.iter().enumerate() {
            assert_eq!(
                prior.cell_birth[cell],
                (plate != 0).then_some(-13.0 * unit_hop(&mesh)),
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
            unit_speed_motion(),
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
            unit_speed_motion(),
            &boundaries,
            CrustBirthPriorConfig { ridge_less_age: 23 },
        )
        .unwrap();

        let hop = unit_hop(&mesh);
        assert_eq!(prior.cell_birth[ridge_edge.cells[0]], Some(0.0));
        assert_eq!(prior.cell_birth[ridge_edge.cells[1]], Some(0.0));
        let same_plate_neighbor = mesh
            .cell_corners(ridge_edge.cells[0])
            .iter()
            .map(|corner| corner.neighbor)
            .find(|&neighbor| partition.cell_plates[neighbor] == ridge_plates[0])
            .unwrap();
        assert_eq!(prior.cell_birth[same_plate_neighbor], Some(-hop));
        for (cell, &plate) in partition.cell_plates.iter().enumerate() {
            if !ridge_plates.contains(&plate) {
                assert_eq!(prior.cell_birth[cell], Some(-23.0 * hop));
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
        let kinematics = unit_speed_motion();

        let mut wrong_partition = partition.clone();
        wrong_partition.cell_plates.pop();
        assert_eq!(
            derive_crust_birth_prior(
                &mesh,
                &wrong_partition,
                &crust,
                kinematics,
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
                kinematics,
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
                kinematics,
                &wrong_boundaries,
                Default::default()
            ),
            Err(StageInputError::Boundaries)
        );
    }

    #[test]
    fn final_age_measures_the_time_since_each_cell_was_born() {
        let (mesh, _, evolution) = final_state_fixture();
        let config = reference_evolution_config();
        let first = derive_seafloor_age(&mesh, &evolution).unwrap();

        assert_eq!(first, derive_seafloor_age(&mesh, &evolution).unwrap());
        assert_eq!(
            evolution.elapsed_time,
            config.step_count as f32 * config.step_duration
        );
        assert_eq!(first.cell_ages.len(), mesh.cell_count());
        for (cell, age) in first.cell_ages.iter().enumerate() {
            assert_eq!(
                age.is_some(),
                evolution.cell_crust().class(cell) == CrustClass::Oceanic
            );
            if let (Some(age), Some(birth)) = (age, evolution.cell_birth[cell]) {
                assert_eq!(*age, evolution.elapsed_time - birth);
            }
        }
        assert_eq!(
            first.diagnostics.oceanic_cell_count,
            first.cell_ages.iter().flatten().count()
        );
        assert_eq!(
            birth_fingerprint(&first.cell_ages),
            11_586_516_672_180_657_809
        );
    }

    #[test]
    fn an_elapsed_time_shorter_than_the_run_is_rejected() {
        let (mesh, _, evolution) = final_state_fixture();
        let latest = evolution
            .cell_birth
            .iter()
            .flatten()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(latest > 0.0, "the reference run creates crust");

        let short = PlateEvolution {
            elapsed_time: latest * 0.5,
            ..evolution.clone()
        };
        assert_eq!(
            derive_seafloor_age(&mesh, &short),
            Err(StageInputError::CrustBirthAfterRun)
        );
        let exact = PlateEvolution {
            elapsed_time: latest,
            ..evolution
        };
        assert!(derive_seafloor_age(&mesh, &exact).is_ok());
        assert_eq!(
            StageInputError::CrustBirthAfterRun.to_string(),
            "no crust can be born after the run's elapsed time"
        );
    }

    /// The exact statement that ages and the cooling curve mean model time.
    ///
    /// Nothing moves, so the two runs differ only in how the same span of time
    /// is sliced; halving a float and doubling a count are both exact, so the
    /// two elapsed times are the same number to the bit and every field
    /// measured against them is too. A moving world cannot make this claim:
    /// cells resolve twice as often, so which particle wins a contested cell
    /// and when a gap opens can differ.
    #[test]
    fn slicing_a_still_run_twice_as_finely_changes_nothing() {
        let fixture = still_world_fixture();
        let coarse = PlateEvolutionConfig {
            step_count: 6,
            pole_drift: NO_POLE_DRIFT,
            lifecycle: NO_LIFECYCLE,
            ..reference_evolution_config()
        };
        let fine = PlateEvolutionConfig {
            step_count: 2 * coarse.step_count,
            step_duration: coarse.step_duration / 2.0,
            ..coarse
        };

        let coarse_run = fixture.evolve(coarse);
        let fine_run = fixture.evolve(fine);
        assert_eq!(coarse_run.elapsed_time, fine_run.elapsed_time);
        assert_eq!(coarse_run.cell_birth, fine_run.cell_birth);

        let coarse_age = derive_seafloor_age(&fixture.mesh, &coarse_run).unwrap();
        let fine_age = derive_seafloor_age(&fixture.mesh, &fine_run).unwrap();
        assert_eq!(coarse_age, fine_age);
        let summary = coarse_age.diagnostics.summary;
        assert!(
            summary.minimum < summary.maximum,
            "the fixture must span a range of ages for this to say anything"
        );

        let base = |run: &PlateEvolution, age: &SeafloorAge| {
            derive_base_elevation(
                &fixture.mesh,
                age,
                run.cell_crust(),
                &reference_flow_field(),
                reference_base_elevation_config(),
            )
            .unwrap()
        };
        assert_eq!(base(&coarse_run, &coarse_age), base(&fine_run, &fine_age));
    }

    /// The prior reads no step duration at all, so nothing in a run's setup
    /// can scale the history it starts from by how the run is sliced.
    #[test]
    fn the_prior_does_not_depend_on_the_step_duration() {
        let fixture = evolution_fixture();
        let config = PlateEvolutionConfig {
            step_count: 0,
            ..reference_evolution_config()
        };
        let halved = PlateEvolutionConfig {
            step_duration: config.step_duration / 2.0,
            ..config
        };

        assert_eq!(
            fixture.evolve(config).cell_birth,
            fixture.birth_prior.cell_birth
        );
        assert_eq!(
            fixture.evolve(halved).cell_birth,
            fixture.birth_prior.cell_birth
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
