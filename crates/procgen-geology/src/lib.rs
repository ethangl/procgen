//! Deterministic geological fields derived from completed tectonic state.
//!
//! This crate consumes completed tectonic state without feeding data back into
//! tectonic generation. Geological fields never mutate elevation and remain
//! independent of rendering.

mod basins;
mod cratons;
mod field;
mod hotspots;
mod oceanic_peaks;
mod volcanic_arcs;

pub use basins::{
    SedimentaryBasin, SedimentaryBasinDiagnostics, SedimentaryBasinField,
    SedimentaryBasinFieldConfig, SedimentaryBasinFieldError, derive_sedimentary_basin_field,
};
pub use cratons::{CratonDiagnostics, CratonField, CratonFieldConfig, derive_craton_field};
pub use hotspots::{
    Hotspot, HotspotDiagnostics, HotspotField, HotspotFieldConfig, HotspotFieldError,
    HotspotFieldInputError, HotspotTrailCell, generate_hotspot_field,
};
pub use oceanic_peaks::{
    OceanicPeak, OceanicPeakDiagnostics, OceanicPeakField, OceanicPeakFieldConfig,
    OceanicPeakFieldError, OceanicPeakKind, derive_oceanic_peak_field,
};
pub use volcanic_arcs::{
    VolcanicArcCell, VolcanicArcDiagnostics, VolcanicArcField, VolcanicArcFieldConfig,
    VolcanicArcFieldError, VolcanicArcSegment, derive_volcanic_arc_field,
};
