use super::{GenerationTimings, WORLD_RADIUS};
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, SphericalDelaunay};
use procgen_tectonics::{
    BaseElevation, BaseElevationConfig, BoundaryClassification, BoundaryDeformation,
    BoundaryDeformationConfig, CellCrust, CoarseElevation, CoarseElevationConfig, CrustBirthPrior,
    CrustBirthPriorConfig, CrustBirthPriorDiagnostics, CrustClassification,
    CrustClassificationConfig, PlateEvolution, PlateEvolutionConfig, PlateEvolutionDiagnostics,
    PlateEvolutionInputs, PlateKinematics, PlateKinematicsConfig, PlatePartition,
    PlatePartitionConfig, SeafloorAge, classify_boundaries, classify_crust,
    compose_coarse_elevation, derive_base_elevation, derive_boundary_deformation,
    derive_crust_birth_prior, derive_seafloor_age, evolve_plate_ownership,
    generate_plate_kinematics, partition_plates,
};
use std::error::Error;

/// Settings for the tectonics phase, which owns the sphere mesh every later
/// phase reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TectonicsSettings {
    pub fibonacci: FibonacciConfig,
    pub plates: PlatePartitionConfig,
    pub crust: CrustClassificationConfig,
    pub kinematics: PlateKinematicsConfig,
    pub birth_prior: CrustBirthPriorConfig,
    pub evolution: PlateEvolutionConfig,
    pub base_elevation: BaseElevationConfig,
    pub deformation: BoundaryDeformationConfig,
    pub elevation: CoarseElevationConfig,
}

impl Default for TectonicsSettings {
    fn default() -> Self {
        Self {
            fibonacci: FibonacciConfig {
                jitter: 0.5,
                seed: 7,
                ..FibonacciConfig::new(65_536)
            },
            plates: PlatePartitionConfig {
                seed: 7,
                ..PlatePartitionConfig::default()
            },
            crust: CrustClassificationConfig {
                target_ocean_fraction: 0.75,
                ..CrustClassificationConfig::new(7)
            },
            kinematics: PlateKinematicsConfig::new(7),
            birth_prior: CrustBirthPriorConfig::default(),
            evolution: PlateEvolutionConfig {
                step_count: 9,
                ..Default::default()
            },
            base_elevation: BaseElevationConfig::default(),
            deformation: BoundaryDeformationConfig::default(),
            elevation: CoarseElevationConfig::default(),
        }
    }
}

/// Results of the tectonics phase.
pub struct TectonicsWorld {
    pub voronoi: SphereMesh,
    pub plates: PlatePartition,
    pub crust: CrustClassification,
    pub kinematics: PlateKinematics,
    pub boundaries: BoundaryClassification,
    /// Step at which each cell's crust was created; the only per-cell answer
    /// to what crust a cell carries.
    pub cell_birth: Vec<Option<i32>>,
    pub birth_prior: CrustBirthPriorDiagnostics,
    pub evolution: PlateEvolutionDiagnostics,
    pub seafloor_age: SeafloorAge,
    pub base_elevation: BaseElevation,
    pub deformation: BoundaryDeformation,
    pub elevation: CoarseElevation,
    pub timings: GenerationTimings,
    pub config: TectonicsSettings,
}

/// Samples and triangulates the sphere mesh the tectonics phase and every
/// later phase read, recording its three stages into `timings`.
pub fn build_mesh(
    config: FibonacciConfig,
    timings: &mut GenerationTimings,
) -> Result<SphereMesh, Box<dyn Error>> {
    let points = timings.record("Sampling", || fibonacci_sphere(config))?;
    let delaunay = timings.record("Delaunay", || SphericalDelaunay::build(points))?;
    let voronoi = timings.record("Voronoi", || {
        SphereMesh::from_delaunay(&delaunay, WORLD_RADIUS)
    })?;
    Ok(voronoi)
}

impl TectonicsWorld {
    /// Generates the phase over `voronoi`, appending its stages to `timings`.
    pub fn generate(
        voronoi: SphereMesh,
        config: TectonicsSettings,
        mut timings: GenerationTimings,
    ) -> Result<Self, Box<dyn Error>> {
        let initial_plates = timings.record("Plate partition", || {
            partition_plates(&voronoi, config.plates)
        })?;
        let crust = timings.record("Crust", || {
            classify_crust(&voronoi, &initial_plates, config.crust)
        })?;
        let kinematics = timings.record("Plate kinematics", || {
            generate_plate_kinematics(&voronoi, &initial_plates, &crust, config.kinematics)
        })?;
        let initial_boundaries = timings.record("Boundary classification", || {
            classify_boundaries(&voronoi, &initial_plates, &kinematics)
        })?;
        let birth_prior = timings.record("Crust birth prior", || {
            derive_crust_birth_prior(
                &voronoi,
                &initial_plates,
                &crust,
                &initial_boundaries,
                config.birth_prior,
            )
        })?;
        let evolution_result = timings.record("Plate evolution", || {
            evolve_plate_ownership(
                &voronoi,
                PlateEvolutionInputs {
                    partition: &initial_plates,
                    crust: &crust,
                    kinematics: &kinematics,
                    boundaries: &initial_boundaries,
                    birth_prior: &birth_prior,
                },
                config.evolution,
            )
        })?;
        let seafloor_age = timings.record("Seafloor age", || {
            derive_seafloor_age(&voronoi, &evolution_result, config.evolution.step_count)
        })?;
        let PlateEvolution {
            partition: plates,
            boundaries,
            cell_birth,
            diagnostics: evolution,
        } = evolution_result;
        let CrustBirthPrior {
            diagnostics: birth_prior,
            ..
        } = birth_prior;
        let base_elevation = timings.record("Base elevation", || {
            derive_base_elevation(&seafloor_age, config.base_elevation)
        })?;
        let deformation = timings.record("Boundary deformation", || {
            derive_boundary_deformation(
                &voronoi,
                &plates,
                CellCrust {
                    cell_birth: &cell_birth,
                },
                &boundaries,
                config.deformation,
            )
        })?;
        let elevation = timings.record("Tectonic elevation", || {
            compose_coarse_elevation(&voronoi, &base_elevation, &deformation, config.elevation)
        })?;

        Ok(Self {
            voronoi,
            plates,
            crust,
            kinematics,
            boundaries,
            cell_birth,
            birth_prior,
            evolution,
            seafloor_age,
            base_elevation,
            deformation,
            elevation,
            timings,
            config,
        })
    }

    /// Per-cell crust after evolution. Plate crust classes describe plates,
    /// not the cells they currently own.
    pub fn cell_crust(&self) -> CellCrust<'_> {
        CellCrust {
            cell_birth: &self.cell_birth,
        }
    }

    pub fn validate(&self) -> Result<(), Box<dyn Error>> {
        let mesh = &self.voronoi;
        if self.config.fibonacci.count != mesh.cell_count() {
            return Err("tectonics results do not match their settings".into());
        }

        mesh.validate()?;
        self.plates.validate(mesh)?;
        self.crust.validate(&self.plates)?;
        self.kinematics.validate(&self.plates)?;
        self.boundaries.validate(mesh)?;
        self.cell_crust().validate(mesh)?;
        self.seafloor_age.validate(mesh)?;
        self.base_elevation.validate(mesh)?;
        self.deformation.validate(mesh)?;
        self.elevation.validate(mesh)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::{tectonics_settings, tectonics_world};
    use procgen_tectonics::CrustClass;

    #[test]
    fn tectonics_runs_without_the_later_phases() {
        let world = tectonics_world(tectonics_settings(128, 7));

        world.validate().unwrap();
        assert!(world.crust.plate_count(CrustClass::Oceanic) > 0);
        assert!(world.crust.plate_count(CrustClass::Continental) > 0);
        assert!(world.evolution.migrated_cell_count > 0);
        assert!(world.seafloor_age.diagnostics.oceanic_cell_count > 0);
        assert!(world.deformation.diagnostics.affected_cell_count() > 0);
        assert!(world.elevation.diagnostics.minimum >= 0.0);
        assert!(world.elevation.diagnostics.maximum <= 1.0);
        assert!(
            world
                .timings
                .stages()
                .iter()
                .any(|stage| stage.label == "Tectonic elevation")
        );
    }
}
