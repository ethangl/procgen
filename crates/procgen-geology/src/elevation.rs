use crate::{
    CratonField, GeologyInputError, HotspotField, SedimentaryBasinField, VolcanicArcField,
    field::{
        ElevationEffectDiagnostics, GeologyStageError, apply_elevation_effect, lerp, unit_interval,
    },
};
use procgen_sphere_mesh::SphereMesh;
use procgen_tectonics::{CoarseElevation, FieldSummary};

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

#[derive(Clone, Copy, Debug)]
pub struct GeologicalElevationInputs<'a> {
    pub tectonic_elevation: &'a CoarseElevation,
    pub hotspots: &'a HotspotField,
    pub volcanic_arcs: &'a VolcanicArcField,
    pub cratons: &'a CratonField,
    pub basins: &'a SedimentaryBasinField,
    /// Validated normalized continental base used by tectonic base elevation.
    pub continental_base: f32,
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

impl GeologicalElevation {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), GeologyInputError> {
        if self.cell_elevations.len() != mesh.cell_count() {
            return Err(GeologyInputError::Elevation);
        }
        Ok(())
    }
}

/// Composes a new normalized geological elevation field in this stable order:
/// hotspot uplift, volcanic-arc uplift, craton flattening, then basin flattening.
///
/// Basin floors are the deterministic component minima captured by the basin
/// field from the input tectonic elevation. No input field is modified, and
/// sparse oceanic peaks are deliberately not consumed by this stage.
pub fn compose_geological_elevation(
    mesh: &SphereMesh,
    inputs: GeologicalElevationInputs<'_>,
    config: GeologicalElevationConfig,
) -> Result<GeologicalElevation, GeologyStageError> {
    validate_inputs(mesh, inputs, config)?;

    let mut cell_elevations = inputs.tectonic_elevation.cell_elevations.clone();
    let mut diagnostics = GeologicalElevationDiagnostics::default();

    apply_elevation_effect(
        &mut cell_elevations,
        &mut diagnostics.hotspots,
        |cell, elevation| {
            (elevation + inputs.hotspots.cell_intensities[cell] * config.hotspot_uplift)
                .clamp(0.0, 1.0)
        },
    );
    apply_elevation_effect(
        &mut cell_elevations,
        &mut diagnostics.volcanic_arcs,
        |cell, elevation| {
            (elevation + inputs.volcanic_arcs.cell_strengths[cell] * config.volcanic_arc_uplift)
                .clamp(0.0, 1.0)
        },
    );
    apply_elevation_effect(
        &mut cell_elevations,
        &mut diagnostics.cratons,
        |cell, elevation| {
            lerp(
                elevation,
                inputs.continental_base,
                inputs.cratons.cell_strengths[cell] * config.craton_flattening,
            )
        },
    );
    apply_elevation_effect(
        &mut cell_elevations,
        &mut diagnostics.basins,
        |cell, elevation| {
            inputs.basins.cell_basins[cell].map_or(elevation, |basin| {
                lerp(
                    elevation,
                    inputs.basins.basins[basin].minimum_elevation,
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

fn validate_inputs(
    mesh: &SphereMesh,
    inputs: GeologicalElevationInputs<'_>,
    config: GeologicalElevationConfig,
) -> Result<(), GeologyStageError> {
    if !unit_interval(config.hotspot_uplift)
        || !unit_interval(config.volcanic_arc_uplift)
        || !unit_interval(config.craton_flattening)
        || !unit_interval(config.basin_flattening)
        || !unit_interval(inputs.continental_base)
    {
        return Err(GeologyStageError::InvalidConfig);
    }

    inputs.tectonic_elevation.validate(mesh)?;
    inputs.hotspots.validate(mesh)?;
    inputs.volcanic_arcs.validate(mesh)?;
    inputs.cratons.validate(mesh)?;
    inputs.basins.validate(mesh)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        SedimentaryBasin,
        test_support::{empty_basins, empty_cratons, empty_hotspots, empty_volcanic_arcs, mesh},
    };
    use procgen_core::fingerprint;
    use procgen_tectonics::StageInputError;

    #[derive(Clone)]
    struct Fixture {
        tectonic_elevation: CoarseElevation,
        hotspots: HotspotField,
        volcanic_arcs: VolcanicArcField,
        cratons: CratonField,
        basins: SedimentaryBasinField,
    }

    impl Fixture {
        fn new(elevations: Vec<f32>) -> Self {
            let cell_count = elevations.len();
            Self {
                tectonic_elevation: CoarseElevation {
                    cell_elevations: elevations,
                    diagnostics: Default::default(),
                },
                hotspots: empty_hotspots(cell_count),
                volcanic_arcs: empty_volcanic_arcs(cell_count),
                cratons: empty_cratons(cell_count),
                basins: empty_basins(cell_count),
            }
        }

        fn compose(
            &self,
            mesh: &SphereMesh,
            continental_base: f32,
            config: GeologicalElevationConfig,
        ) -> Result<GeologicalElevation, GeologyStageError> {
            compose_geological_elevation(
                mesh,
                GeologicalElevationInputs {
                    tectonic_elevation: &self.tectonic_elevation,
                    hotspots: &self.hotspots,
                    volcanic_arcs: &self.volcanic_arcs,
                    cratons: &self.cratons,
                    basins: &self.basins,
                    continental_base,
                },
                config,
            )
        }
    }

    #[test]
    fn composition_is_deterministic_and_preserves_every_input() {
        let mesh = mesh(8);
        let mut fixture = Fixture::new(vec![0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0]);
        fixture.hotspots.cell_intensities = vec![1.0, 0.5, 0.0, 0.2, 0.0, 0.0, 0.8, 1.0];
        fixture.volcanic_arcs.cell_strengths = vec![0.0, 0.4, 1.0, 0.2, 0.0, 0.7, 0.8, 1.0];
        fixture.cratons.cell_strengths = vec![0.0, 0.0, 0.0, 0.5, 1.0, 0.6, 0.8, 1.0];
        fixture.basins.cell_basins = vec![
            None,
            None,
            None,
            Some(0),
            Some(0),
            Some(0),
            Some(0),
            Some(0),
        ];
        fixture.basins.basins = vec![SedimentaryBasin {
            root_cell: 3,
            cell_count: 5,
            ocean_perimeter_fraction: 0.0,
            minimum_elevation: 0.6,
        }];
        let original = fixture.clone();
        let compose = || {
            fixture
                .compose(&mesh, 0.6, GeologicalElevationConfig::default())
                .unwrap()
        };

        let first = compose();
        assert_eq!(first, compose());
        assert_eq!(fixture.tectonic_elevation, original.tectonic_elevation);
        assert_eq!(fixture.hotspots, original.hotspots);
        assert_eq!(fixture.volcanic_arcs, original.volcanic_arcs);
        assert_eq!(fixture.cratons, original.cratons);
        assert_eq!(fixture.basins, original.basins);
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
        let mesh = mesh(4);
        let mut fixture = Fixture::new(vec![0.6, 0.4, 0.8, 0.2]);
        fixture.hotspots.cell_intensities = vec![0.4, 0.0, 0.0, 0.0];
        fixture.volcanic_arcs.cell_strengths = vec![0.5, 0.0, 0.0, 0.0];
        fixture.cratons.cell_strengths = vec![0.5, 0.0, 0.0, 0.0];
        fixture.basins.cell_basins = vec![Some(0), Some(0), None, None];
        fixture.basins.basins = vec![SedimentaryBasin {
            root_cell: 0,
            cell_count: 2,
            ocean_perimeter_fraction: 0.0,
            minimum_elevation: 0.4,
        }];
        let config = GeologicalElevationConfig {
            hotspot_uplift: 0.5,
            volcanic_arc_uplift: 0.2,
            craton_flattening: 0.5,
            basin_flattening: 0.5,
        };

        let result = fixture.compose(&mesh, 0.5, config).unwrap();
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
        let mesh = mesh(4);
        let mut fixture = Fixture::new(vec![1.0, 0.0, 0.5, 0.75]);
        fixture.hotspots.cell_intensities = vec![1.0, 0.0, 0.0, 0.0];
        fixture.volcanic_arcs.cell_strengths = vec![1.0, 0.0, 0.0, 0.0];
        let result = fixture
            .compose(&mesh, 0.6, GeologicalElevationConfig::default())
            .unwrap();

        assert_eq!(
            result.cell_elevations,
            fixture.tectonic_elevation.cell_elevations
        );
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
        let mesh = mesh(4);
        let mut fixture = Fixture::new(vec![0.5; 4]);

        fixture.tectonic_elevation.cell_elevations.pop();
        assert_eq!(
            fixture.compose(&mesh, 0.6, GeologicalElevationConfig::default()),
            Err(GeologyStageError::Input(StageInputError::Elevation))
        );
        fixture.tectonic_elevation.cell_elevations.push(0.5);

        fixture.hotspots.cell_intensities.pop();
        assert_eq!(
            fixture.compose(&mesh, 0.6, GeologicalElevationConfig::default()),
            Err(GeologyStageError::Geology(GeologyInputError::Hotspots))
        );
        fixture.hotspots.cell_intensities.push(0.0);

        fixture.volcanic_arcs.cell_strengths.pop();
        assert_eq!(
            fixture.compose(&mesh, 0.6, GeologicalElevationConfig::default()),
            Err(GeologyStageError::Geology(GeologyInputError::VolcanicArcs))
        );
        fixture.volcanic_arcs.cell_strengths.push(0.0);

        fixture.cratons.cell_strengths.pop();
        assert_eq!(
            fixture.compose(&mesh, 0.6, GeologicalElevationConfig::default()),
            Err(GeologyStageError::Geology(GeologyInputError::Cratons))
        );
        fixture.cratons.cell_strengths.push(0.0);

        fixture.basins.cell_basins[0] = Some(0);
        assert_eq!(
            fixture.compose(&mesh, 0.6, GeologicalElevationConfig::default()),
            Err(GeologyStageError::Geology(GeologyInputError::Basins))
        );
        fixture.basins.cell_basins[0] = None;

        let invalid = GeologicalElevationConfig {
            hotspot_uplift: f32::NAN,
            ..Default::default()
        };
        assert_eq!(
            fixture.compose(&mesh, 0.6, invalid),
            Err(GeologyStageError::InvalidConfig)
        );
    }
}
