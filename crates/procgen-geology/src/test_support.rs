use crate::{
    CratonDiagnostics, CratonField, HotspotDiagnostics, HotspotField, SedimentaryBasinDiagnostics,
    SedimentaryBasinField, VolcanicArcDiagnostics, VolcanicArcField,
};
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, build_sphere_mesh, mean_cell_width};
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

/// The model length that spans `hops` on a mesh of `cell_count` cells. The
/// fixture meshes are far coarser than the 65,536-cell mesh the length
/// defaults were set against, so a fixture that means a hop count says so
/// through this.
pub(crate) fn hop_length(cell_count: usize, hops: usize) -> f32 {
    hops as f32 * mean_cell_width(1.0, cell_count)
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
