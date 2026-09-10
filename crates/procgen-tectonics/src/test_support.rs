pub use procgen_core::fingerprint;
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, build_sphere_mesh};

use crate::field::DEFAULT_STEP_DURATION;
use crate::{
    BoundaryClass, BoundaryClassification, BoundaryDeformationConfig, CrustBirthPrior,
    CrustBirthPriorConfig, CrustClass, CrustClassification, CrustClassificationConfig,
    PlateEvolution, PlateEvolutionConfig, PlateEvolutionInputs, PlateKinematics,
    PlateKinematicsConfig, PlateMigrationConfig, PlatePartition, PlatePartitionConfig,
    PoleDriftConfig, classify_boundaries, classify_crust, derive_crust_birth_prior,
    evolve_plate_ownership, generate_plate_kinematics, partition_plates,
};

/// Model time per step scaled to the 512-cell reference mesh. Its cell width
/// is 0.157 against the default mesh's 0.0138, so a step here has to be about
/// eleven times longer to move a plate the same one cell.
pub const REFERENCE_STEP_DURATION: f32 = 0.15;

/// No pole drift, for the fixtures whose assertions are about what one fixed
/// motion does over several steps. Their steps are long enough that the
/// default rates would turn an axis a large fraction of a right angle each.
pub const NO_POLE_DRIFT: PoleDriftConfig = PoleDriftConfig {
    axis_drift_rate: 0.0,
    speed_drift_rate: 0.0,
};

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
    let default = PlateEvolutionConfig::default();
    let time_scale = DEFAULT_STEP_DURATION / REFERENCE_STEP_DURATION;
    PlateEvolutionConfig {
        step_duration: REFERENCE_STEP_DURATION,
        pole_drift: PoleDriftConfig {
            // Drift rates are per unit time and this step is eleven times the
            // default one, so scaling them by the same ratio gives the
            // reference run the per-step wander the viewer's defaults produce.
            axis_drift_rate: default.pole_drift.axis_drift_rate * time_scale,
            speed_drift_rate: default.pole_drift.speed_drift_rate * time_scale,
        },
        deformation: BoundaryDeformationConfig {
            // The whole reference run, so a boundary that converged throughout
            // reaches its full profile offset, as the viewer's defaults do.
            full_deformation_time: default.step_count as f32 * REFERENCE_STEP_DURATION,
            ..BoundaryDeformationConfig::default()
        },
        ..default
    }
}

/// Everything evolution reads, built once over the reference partition.
pub struct EvolutionFixture {
    pub mesh: SphereMesh,
    pub partition: PlatePartition,
    pub crust: CrustClassification,
    pub kinematics: PlateKinematics,
    /// The bounds a run's drift clamps back into. Fixtures whose rotations are
    /// hand-built rather than fitted still need a band to stay inside.
    pub kinematics_config: PlateKinematicsConfig,
    pub boundaries: BoundaryClassification,
    pub birth_prior: CrustBirthPrior,
}

impl EvolutionFixture {
    pub fn inputs(&self) -> PlateEvolutionInputs<'_> {
        PlateEvolutionInputs {
            partition: &self.partition,
            crust: &self.crust,
            kinematics: &self.kinematics,
            kinematics_config: self.kinematics_config,
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
    let kinematics_config = PlateKinematicsConfig::new(7);
    let kinematics =
        generate_plate_kinematics(&mesh, &partition, &crust, kinematics_config).unwrap();
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
        kinematics_config,
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

/// Rigid rotations that carry the two cells of `edge` along their own
/// separation direction at unit speed: `outward` of one pulls them apart
/// and minus one pushes them together. `omega = p x d` is the rotation
/// whose velocity at `p` is the tangential part of `d`.
pub fn opposed_kinematics(mesh: &SphereMesh, edge: usize, outward: f32) -> PlateKinematics {
    let [first, second] = mesh.edges[edge].cells.map(|cell| mesh.cell_centers[cell]);
    let apart = (second - first).normalized() * outward;
    PlateKinematics {
        angular_velocities: vec![first.cross(-apart), second.cross(apart)],
    }
}

/// A one-cell plate inside a larger one, moving apart from or into it.
pub fn two_plate_fixture(outward: f32, plate_classes: Vec<CrustClass>) -> EvolutionFixture {
    let (mesh, edge, partition) = two_plate_boundary_partition();
    let crust = CrustClassification { plate_classes };
    let kinematics = opposed_kinematics(&mesh, edge, outward);
    // `opposed_kinematics` builds unit-speed rotations, so the band has to
    // reach one for a drifted speed to have anywhere to go.
    let kinematics_config = PlateKinematicsConfig {
        minimum_angular_speed: 0.0,
        maximum_angular_speed: 1.0,
        ..PlateKinematicsConfig::new(7)
    };
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
        kinematics_config,
        boundaries,
        birth_prior,
    }
}

/// A step long enough to travel one cell width on the 32-cell mesh, with
/// migration off: the one-cell plate the rifting fixture opens from would
/// otherwise be swallowed by its own convergent edge before it opened
/// anything.
pub fn rift_config(step_count: usize) -> PlateEvolutionConfig {
    PlateEvolutionConfig {
        step_count,
        step_duration: 0.7,
        migration: PlateMigrationConfig {
            minimum_convergence: f32::MAX,
        },
        pole_drift: NO_POLE_DRIFT,
        ..PlateEvolutionConfig::default()
    }
}

/// A run on the two-plate fixture that isolates pole drift: migration is off
/// and the step is far too short for a cell to travel a cell width, so
/// nothing but the drifting motion can change what the boundaries are. The
/// default rates over a step this long turn an axis about nine degrees.
pub fn drift_config(step_count: usize) -> PlateEvolutionConfig {
    PlateEvolutionConfig {
        step_count,
        step_duration: 0.02,
        migration: PlateMigrationConfig {
            minimum_convergence: f32::MAX,
        },
        ..PlateEvolutionConfig::default()
    }
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
