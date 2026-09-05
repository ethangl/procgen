//! Deterministic geological fields derived from completed tectonic state.
//!
//! This crate consumes completed tectonic state without feeding data back into
//! tectonic generation. Geological fields never mutate tectonic elevation;
//! composition produces a separate elevation field. All stages remain
//! independent of rendering.

mod basins;
mod cratons;
mod elevation;
mod field;
mod hotspots;
mod isostasy;
mod oceanic_peaks;
mod volcanic_arcs;

pub use field::GeologyInputError;

pub use basins::{
    SedimentaryBasin, SedimentaryBasinDiagnostics, SedimentaryBasinField,
    SedimentaryBasinFieldConfig, SedimentaryBasinFieldError, derive_sedimentary_basin_field,
};
pub use cratons::{CratonDiagnostics, CratonField, CratonFieldConfig, derive_craton_field};
pub use elevation::{
    ElevationEffectDiagnostics, GeologicalElevation, GeologicalElevationConfig,
    GeologicalElevationDiagnostics, GeologicalElevationError, GeologicalElevationInputs,
    compose_geological_elevation,
};
pub use hotspots::{
    Hotspot, HotspotDiagnostics, HotspotField, HotspotFieldConfig, HotspotFieldError,
    HotspotTrailCell, generate_hotspot_field,
};
pub use isostasy::{
    IsostaticAdjustment, IsostaticAdjustmentConfig, IsostaticAdjustmentDiagnostics,
    IsostaticAdjustmentError, IsostaticAdjustmentInputs, derive_isostatic_adjustment,
};
pub use oceanic_peaks::{
    OceanicPeak, OceanicPeakDiagnostics, OceanicPeakField, OceanicPeakFieldConfig,
    OceanicPeakFieldError, OceanicPeakKind, derive_oceanic_peak_field,
};
pub use volcanic_arcs::{
    VolcanicArcCell, VolcanicArcDiagnostics, VolcanicArcField, VolcanicArcFieldConfig,
    VolcanicArcFieldError, VolcanicArcSegment, derive_volcanic_arc_field,
};
