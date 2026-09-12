use crate::{
    CratonDiagnostics, CratonField, HotspotDiagnostics, HotspotField, HotspotFieldConfig,
    SedimentaryBasinDiagnostics, SedimentaryBasinField, VolcanicArcDiagnostics, VolcanicArcField,
};
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{DEFAULT_CELL_COUNT, SphereMesh, build_sphere_mesh, hop_length};
use procgen_tectonics::{
    BoundaryClass, BoundaryClassification, CellCrust, CrustClass, CrustClassification,
    CrustClassificationConfig, PlateKinematics, PlateKinematicsConfig, PlatePartition,
    PlatePartitionConfig, classify_boundaries, classify_crust, generate_plate_kinematics,
    partition_plates,
};

/// The normalized density that gives one cell of a mesh of `cell_count` cells
/// the chance a cell of the default mesh has at `per_default_cell`.
///
/// A density is per unit area, and these fixture meshes have cells hundreds of
/// times wider than the default mesh's, so a fixture that wants a draw its own
/// cells can express has to state a much smaller density than the default.
pub(crate) fn density_per_cell(cell_count: usize, per_default_cell: f32) -> f32 {
    per_default_cell * cell_count as f32 / DEFAULT_CELL_COUNT as f32
}

/// The mesh the hotspot and province pins run on. Its cells are far wider than
/// the default mesh's, so the configs below state their lengths as hops on it.
pub(crate) const HOTSPOT_CELL_COUNT: usize = 512;

/// A mesh with a plate set, the per-cell crust birth the crust class is read
/// from, and the plate motion: everything the present-day geology stages read
/// out of a finished tectonics run, without running one.
pub(crate) struct PlateFixture {
    pub(crate) mesh: SphereMesh,
    pub(crate) plates: PlatePartition,
    pub(crate) cell_birth: Vec<Option<f32>>,
    pub(crate) kinematics: PlateKinematics,
}

impl PlateFixture {
    pub(crate) fn new(cell_count: usize, arc_count: usize) -> Self {
        let mesh = build_sphere_mesh(
            fibonacci_sphere(FibonacciConfig {
                count: cell_count,
                jitter: 0.5,
                seed: 7,
            })
            .unwrap(),
            1.0,
        )
        .unwrap();
        let plates = partition_plates(
            &mesh,
            PlatePartitionConfig {
                arc_count,
                piece_fraction: 32.0 / cell_count as f32,
                growth_roughness: 0,
                seed: 11,
                ..PlatePartitionConfig::default()
            },
        )
        .unwrap();
        let crust = classify_crust(&mesh, CrustClassificationConfig::new(17)).unwrap();
        let kinematics =
            generate_plate_kinematics(&mesh, &plates, &crust, PlateKinematicsConfig::new(13))
                .unwrap();
        Self {
            cell_birth: classified_cell_birth(&crust),
            mesh,
            plates,
            kinematics,
        }
    }

    pub(crate) fn crust(&self) -> CellCrust<'_> {
        crust(&self.cell_birth)
    }

    pub(crate) fn boundaries(&self) -> BoundaryClassification {
        classify_boundaries(&self.mesh, &self.plates, &self.kinematics).unwrap()
    }
}

/// The world the hotspot and province pins are taken over.
pub(crate) fn hotspot_fixture(cell_count: usize) -> PlateFixture {
    PlateFixture::new(cell_count, 4)
}

/// No hotspot erupts a province, which is the field as it stood before
/// provinces existed. The lengths are the cells the defaults mean, on a mesh
/// far coarser than the one they were set against.
pub(crate) fn hotspot_reference_config() -> HotspotFieldConfig {
    HotspotFieldConfig {
        hotspot_count: 24,
        maximum_trail_length: hop_length(HOTSPOT_CELL_COUNT, 7.0),
        province_fraction: 0.0,
        province_radius: hop_length(HOTSPOT_CELL_COUNT, 5.0),
        province_rim: hop_length(HOTSPOT_CELL_COUNT, 2.0),
        ..HotspotFieldConfig::new(17)
    }
}

/// Every continental-source hotspot erupts, so the province geometry is
/// visible without depending on where the hashed draw happens to fall.
pub(crate) fn hotspot_province_config() -> HotspotFieldConfig {
    HotspotFieldConfig {
        province_fraction: 1.0,
        ..hotspot_reference_config()
    }
}

/// Two plates split at the equator, converging along the whole of their one
/// boundary: the smallest world with a single arc in it.
pub(crate) fn hemisphere_fixture(
    cell_count: usize,
) -> (SphereMesh, PlatePartition, BoundaryClassification) {
    let mesh = build_sphere_mesh(
        fibonacci_sphere(FibonacciConfig {
            count: cell_count,
            jitter: 0.5,
            seed: 7,
        })
        .unwrap(),
        1.0,
    )
    .unwrap();
    let cell_plates: Vec<_> = mesh
        .cell_centers
        .iter()
        .map(|center| usize::from(center.z < 0.0))
        .collect();
    let mut boundaries = BoundaryClassification {
        edge_classes: vec![BoundaryClass::Interior; mesh.edge_count()],
        edge_normal_speeds: vec![[0.0; 2]; mesh.edge_count()],
        edge_shear: vec![0.0; mesh.edge_count()],
    };
    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        if cell_plates[edge.cells[0]] != cell_plates[edge.cells[1]] {
            boundaries.edge_classes[edge_index] = BoundaryClass::Convergent;
            boundaries.edge_normal_speeds[edge_index] = [0.5, 0.5];
        }
    }
    let plates = PlatePartition {
        cell_plates,
        plate_count: 2,
    };
    (mesh, plates, boundaries)
}

/// One birth time per plate, for the fixtures that set a whole plate's crust
/// class at once.
pub(crate) fn plate_birth(
    plates: &PlatePartition,
    plate_births: [Option<f32>; 2],
) -> Vec<Option<f32>> {
    plates
        .cell_plates
        .iter()
        .map(|&plate| plate_births[plate])
        .collect()
}

pub(crate) fn crust(cell_birth: &[Option<f32>]) -> CellCrust<'_> {
    CellCrust { cell_birth }
}

/// The birth field an initial per-cell classification implies, for the stages
/// tested without running evolution: oceanic crust born at step zero,
/// continental crust that nothing has re-made.
pub(crate) fn classified_cell_birth(crust: &CrustClassification) -> Vec<Option<f32>> {
    crust
        .cell_classes
        .iter()
        .map(|&class| (class == CrustClass::Oceanic).then_some(0.0))
        .collect()
}

pub(crate) fn mesh(cell_count: usize) -> SphereMesh {
    build_sphere_mesh(
        fibonacci_sphere(FibonacciConfig::new(cell_count)).unwrap(),
        1.0,
    )
    .unwrap()
}

pub(crate) fn empty_hotspots(cell_count: usize) -> HotspotField {
    HotspotField {
        hotspots: Vec::new(),
        cell_intensities: vec![0.0; cell_count],
        cell_hotspots: vec![None; cell_count],
        cell_plateau: vec![0.0; cell_count],
        diagnostics: HotspotDiagnostics::default(),
    }
}

pub(crate) fn empty_volcanic_arcs(cell_count: usize) -> VolcanicArcField {
    VolcanicArcField {
        segments: Vec::new(),
        cell_strengths: vec![0.0; cell_count],
        cell_segments: vec![None; cell_count],
        diagnostics: VolcanicArcDiagnostics::default(),
    }
}

pub(crate) fn empty_cratons(cell_count: usize) -> CratonField {
    CratonField {
        cell_strengths: vec![0.0; cell_count],
        diagnostics: CratonDiagnostics::default(),
    }
}

pub(crate) fn empty_basins(cell_count: usize) -> SedimentaryBasinField {
    SedimentaryBasinField {
        cell_basins: vec![None; cell_count],
        basins: Vec::new(),
        diagnostics: SedimentaryBasinDiagnostics::default(),
    }
}
