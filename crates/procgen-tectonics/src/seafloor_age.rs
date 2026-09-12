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
    PlateEvolution, PlateKinematicsConfig, PlateKinematicsError, PlatePartition,
    motion::validate_config as validate_motion_config, stage::StageInputError,
};
use procgen_sphere_mesh::{
    SphereMesh, default_hop_length, mean_cell_width, multi_source_distances,
};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrustBirthPriorConfig {
    /// Age before step zero given to every cell on an oceanic plate with no
    /// divergent boundary of its own.
    ///
    /// It is a model time, like every other age the pipeline carries. The hop
    /// distances the walk measures are geometry and scale with the mesh, but
    /// this is the age a floor is born with, which is a fact about the world:
    /// as a hop count it made that floor younger on a finer mesh. The default
    /// is what eight hops of the default mesh stand for at the unit speed,
    /// which is what it was.
    pub ridge_less_age: f32,
}

impl Default for CrustBirthPriorConfig {
    fn default() -> Self {
        Self {
            ridge_less_age: 8.0 * default_hop_length(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CrustBirthPriorDiagnostics {
    /// Summary of the ages the prior turned into birth times, in model time.
    /// The walk measures hops and the fallback is a time, so this is stated in
    /// the unit both end up in.
    pub age: FieldSummary,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrustBirthPriorError {
    Motion(PlateKinematicsError),
    Input(StageInputError),
}

impl fmt::Display for CrustBirthPriorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Motion(error) => error.fmt(formatter),
            Self::Input(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CrustBirthPriorError {}

impl From<PlateKinematicsError> for CrustBirthPriorError {
    fn from(error: PlateKinematicsError) -> Self {
        Self::Motion(error)
    }
}

impl From<StageInputError> for CrustBirthPriorError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
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
/// whole ocean. The caller holds the config to the kinematics stage's own
/// rules before this runs, so there is always a positive speed to divide by.
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
) -> Result<CrustBirthPrior, CrustBirthPriorError> {
    // The motion config is the one input here that is not a stage output, so
    // it is held to the rules of the stage that owns it rather than taken on
    // trust: a maximum speed of zero would otherwise divide a hop by nothing.
    validate_motion_config(kinematics)?;
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
    let cell_hops = multi_source_distances(mesh, &ridge_cells, |cell, neighbor| {
        partition.cell_plates[cell] == partition.cell_plates[neighbor] && oceanic(neighbor)
    });
    let ridge_cell_count = cell_hops.iter().filter(|&&hops| hops == Some(0)).count();

    // The walk's hops become a time here, at the one relation the prior
    // states; the fallback is already one.
    let hop = hop_duration(mesh, kinematics);
    let mut oceanic_plates = vec![false; partition.plate_count];
    let mut fallback_cell_count = 0;
    let mut cell_ages = vec![None; mesh.cell_count()];
    for (cell, hops) in cell_hops.iter().enumerate() {
        if !oceanic(cell) {
            continue;
        }
        oceanic_plates[partition.cell_plates[cell]] = true;
        cell_ages[cell] = Some(match hops {
            Some(hops) => *hops as f32 * hop,
            None => {
                fallback_cell_count += 1;
                config.ridge_less_age
            }
        });
    }
    let oceanic_ages: Vec<_> = cell_ages.iter().flatten().copied().collect();
    let ridge_plate_count = ridge_plates.iter().filter(|&&has_ridge| has_ridge).count();
    let diagnostics = CrustBirthPriorDiagnostics {
        age: FieldSummary::from_values(&oceanic_ages),
        oceanic_cell_count: oceanic_ages.len(),
        ridge_cell_count,
        ridge_plate_count,
        ridge_less_plate_count: oceanic_plates.iter().filter(|&&oceanic| oceanic).count()
            - ridge_plate_count,
        fallback_cell_count,
    };

    Ok(CrustBirthPrior {
        cell_birth: cell_ages.iter().map(|age| age.map(|age| -age)).collect(),
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
        reference_evolution_config, reference_flow_field, reference_partition,
        scaled_birth_prior_config, still_world_fixture,
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
        let config = scaled_birth_prior_config(mesh.cell_count());
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
        // The prior reads the fixture's boundaries, which the crust factor
        // alone now scales the motion behind: every plate keeps a ridge of its
        // own where one used to have none.
        // The world the prior describes is unchanged: the walk still reaches
        // nine hops and averages 1.7781065 of them over the same 338 cells.
        // What moved is the unit. The summary is in model time now, because
        // the fallback age is a time and no longer a hop count, so every value
        // is what it was times this mesh's hop duration.
        assert_eq!(
            first.diagnostics,
            CrustBirthPriorDiagnostics {
                age: FieldSummary {
                    minimum: 0.0,
                    maximum: 9.0 * unit_hop(&mesh),
                    mean: 0.278_565_76,
                },
                oceanic_cell_count: 338,
                ridge_cell_count: 102,
                ridge_plate_count: 30,
                ridge_less_plate_count: 0,
                // A plate can hold a ridge and still leave cells the walk
                // never reaches, in a piece of itself the ridge is not on.
                // Those take the fallback age too.
                fallback_cell_count: 6,
            }
        );
        assert_eq!(
            birth_fingerprint(&first.cell_birth),
            13_907_807_829_833_123_813
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
            CrustBirthPriorConfig {
                ridge_less_age: 13.0 * unit_hop(&mesh),
            },
        )
        .unwrap();

        for (cell, &plate) in partition.cell_plates.iter().enumerate() {
            assert_eq!(
                prior.cell_birth[cell],
                (plate != 0).then_some(-(13.0 * unit_hop(&mesh))),
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
            CrustBirthPriorConfig {
                ridge_less_age: 23.0 * unit_hop(&mesh),
            },
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
                assert_eq!(prior.cell_birth[cell], Some(-(23.0 * hop)));
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
            Err(CrustBirthPriorError::Input(StageInputError::Cells))
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
            Err(CrustBirthPriorError::Input(StageInputError::CrustClasses))
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
            Err(CrustBirthPriorError::Input(StageInputError::Boundaries))
        );

        // A hop is a cell width over a plate speed, so a world whose plates
        // cannot move has no hop to date crust by. The prior holds the config
        // to the kinematics stage's rule rather than dividing by nothing.
        let at_rest = PlateKinematicsConfig {
            minimum_angular_speed: 0.0,
            maximum_angular_speed: 0.0,
            ..kinematics
        };
        assert_eq!(
            derive_crust_birth_prior(
                &mesh,
                &partition,
                &crust,
                at_rest,
                &empty_boundaries(&mesh),
                Default::default()
            ),
            Err(CrustBirthPriorError::Motion(
                PlateKinematicsError::InvalidAngularSpeedRange
            ))
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
            16_070_050_445_168_748_665
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
