//! Deterministic geological fields derived from completed tectonic state.
//!
//! This crate consumes tectonic ownership and motion without feeding data back
//! into tectonic generation. Hotspot fields are deliberately independent of
//! elevation and rendering.

mod hotspots;

pub use hotspots::{
    Hotspot, HotspotDiagnostics, HotspotField, HotspotFieldConfig, HotspotFieldError,
    HotspotTrailCell, generate_hotspot_field,
};
