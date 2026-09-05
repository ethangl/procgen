//! Deterministic geological fields derived from completed tectonic state.
//!
//! This crate consumes completed tectonic state without feeding data back into
//! tectonic generation. Geological fields never mutate elevation and remain
//! independent of rendering.

mod cratons;
mod field;
mod hotspots;
mod volcanic_arcs;

pub use cratons::{CratonDiagnostics, CratonField, CratonFieldConfig, derive_craton_field};
pub use hotspots::{
    Hotspot, HotspotDiagnostics, HotspotField, HotspotFieldConfig, HotspotFieldError,
    HotspotTrailCell, generate_hotspot_field,
};
pub use volcanic_arcs::{
    VolcanicArcCell, VolcanicArcDiagnostics, VolcanicArcField, VolcanicArcFieldConfig,
    VolcanicArcFieldError, VolcanicArcSegment, derive_volcanic_arc_field,
};
