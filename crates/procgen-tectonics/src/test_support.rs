use procgen_core::Vec3;
pub use procgen_core::{fingerprint, quantized_fingerprint};
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, build_sphere_mesh};

use crate::field::DEFAULT_STEP_DURATION;
use crate::{
    BaseElevation, BaseElevationConfig, BoundaryClass, BoundaryClassification,
    BoundaryDeformationConfig, CellCrust, CrustBirthPrior, CrustBirthPriorConfig, CrustClass,
    CrustClassification, CrustClassificationConfig, CrustClassificationDiagnostics, FlowField,
    PlateEvolution, PlateEvolutionConfig, PlateEvolutionInputs, PlateKinematics,
    PlateKinematicsConfig, PlateLifecycleConfig, PlatePartition, PlatePartitionConfig,
    PoleDriftConfig, SeafloorAge, classify_boundaries, classify_crust, derive_base_elevation,
    derive_crust_birth_prior, derive_seafloor_age, evolve_plate_ownership,
    generate_plate_kinematics, partition_plates,
};

/// Model time per step scaled to the 512-cell reference mesh. Its cell width
/// is 0.157 against the default mesh's 0.0138, so a step here has to be about
/// eleven times longer to move a plate the same one cell.
pub const REFERENCE_STEP_DURATION: f32 = 0.15;

/// Kinematics seed of the reference fixtures, and so of their flow field.
const REFERENCE_MOTION_SEED: u64 = 7;

/// The motion config the reference fixtures fit against, and the one the crust
/// birth prior scales a hop by. The fixtures that build their kinematics by
/// hand turn their plates at unit speed, which is this config's maximum, so
/// they date their ocean by the same hop as the fitted ones.
fn reference_motion_config() -> PlateKinematicsConfig {
    PlateKinematicsConfig::new(REFERENCE_MOTION_SEED)
}

/// No pole drift, for the fixtures whose assertions are about what one fixed
/// motion does over several steps. Their steps are long enough that the
/// default rates would turn an axis a large fraction of a right angle each.
pub const NO_POLE_DRIFT: PoleDriftConfig = PoleDriftConfig {
    axis_drift_rate: 0.0,
    speed_drift_rate: 0.0,
    speed_drift_limit: 0.0,
};

/// No rifting and no suturing, for the fixtures whose assertions are about a
/// fixed plate set. A zero rift rate never draws and an infinite suture time
/// is never reached, so the remaining fields cannot be read.
pub const NO_LIFECYCLE: PlateLifecycleConfig = PlateLifecycleConfig {
    rift_rate: 0.0,
    rift_minimum_area_fraction: 0.0,
    rift_curvature: 0.0,
    rift_opening_speed: 0.0,
    suture_time: f32::INFINITY,
    suture_minimum_shared_edges: 0,
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

/// The flow field the reference fixtures' plates were fitted to, for the
/// stages downstream of a run that read the same field the fit did.
pub fn reference_flow_field() -> FlowField {
    FlowField::new(&PlateKinematicsConfig::new(REFERENCE_MOTION_SEED))
}

pub fn reference_evolution_config() -> PlateEvolutionConfig {
    let default = PlateEvolutionConfig::default();
    let time_scale = DEFAULT_STEP_DURATION / REFERENCE_STEP_DURATION;
    PlateEvolutionConfig {
        step_duration: REFERENCE_STEP_DURATION,
        pole_drift: PoleDriftConfig {
            // Drift rates are per unit root time, so the ratio that gives the
            // reference run the per-step wander the viewer's defaults produce
            // is the root of the ratio of the two steps.
            axis_drift_rate: default.pole_drift.axis_drift_rate * time_scale.sqrt(),
            speed_drift_rate: default.pole_drift.speed_drift_rate * time_scale.sqrt(),
            ..default.pole_drift
        },
        deformation: BoundaryDeformationConfig {
            // The whole reference run, so a boundary that converged throughout
            // reaches its full profile offset, as the viewer's defaults do.
            full_deformation_time: default.step_count as f32 * REFERENCE_STEP_DURATION,
            ..BoundaryDeformationConfig::default()
        },
        lifecycle: PlateLifecycleConfig {
            // A rate per unit time and a time, scaled by the same ratio as
            // the drift rates, so the reference run sees the events at about
            // the density the viewer's defaults produce.
            rift_rate: default.lifecycle.rift_rate * time_scale,
            // Roughly the share of the run the default eight steps are of the
            // viewer's fifteen, so a collision can still merge inside a
            // reference run of five.
            suture_time: 3.0 * REFERENCE_STEP_DURATION,
            // A collision front is a length, and an edge count for a fixed
            // area fraction goes as the square root of the cell count: this
            // mesh has an eighth of the default's cells per plate, so a front
            // here is about a tenth of the edges.
            suture_minimum_shared_edges: 2,
            ..default.lifecycle
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
    pub boundaries: BoundaryClassification,
    pub birth_prior: CrustBirthPrior,
}

impl EvolutionFixture {
    pub fn inputs(&self) -> PlateEvolutionInputs<'_> {
        PlateEvolutionInputs {
            partition: &self.partition,
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
    fixture_over_crust(reference_crust_config())
}

fn fixture_over_crust(crust_config: CrustClassificationConfig) -> EvolutionFixture {
    let (mesh, partition) = reference_partition();
    let crust = classify_crust(&mesh, crust_config).unwrap();
    let kinematics =
        generate_plate_kinematics(&mesh, &partition, &crust, reference_motion_config()).unwrap();
    let boundaries = classify_boundaries(&mesh, &partition, &kinematics).unwrap();
    let birth_prior = derive_crust_birth_prior(
        &mesh,
        &partition,
        &crust,
        reference_motion_config(),
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
    final_state_fixture_with_crust(reference_crust_config())
}

fn final_state_fixture_with_crust(
    crust_config: CrustClassificationConfig,
) -> (SphereMesh, CrustClassification, PlateEvolution) {
    let fixture = fixture_over_crust(crust_config);
    let evolution = fixture.evolve(reference_evolution_config());
    (fixture.mesh, fixture.crust, evolution)
}

/// A mesh, the reference world's final age and crust over it, and the default
/// flow field: everything base elevation and its interior relief terms read.
pub struct BaseElevationFixture {
    pub mesh: SphereMesh,
    pub age: SeafloorAge,
    pub cell_birth: Vec<Option<f32>>,
    pub flow: FlowField,
}

impl BaseElevationFixture {
    pub fn derive(&self, config: BaseElevationConfig) -> BaseElevation {
        derive_base_elevation(
            &self.mesh,
            &self.age,
            CellCrust {
                cell_birth: &self.cell_birth,
            },
            &self.flow,
            config,
        )
        .unwrap()
    }
}

pub fn base_elevation_fixture() -> BaseElevationFixture {
    base_elevation_fixture_with_crust(reference_crust_config())
}

/// The same over a chosen crust classification, for the field pinned at the
/// `continental_fraction` the margin taper retuned away from.
pub fn base_elevation_fixture_with_crust(
    crust_config: CrustClassificationConfig,
) -> BaseElevationFixture {
    let (mesh, _, evolution) = final_state_fixture_with_crust(crust_config);
    let age = derive_seafloor_age(&mesh, &evolution).unwrap();
    BaseElevationFixture {
        mesh,
        age,
        cell_birth: evolution.cell_birth,
        flow: reference_flow_field(),
    }
}

/// Stands in for a cell with no birth where a float fingerprint needs one
/// number per cell. It is six orders of magnitude outside any model time a run
/// produces, so it cannot collide with a real birth on the 1/1024 grid.
const NO_BIRTH_MARKER: f32 = 1.0e6;

/// Fingerprints a per-cell birth or age field, original continental crust
/// included.
pub fn birth_fingerprint(cell_birth: &[Option<f32>]) -> u64 {
    quantized_fingerprint(
        cell_birth
            .iter()
            .map(|birth| birth.unwrap_or(NO_BIRTH_MARKER)),
    )
}

/// The base-elevation config scaled to the reference world, the way
/// [`reference_evolution_config`] scales the run that feeds it.
///
/// A hop on the 512-cell mesh is eleven times the default mesh's, and the
/// reference step with it, so the ages that world reaches are eleven times the
/// ages the default world reaches and the age at which the floor is reached
/// has to scale with them. At the shipped default four fifths of the reference
/// ocean would sit flat on the deep floor and the curve these fixtures pin
/// would be a constant.
pub fn reference_base_elevation_config() -> BaseElevationConfig {
    let default = BaseElevationConfig::default();
    BaseElevationConfig {
        cooling_age: default.cooling_age * REFERENCE_STEP_DURATION / DEFAULT_STEP_DURATION,
        ..default
    }
}

/// Interior relief switched off, so that a continental cell stands at the
/// margin taper's answer and nothing else.
pub fn no_interior_relief() -> BaseElevationConfig {
    BaseElevationConfig {
        dynamic_topography_amplitude: 0.0,
        basement_amplitude: 0.0,
        ..reference_base_elevation_config()
    }
}

/// The reference world with every plate at rest, so that a run moves no
/// material at all and the only thing a step advances is the clock.
///
/// That is what lets one run be compared against the same run sliced twice as
/// finely: with nothing moving, which particle wins a cell and where a gap
/// opens can no longer differ between the two. The birth prior is untouched by
/// the freezing — it reads the configured plate speed, not the fitted plates —
/// so the fixture keeps a real spread of birth times for a run to age.
pub fn still_world_fixture() -> EvolutionFixture {
    let mut fixture = evolution_fixture();
    fixture.kinematics = PlateKinematics {
        angular_velocities: vec![Vec3::ZERO; fixture.partition.plate_count],
    };
    fixture.boundaries =
        classify_boundaries(&fixture.mesh, &fixture.partition, &fixture.kinematics).unwrap();
    fixture
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
    let crust = plate_crust(&partition, &plate_classes);
    let kinematics = opposed_kinematics(&mesh, edge, outward);
    let boundaries = classify_boundaries(&mesh, &partition, &kinematics).unwrap();
    let birth_prior = derive_crust_birth_prior(
        &mesh,
        &partition,
        &crust,
        reference_motion_config(),
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

/// A run on the two-plate fixture that isolates pole drift: the step is far
/// too short to move any material out of its own cell, so nothing but the
/// drifting motion can change what the boundaries are. The default rates over
/// a step this long turn an axis about fifteen degrees.
pub fn drift_config(step_count: usize) -> PlateEvolutionConfig {
    PlateEvolutionConfig {
        step_count,
        step_duration: 0.02,
        lifecycle: NO_LIFECYCLE,
        ..PlateEvolutionConfig::default()
    }
}

/// A two-plate world converging head on, and the step at which the
/// continental plate's material first crosses into the oceanic cell.
///
/// The one-cell oceanic plate lies inside a continental one, so the whole of
/// it is a trench: the continental material that arrives overrides it, and
/// the oceanic particle under it is what subduction takes.
pub fn convergent_fixture() -> (EvolutionFixture, PlateEvolutionConfig, usize) {
    let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental, CrustClass::Oceanic]);
    let config = PlateEvolutionConfig {
        step_count: 0,
        step_duration: 0.1,
        // The schedule below is the whole point of the fixture, and drift or
        // a split plate would change the speed it is computed from.
        pole_drift: NO_POLE_DRIFT,
        lifecycle: NO_LIFECYCLE,
        ..PlateEvolutionConfig::default()
    };
    let steps = (1..)
        .find(|&steps| {
            fixture
                .evolve(PlateEvolutionConfig {
                    step_count: steps,
                    ..config
                })
                .diagnostics
                .owner_change_count
                > 0
        })
        .expect("converging plates must eventually override a cell");
    assert!(steps > 1, "the crossing must take more than one step");
    (fixture, config, steps)
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
) -> Vec<Option<f32>> {
    partition
        .cell_plates
        .iter()
        .map(|&plate| (plate_classes[plate] == CrustClass::Oceanic).then_some(0.0))
        .collect()
}

/// A classification whose cells take their plate's class, for the fixtures
/// whose crust is chosen rather than grown. Its diagnostics are the default:
/// nothing in the pipeline reads them, only the viewer's summary.
pub fn plate_crust(
    partition: &PlatePartition,
    plate_classes: &[CrustClass],
) -> CrustClassification {
    CrustClassification {
        cell_classes: partition
            .cell_plates
            .iter()
            .map(|&plate| plate_classes[plate])
            .collect(),
        diagnostics: CrustClassificationDiagnostics::default(),
    }
}

/// Nucleus count scaled to the 512-cell reference mesh. The default eight
/// nuclei over the default mesh's 111 plates put a continent across about four
/// of them; this mesh has thirty-three plates, so three nuclei keep that ratio.
pub fn reference_crust_config() -> CrustClassificationConfig {
    CrustClassificationConfig {
        nucleus_count: 3,
        ..CrustClassificationConfig::new(17)
    }
}

/// A world whose continental plate is a large cap, and a run that rifts it on
/// its first step.
///
/// The cap is wide enough that a rift arc across it runs from one side of its
/// boundary to the other, and open enough that the arc does not close on
/// itself: a closed arc's wall has its mean center at the sphere's own, which
/// leaves the opening direction undefined. The draw always passes, and after
/// the split each half is below the minimum area, so the plate breaks up
/// exactly once. The step is long enough for the parting halves to travel a
/// cell width of this mesh in a few steps, so the ridge the rift opened has
/// time to make crust.
pub fn forced_rift_fixture() -> (EvolutionFixture, PlateEvolutionConfig) {
    let (mesh, partition, config) = rift_cap_world();
    let crust = plate_crust(&partition, &[CrustClass::Continental, CrustClass::Oceanic]);
    (fixture_over(mesh, partition, crust, at_rest(2)), config)
}

/// The same cap with an ocean over the half of it below the equator: the
/// plate's total area still clears `rift_minimum_area_fraction` and its
/// continental area no longer does, so the rift is never drawn for.
pub fn half_oceanic_rift_fixture() -> (EvolutionFixture, PlateEvolutionConfig) {
    let (mesh, partition, config) = rift_cap_world();
    let cell_classes = mesh
        .cell_centers
        .iter()
        .map(|center| match center.z > 0.0 {
            true => CrustClass::Continental,
            false => CrustClass::Oceanic,
        })
        .collect();
    let crust = CrustClassification {
        cell_classes,
        diagnostics: CrustClassificationDiagnostics::default(),
    };
    (fixture_over(mesh, partition, crust, at_rest(2)), config)
}

/// The cap world the two rift fixtures share: a plate of about three fifths
/// of the sphere beside one holding the rest, and a run that draws for a rift
/// every step.
fn rift_cap_world() -> (SphereMesh, PlatePartition, PlateEvolutionConfig) {
    let mesh = mesh(256);
    let partition = PlatePartition {
        cell_plates: mesh
            .cell_centers
            .iter()
            .map(|center| usize::from(center.z < CONTINENT_EDGE))
            .collect(),
        plate_count: 2,
    };
    let config = PlateEvolutionConfig {
        step_count: 6,
        step_duration: 0.25,
        pole_drift: NO_POLE_DRIFT,
        lifecycle: PlateLifecycleConfig {
            // A chance of two against a draw in [0, 1): certain.
            rift_rate: 8.0,
            // Only the whole cap passes, so neither half rifts again.
            rift_minimum_area_fraction: 0.5,
            // A great circle, so the arc stays in the plane the halves part
            // across and the boundary it leaves is a clean one.
            rift_curvature: 0.0,
            suture_time: f32::INFINITY,
            ..PlateLifecycleConfig::default()
        },
        ..PlateEvolutionConfig::default()
    };
    (mesh, partition, config)
}

/// Plates at rest, so a rift's opening term is the whole of the halves'
/// motion and the boundary it makes is the only one a run can open.
fn at_rest(plate_count: usize) -> PlateKinematics {
    PlateKinematics {
        angular_velocities: vec![Vec3::ZERO; plate_count],
    }
}

/// Height above which the forced-rift fixture's cap is continental: about
/// three fifths of the sphere, so an arc across it is long but open.
const CONTINENT_EDGE: f32 = -0.2;

/// A world whose only continental plate is a single cell, and a run that tries
/// to rift it every step. The arc leaves the plate on its first step, so the
/// wall is the whole plate and there is nothing for it to separate.
pub fn failed_rift_fixture() -> (EvolutionFixture, PlateEvolutionConfig) {
    let (mesh, _, partition) = two_plate_boundary_partition();
    let config = PlateEvolutionConfig {
        step_count: 1,
        step_duration: 0.1,
        pole_drift: NO_POLE_DRIFT,
        lifecycle: PlateLifecycleConfig {
            rift_rate: 20.0,
            // Any plate with area at all, so one cell is enough to be drawn.
            rift_minimum_area_fraction: 0.0,
            suture_time: f32::INFINITY,
            ..PlateLifecycleConfig::default()
        },
        ..PlateEvolutionConfig::default()
    };
    let crust = plate_crust(&partition, &[CrustClass::Oceanic, CrustClass::Continental]);
    (fixture_over(mesh, partition, crust, at_rest(2)), config)
}

/// A world of one continental plate covering the sphere, and a run that tries
/// to rift it. The arc meets nothing to stop it, so it closes on itself, and
/// its wall's mean center is the sphere's own: the halves have no direction to
/// part in.
pub fn closed_arc_rift_fixture() -> (EvolutionFixture, PlateEvolutionConfig) {
    let mesh = mesh(256);
    let partition = PlatePartition {
        cell_plates: vec![0; mesh.cell_count()],
        plate_count: 1,
    };
    let config = PlateEvolutionConfig {
        step_count: 1,
        step_duration: 0.1,
        pole_drift: NO_POLE_DRIFT,
        lifecycle: PlateLifecycleConfig {
            rift_rate: 20.0,
            rift_minimum_area_fraction: 0.5,
            rift_curvature: 0.0,
            suture_time: f32::INFINITY,
            ..PlateLifecycleConfig::default()
        },
        ..PlateEvolutionConfig::default()
    };
    let crust = plate_crust(&partition, &[CrustClass::Continental]);
    (fixture_over(mesh, partition, crust, at_rest(1)), config)
}

/// Two continental plates in sustained head-on convergence, and the run that
/// merges them. Returns the fixture, its config, and the step at which the
/// collision time reaches `suture_time`.
///
/// The step is far too short to move any material out of its own cell, so the
/// pair's collision is the only thing the run advances.
pub fn forced_suture_fixture() -> (EvolutionFixture, PlateEvolutionConfig, usize) {
    let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental; 2]);
    let steps = 3;
    let step_duration = 0.02;
    let config = PlateEvolutionConfig {
        step_count: steps,
        step_duration,
        pole_drift: NO_POLE_DRIFT,
        lifecycle: PlateLifecycleConfig {
            rift_rate: 0.0,
            // Half a step short of `steps` steps of collision, so the merge
            // lands on that step whatever the last bits of the sum do.
            suture_time: (steps as f32 - 0.5) * step_duration,
            // The one-cell plate shares five or six edges with its neighbour;
            // every convergent one of them counts.
            suture_minimum_shared_edges: 1,
            ..PlateLifecycleConfig::default()
        },
        ..PlateEvolutionConfig::default()
    };
    (fixture, config, steps)
}

/// The boundaries and birth prior a hand-built ownership, crust, and motion
/// imply, for the fixtures whose crust and motion are chosen rather than
/// grown and fitted.
fn fixture_over(
    mesh: SphereMesh,
    partition: PlatePartition,
    crust: CrustClassification,
    kinematics: PlateKinematics,
) -> EvolutionFixture {
    let boundaries = classify_boundaries(&mesh, &partition, &kinematics).unwrap();
    let birth_prior = derive_crust_birth_prior(
        &mesh,
        &partition,
        &crust,
        reference_motion_config(),
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
