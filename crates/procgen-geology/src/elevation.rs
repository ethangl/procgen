use crate::{CratonField, HotspotField, SedimentaryBasinField, VolcanicArcField};
use procgen_tectonics::{CoarseElevation, FieldSummary};
use std::fmt;

/// Configuration for the ordered coarse geological-elevation composition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeologicalElevationConfig {
    /// Maximum normalized uplift contributed by a full-strength hotspot.
    pub hotspot_uplift: f32,
    /// Maximum normalized uplift contributed by a full-strength volcanic arc.
    pub volcanic_arc_uplift: f32,
    /// Fraction of the distance toward `continental_base` applied at full craton strength.
    pub craton_flattening: f32,
    /// Fraction of the distance toward a basin's component floor.
    pub basin_flattening: f32,
}

impl Default for GeologicalElevationConfig {
    fn default() -> Self {
        Self {
            hotspot_uplift: 0.08,
            volcanic_arc_uplift: 0.12,
            craton_flattening: 0.5,
            basin_flattening: 0.65,
        }
    }
}

/// Aggregate change produced by one composition effect.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ElevationEffectDiagnostics {
    pub affected_cell_count: usize,
    /// Signed sum of the effect's actual per-cell changes after clamping.
    pub total_delta: f64,
    pub maximum_absolute_delta: f32,
}

impl ElevationEffectDiagnostics {
    fn record(&mut self, before: f32, after: f32) {
        let delta = after - before;
        if delta != 0.0 {
            self.affected_cell_count += 1;
            self.total_delta += f64::from(delta);
            self.maximum_absolute_delta = self.maximum_absolute_delta.max(delta.abs());
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GeologicalElevationDiagnostics {
    pub elevation: FieldSummary,
    pub hotspots: ElevationEffectDiagnostics,
    pub volcanic_arcs: ElevationEffectDiagnostics,
    pub cratons: ElevationEffectDiagnostics,
    pub basins: ElevationEffectDiagnostics,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeologicalElevation {
    pub cell_elevations: Vec<f32>,
    pub diagnostics: GeologicalElevationDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeologicalElevationError {
    InvalidConfig,
    FieldCountMismatch,
    InvalidBasinId,
}

impl fmt::Display for GeologicalElevationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter
                .write_str("geological elevation values must be finite and between zero and one"),
            Self::FieldCountMismatch => formatter.write_str(
                "tectonic elevation and geological fields must have the same cell count",
            ),
            Self::InvalidBasinId => {
                formatter.write_str("basin cell ownership must index an existing basin")
            }
        }
    }
}

impl std::error::Error for GeologicalElevationError {}

/// Composes a new normalized coarse elevation field in this stable order:
/// hotspot uplift, volcanic-arc uplift, craton flattening, then basin flattening.
///
/// Basin floors are the deterministic component minima captured by the basin
/// field from the input tectonic elevation. No input field is modified, and
/// sparse oceanic peaks are deliberately not consumed by this stage.
pub fn compose_geological_elevation(
    tectonic_elevation: &CoarseElevation,
    hotspots: &HotspotField,
    volcanic_arcs: &VolcanicArcField,
    cratons: &CratonField,
    basins: &SedimentaryBasinField,
    continental_base: f32,
    config: GeologicalElevationConfig,
) -> Result<GeologicalElevation, GeologicalElevationError> {
    validate_inputs(
        tectonic_elevation,
        hotspots,
        volcanic_arcs,
        cratons,
        basins,
        continental_base,
        config,
    )?;

    let mut cell_elevations = tectonic_elevation.cell_elevations.clone();
    let mut diagnostics = GeologicalElevationDiagnostics::default();

    apply(
        &mut cell_elevations,
        &mut diagnostics.hotspots,
        |cell, elevation| {
            (elevation + hotspots.cell_intensities[cell] * config.hotspot_uplift).clamp(0.0, 1.0)
        },
    );
    apply(
        &mut cell_elevations,
        &mut diagnostics.volcanic_arcs,
        |cell, elevation| {
            (elevation + volcanic_arcs.cell_strengths[cell] * config.volcanic_arc_uplift)
                .clamp(0.0, 1.0)
        },
    );
    apply(
        &mut cell_elevations,
        &mut diagnostics.cratons,
        |cell, elevation| {
            lerp(
                elevation,
                continental_base,
                cratons.cell_strengths[cell] * config.craton_flattening,
            )
        },
    );
    apply(
        &mut cell_elevations,
        &mut diagnostics.basins,
        |cell, elevation| {
            basins.cell_basins[cell].map_or(elevation, |basin| {
                lerp(
                    elevation,
                    basins.basins[basin].minimum_elevation,
                    config.basin_flattening,
                )
            })
        },
    );

    diagnostics.elevation = FieldSummary::from_values(&cell_elevations);
    Ok(GeologicalElevation {
        cell_elevations,
        diagnostics,
    })
}

fn apply(
    elevations: &mut [f32],
    diagnostics: &mut ElevationEffectDiagnostics,
    mut effect: impl FnMut(usize, f32) -> f32,
) {
    for (cell, elevation) in elevations.iter_mut().enumerate() {
        let before = *elevation;
        *elevation = effect(cell, before);
        diagnostics.record(before, *elevation);
    }
}

fn lerp(value: f32, target: f32, amount: f32) -> f32 {
    value + (target - value) * amount
}

fn validate_inputs(
    tectonic_elevation: &CoarseElevation,
    hotspots: &HotspotField,
    volcanic_arcs: &VolcanicArcField,
    cratons: &CratonField,
    basins: &SedimentaryBasinField,
    continental_base: f32,
    config: GeologicalElevationConfig,
) -> Result<(), GeologicalElevationError> {
    let unit_interval = |value: f32| value.is_finite() && (0.0..=1.0).contains(&value);
    if !unit_interval(config.hotspot_uplift)
        || !unit_interval(config.volcanic_arc_uplift)
        || !unit_interval(config.craton_flattening)
        || !unit_interval(config.basin_flattening)
        || !unit_interval(continental_base)
    {
        return Err(GeologicalElevationError::InvalidConfig);
    }

    let cell_count = tectonic_elevation.cell_elevations.len();
    if hotspots.cell_intensities.len() != cell_count
        || volcanic_arcs.cell_strengths.len() != cell_count
        || cratons.cell_strengths.len() != cell_count
        || basins.cell_basins.len() != cell_count
    {
        return Err(GeologicalElevationError::FieldCountMismatch);
    }
    if basins
        .cell_basins
        .iter()
        .flatten()
        .any(|&basin| basin >= basins.basins.len())
    {
        return Err(GeologicalElevationError::InvalidBasinId);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CratonDiagnostics, HotspotDiagnostics, SedimentaryBasin, SedimentaryBasinDiagnostics,
        VolcanicArcDiagnostics,
    };
    use procgen_core::fingerprint;

    fn fields(
        elevations: Vec<f32>,
        hotspot_strengths: Vec<f32>,
        arc_strengths: Vec<f32>,
        craton_strengths: Vec<f32>,
        cell_basins: Vec<Option<usize>>,
        basins: Vec<SedimentaryBasin>,
    ) -> (
        CoarseElevation,
        HotspotField,
        VolcanicArcField,
        CratonField,
        SedimentaryBasinField,
    ) {
        (
            CoarseElevation {
                cell_elevations: elevations,
                diagnostics: Default::default(),
            },
            HotspotField {
                hotspots: Vec::new(),
                cell_hotspots: hotspot_strengths
                    .iter()
                    .map(|&strength| (strength > 0.0).then_some(0))
                    .collect(),
                cell_intensities: hotspot_strengths,
                diagnostics: HotspotDiagnostics::default(),
            },
            VolcanicArcField {
                segments: Vec::new(),
                cell_segments: arc_strengths
                    .iter()
                    .map(|&strength| (strength > 0.0).then_some(0))
                    .collect(),
                cell_strengths: arc_strengths,
                diagnostics: VolcanicArcDiagnostics::default(),
            },
            CratonField {
                cell_strengths: craton_strengths,
                diagnostics: CratonDiagnostics::default(),
            },
            SedimentaryBasinField {
                cell_basins,
                basins,
                diagnostics: SedimentaryBasinDiagnostics::default(),
            },
        )
    }

    #[test]
    fn composition_is_deterministic_and_preserves_every_input() {
        let inputs = fields(
            vec![0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0],
            vec![1.0, 0.5, 0.0, 0.2, 0.0, 0.0, 0.8, 1.0],
            vec![0.0, 0.4, 1.0, 0.2, 0.0, 0.7, 0.8, 1.0],
            vec![0.0, 0.0, 0.0, 0.5, 1.0, 0.6, 0.8, 1.0],
            vec![
                None,
                None,
                None,
                Some(0),
                Some(0),
                Some(0),
                Some(0),
                Some(0),
            ],
            vec![SedimentaryBasin {
                root_cell: 3,
                cell_count: 5,
                ocean_perimeter_fraction: 0.0,
                minimum_elevation: 0.6,
            }],
        );
        let original = inputs.clone();
        let compose = || {
            compose_geological_elevation(
                &inputs.0,
                &inputs.1,
                &inputs.2,
                &inputs.3,
                &inputs.4,
                0.6,
                GeologicalElevationConfig::default(),
            )
            .unwrap()
        };

        let first = compose();
        assert_eq!(first, compose());
        assert_eq!(inputs, original);
        assert!(
            first
                .cell_elevations
                .iter()
                .all(|value| (0.0..=1.0).contains(value))
        );
        assert_eq!(
            fingerprint(
                first
                    .cell_elevations
                    .iter()
                    .map(|value| u64::from(value.to_bits()))
            ),
            14_138_733_168_948_866_849
        );
    }

    #[test]
    fn overlapping_effects_follow_the_documented_stable_order() {
        let inputs = fields(
            vec![0.6, 0.4, 0.8, 0.2],
            vec![0.4, 0.0, 0.0, 0.0],
            vec![0.5, 0.0, 0.0, 0.0],
            vec![0.5, 0.0, 0.0, 0.0],
            vec![Some(0), Some(0), None, None],
            vec![SedimentaryBasin {
                root_cell: 0,
                cell_count: 2,
                ocean_perimeter_fraction: 0.0,
                minimum_elevation: 0.4,
            }],
        );
        let config = GeologicalElevationConfig {
            hotspot_uplift: 0.5,
            volcanic_arc_uplift: 0.2,
            craton_flattening: 0.5,
            basin_flattening: 0.5,
        };

        let result = compose_geological_elevation(
            &inputs.0, &inputs.1, &inputs.2, &inputs.3, &inputs.4, 0.5, config,
        )
        .unwrap();
        // 0.6 + 0.2 hotspot + 0.1 arc = 0.9; quarter-way toward 0.5 because
        // craton strength and flattening are both 0.5 = 0.8; halfway toward
        // the original component floor 0.4 = 0.6.
        assert_eq!(result.cell_elevations[0], 0.6);
        assert_eq!(result.diagnostics.hotspots.affected_cell_count, 1);
        assert_eq!(result.diagnostics.volcanic_arcs.affected_cell_count, 1);
        assert_eq!(result.diagnostics.cratons.affected_cell_count, 1);
        assert_eq!(result.diagnostics.basins.affected_cell_count, 1);
    }

    #[test]
    fn zero_strengths_are_identity_and_uplift_reports_clamped_actual_delta() {
        let inputs = fields(
            vec![1.0, 0.0, 0.5, 0.75],
            vec![1.0, 0.0, 0.0, 0.0],
            vec![1.0, 0.0, 0.0, 0.0],
            vec![0.0; 4],
            vec![None; 4],
            Vec::new(),
        );
        let result = compose_geological_elevation(
            &inputs.0,
            &inputs.1,
            &inputs.2,
            &inputs.3,
            &inputs.4,
            0.6,
            GeologicalElevationConfig::default(),
        )
        .unwrap();

        assert_eq!(result.cell_elevations, inputs.0.cell_elevations);
        assert_eq!(
            result.diagnostics.hotspots,
            ElevationEffectDiagnostics::default()
        );
        assert_eq!(
            result.diagnostics.volcanic_arcs,
            ElevationEffectDiagnostics::default()
        );
        assert_eq!(
            result.diagnostics.cratons,
            ElevationEffectDiagnostics::default()
        );
        assert_eq!(
            result.diagnostics.basins,
            ElevationEffectDiagnostics::default()
        );
    }

    #[test]
    fn rejects_invalid_configuration_shapes_and_basin_ownership() {
        let mut inputs = fields(
            vec![0.5; 4],
            vec![0.0; 4],
            vec![0.0; 4],
            vec![0.0; 4],
            vec![None; 4],
            Vec::new(),
        );
        inputs.1.cell_intensities.pop();
        assert_eq!(
            compose_geological_elevation(
                &inputs.0,
                &inputs.1,
                &inputs.2,
                &inputs.3,
                &inputs.4,
                0.6,
                GeologicalElevationConfig::default(),
            ),
            Err(GeologicalElevationError::FieldCountMismatch)
        );
        inputs.1.cell_intensities.push(0.0);
        inputs.4.cell_basins[0] = Some(0);
        assert_eq!(
            compose_geological_elevation(
                &inputs.0,
                &inputs.1,
                &inputs.2,
                &inputs.3,
                &inputs.4,
                0.6,
                GeologicalElevationConfig::default(),
            ),
            Err(GeologicalElevationError::InvalidBasinId)
        );

        let invalid = GeologicalElevationConfig {
            hotspot_uplift: f32::NAN,
            ..Default::default()
        };
        assert_eq!(
            compose_geological_elevation(
                &inputs.0, &inputs.1, &inputs.2, &inputs.3, &inputs.4, 0.6, invalid,
            ),
            Err(GeologicalElevationError::InvalidConfig)
        );
    }
}
