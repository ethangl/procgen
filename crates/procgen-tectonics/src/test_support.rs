pub use procgen_core::fingerprint;
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, build_sphere_mesh};

use crate::{
    BoundaryClass, BoundaryClassification, CrustBirthPrior, CrustBirthPriorConfig, CrustClass,
    CrustClassification, CrustClassificationConfig, PlateEvolution, PlateEvolutionConfig,
    PlateEvolutionInputs, PlateKinematics, PlateKinematicsConfig, PlatePartition,
    PlatePartitionConfig, classify_boundaries, classify_crust, derive_crust_birth_prior,
    evolve_plate_ownership, generate_plate_kinematics, partition_plates,
};

/// Model time per step scaled to the 512-cell reference mesh. Its cell width
/// is 0.157 against the default mesh's 0.0138, so a step here has to be about
/// eleven times longer to move a plate the same one cell.
pub const REFERENCE_STEP_DURATION: f32 = 0.15;

pub fn mesh(cell_count: usize) -> SphereMesh {
    build_sphere_mesh(
        fibonacci_sphere(FibonacciConfig {
            count: cell_count,
            jitter: 0.5,
            seed: 7,
        })
        .unwrap(),
        1.0,
    )
    .unwrap()
}

/// Arcs and minor-plate size scaled to the 512-cell reference mesh: enough
/// crack faces for merging to matter, splitting into plates of roughly
/// thirty-two cells.
pub fn reference_partition_config() -> PlatePartitionConfig {
    PlatePartitionConfig {
        arc_count: 20,
        piece_fraction: 32.0 / 512.0,
        growth_roughness: 0,
        seed: 7,
        ..PlatePartitionConfig::default()
    }
}

pub fn reference_partition() -> (SphereMesh, PlatePartition) {
    let mesh = mesh(512);
    let partition = partition_plates(&mesh, reference_partition_config()).unwrap();
    (mesh, partition)
}

pub fn reference_evolution_config() -> PlateEvolutionConfig {
    PlateEvolutionConfig {
        step_duration: REFERENCE_STEP_DURATION,
        ..PlateEvolutionConfig::default()
    }
}

/// Everything evolution reads, built once over the reference partition.
pub struct EvolutionFixture {
    pub mesh: SphereMesh,
    pub partition: PlatePartition,
    pub crust: CrustClassification,
    pub kinematics: PlateKinematics,
    pub boundaries: BoundaryClassification,
    pub birth_prior: CrustBirthPrior,
}

impl EvolutionFixture {
    pub fn inputs(&self) -> PlateEvolutionInputs<'_> {
        PlateEvolutionInputs {
            partition: &self.partition,
            crust: &self.crust,
            kinematics: &self.kinematics,
            boundaries: &self.boundaries,
            birth_prior: &self.birth_prior,
        }
    }

    pub fn evolve(&self, config: PlateEvolutionConfig) -> PlateEvolution {
        evolve_plate_ownership(&self.mesh, self.inputs(), config).unwrap()
    }
}

pub fn evolution_fixture() -> EvolutionFixture {
    let (mesh, partition) = reference_partition();
    let crust = classify_crust(&mesh, &partition, CrustClassificationConfig::new(17)).unwrap();
    let kinematics =
        generate_plate_kinematics(&mesh, &partition, &crust, PlateKinematicsConfig::new(7))
            .unwrap();
    let boundaries = classify_boundaries(&mesh, &partition, &kinematics).unwrap();
    let birth_prior = derive_crust_birth_prior(
        &mesh,
        &partition,
        &crust,
        &boundaries,
        CrustBirthPriorConfig::default(),
    )
    .unwrap();
    EvolutionFixture {
        mesh,
        partition,
        crust,
        kinematics,
        boundaries,
        birth_prior,
    }
}

/// The reference world after a default evolution run, for the stages that read
/// only its end state.
pub fn final_state_fixture() -> (SphereMesh, CrustClassification, PlateEvolution) {
    let fixture = evolution_fixture();
    let evolution = fixture.evolve(reference_evolution_config());
    (fixture.mesh, fixture.crust, evolution)
}

pub fn two_plate_boundary_partition() -> (SphereMesh, usize, PlatePartition) {
    let mesh = mesh(32);
    let edge_index = 0;
    let edge = mesh.edges[edge_index];
    let mut cell_plates = vec![0; mesh.cell_count()];
    cell_plates[edge.cells[1]] = 1;
    let partition = PlatePartition {
        cell_plates,
        plate_count: 2,
    };
    (mesh, edge_index, partition)
}

pub fn empty_boundaries(mesh: &SphereMesh) -> BoundaryClassification {
    BoundaryClassification {
        edge_classes: vec![BoundaryClass::Interior; mesh.edge_count()],
        edge_normal_speeds: vec![[0.0; 2]; mesh.edge_count()],
        edge_shear: vec![0.0; mesh.edge_count()],
    }
}

/// The per-cell birth field that reproduces plate crust classes cell by cell,
/// for stages tested on a hand-built partition rather than a real run.
pub fn plate_cell_birth(
    partition: &PlatePartition,
    plate_classes: &[CrustClass],
) -> Vec<Option<i32>> {
    partition
        .cell_plates
        .iter()
        .map(|&plate| (plate_classes[plate] == CrustClass::Oceanic).then_some(0))
        .collect()
}
