//! Deterministic geological fields derived from completed tectonic state.
//!
//! This crate consumes completed tectonic state without feeding data back into
//! tectonic generation. Geological fields are deliberately independent of
//! elevation and rendering.

mod field;
mod hotspots;
mod volcanic_arcs;

pub use hotspots::{
    Hotspot, HotspotDiagnostics, HotspotField, HotspotFieldConfig, HotspotFieldError,
    HotspotTrailCell, generate_hotspot_field,
};
pub use volcanic_arcs::{
    VolcanicArcCell, VolcanicArcDiagnostics, VolcanicArcField, VolcanicArcFieldConfig,
    VolcanicArcFieldError, VolcanicArcSegment, VolcanicPeakCandidate, derive_volcanic_arc_field,
};
