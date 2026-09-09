use crate::{
    CratonDiagnostics, CratonField, HotspotDiagnostics, HotspotField, SedimentaryBasinDiagnostics,
    SedimentaryBasinField, VolcanicArcDiagnostics, VolcanicArcField,
};
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, build_sphere_mesh};
use procgen_tectonics::{CrustClass, PlatePartition};

/// The per-cell birth field that reproduces plate crust classes cell by cell,
/// for stages tested on a hand-built partition rather than a real run.
pub(crate) fn plate_cell_birth(
    plates: &PlatePartition,
    plate_classes: &[CrustClass],
) -> Vec<Option<i32>> {
    plates
        .cell_plates
        .iter()
        .map(|&plate| (plate_classes[plate] == CrustClass::Oceanic).then_some(0))
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
