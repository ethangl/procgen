use crate::{
    CratonDiagnostics, CratonField, HotspotDiagnostics, HotspotField, SedimentaryBasinDiagnostics,
    SedimentaryBasinField, VolcanicArcDiagnostics, VolcanicArcField,
};
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, build_sphere_mesh};
use procgen_tectonics::{CrustClass, CrustClassification};

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
