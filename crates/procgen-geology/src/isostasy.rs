use crate::{
    CratonField, GeologicalElevation, SedimentaryBasinField,
    field::{
        GeologyStageError, SignedEffectDiagnostics, apply_elevation_effect, lerp, unit_interval,
    },
};
use procgen_sphere_mesh::{SphereMesh, edge_cell_distances};
use procgen_tectonics::{
    BoundaryClass, BoundaryClassification, CellCrust, CrustClass, ElevationField, FieldSummary,
};

/// Configuration for deterministic present-day isostatic support and adjustment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IsostaticAdjustmentConfig {
    /// Fraction of the distance from geological elevation to isostatic support.
    pub adjustment_strength: f32,
    /// Equilibrium support of continental crust away from other effects.
    pub continental_support: f32,
    /// Maximum support added at a convergent boundary.
    pub convergent_support_bonus: f32,
    /// Maximum support removed at a divergent boundary.
    pub divergent_support_penalty: f32,
    /// Maximum support added by full craton strength.
    pub craton_support_bonus: f32,
    /// Inclusive graph-hop range over which boundary proximity decays.
    pub maximum_boundary_distance: usize,
}

impl Default for IsostaticAdjustmentConfig {
    fn default() -> Self {
        Self {
            adjustment_strength: 0.4,
            continental_support: 0.65,
            convergent_support_bonus: 0.25,
            divergent_support_penalty: 0.15,
            craton_support_bonus: 0.10,
            maximum_boundary_distance: 5,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct IsostaticAdjustmentInputs<'a> {
    pub crust: CellCrust<'a>,
    pub boundaries: &'a BoundaryClassification,
    pub cratons: &'a CratonField,
    pub basins: &'a SedimentaryBasinField,
    pub geological_elevation: &'a GeologicalElevation,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IsostaticAdjustmentDiagnostics {
    pub support: FieldSummary,
    pub elevation: FieldSummary,
    pub oceanic_cell_count: usize,
    pub preserved_basin_cell_count: usize,
    pub adjustment: SignedEffectDiagnostics,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IsostaticAdjustment {
    /// Continental equilibrium support. Preserved oceanic and basin cells use
    /// their geological elevation so the uniform adjustment remains a no-op.
    pub cell_support: Vec<f32>,
    pub cell_elevations: Vec<f32>,
    /// Datum this field was adjusted against, carried from the geological
    /// elevation it started from.
    pub sea_level: f32,
    pub diagnostics: IsostaticAdjustmentDiagnostics,
}

impl IsostaticAdjustment {
    pub fn field(&self) -> ElevationField<'_> {
        ElevationField {
            cell_elevations: &self.cell_elevations,
            sea_level: self.sea_level,
        }
    }

    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), crate::GeologyInputError> {
        if self.cell_support.len() != mesh.cell_count()
            || self.cell_elevations.len() != mesh.cell_count()
        {
            return Err(crate::GeologyInputError::Isostasy);
        }
        Ok(())
    }
}

/// Derives per-cell support from present-day fields and returns a separately
/// adjusted, clamped elevation field. Oceanic cells and sedimentary-basin
/// floors are unchanged, and no input field is modified.
pub fn derive_isostatic_adjustment(
    mesh: &SphereMesh,
    inputs: IsostaticAdjustmentInputs<'_>,
    config: IsostaticAdjustmentConfig,
) -> Result<IsostaticAdjustment, GeologyStageError> {
    validate_inputs(mesh, inputs, config)?;

    let convergent_distances = edge_cell_distances(mesh, |edge, _| {
        inputs.boundaries.edge_classes[edge] == BoundaryClass::Convergent
    });
    let divergent_distances = edge_cell_distances(mesh, |edge, _| {
        inputs.boundaries.edge_classes[edge] == BoundaryClass::Divergent
    });
    let oceanic = |cell: usize| inputs.crust.class(cell) == CrustClass::Oceanic;
    let basin = |cell: usize| inputs.basins.cell_basins[cell].is_some();
    let preserved = |cell: usize| oceanic(cell) || basin(cell);
    let oceanic_cell_count = (0..mesh.cell_count()).filter(|&cell| oceanic(cell)).count();
    let preserved_basin_cell_count = (0..mesh.cell_count())
        .filter(|&cell| !oceanic(cell) && basin(cell))
        .count();
    let mut diagnostics = IsostaticAdjustmentDiagnostics {
        oceanic_cell_count,
        preserved_basin_cell_count,
        ..Default::default()
    };

    let cell_support: Vec<_> = (0..mesh.cell_count())
        .map(|cell| {
            let elevation = inputs.geological_elevation.cell_elevations[cell];
            if preserved(cell) {
                elevation
            } else {
                (config.continental_support
                    + proximity(convergent_distances[cell], config.maximum_boundary_distance)
                        * config.convergent_support_bonus
                    + inputs.cratons.cell_strengths[cell] * config.craton_support_bonus
                    - proximity(divergent_distances[cell], config.maximum_boundary_distance)
                        * config.divergent_support_penalty)
                    .clamp(0.0, 1.0)
            }
        })
        .collect();
    let mut cell_elevations = inputs.geological_elevation.cell_elevations.clone();
    apply_elevation_effect(
        &mut cell_elevations,
        &mut diagnostics.adjustment,
        |cell, elevation| {
            lerp(elevation, cell_support[cell], config.adjustment_strength).clamp(0.0, 1.0)
        },
    );

    diagnostics.support = FieldSummary::from_values(&cell_support);
    diagnostics.elevation = FieldSummary::from_values(&cell_elevations);
    Ok(IsostaticAdjustment {
        cell_support,
        cell_elevations,
        sea_level: inputs.geological_elevation.sea_level,
        diagnostics,
    })
}

fn proximity(distance: Option<usize>, maximum_distance: usize) -> f32 {
    distance.map_or(0.0, |distance| {
        if distance > maximum_distance {
            0.0
        } else {
            1.0 - distance as f32 / (maximum_distance as f32 + 1.0)
        }
    })
}

fn validate_inputs(
    mesh: &SphereMesh,
    inputs: IsostaticAdjustmentInputs<'_>,
    config: IsostaticAdjustmentConfig,
) -> Result<(), GeologyStageError> {
    if !unit_interval(config.adjustment_strength)
        || !unit_interval(config.continental_support)
        || !unit_interval(config.convergent_support_bonus)
        || !unit_interval(config.divergent_support_penalty)
        || !unit_interval(config.craton_support_bonus)
    {
        return Err(GeologyStageError::InvalidConfig);
    }
    inputs.crust.validate(mesh)?;
    inputs.boundaries.validate(mesh)?;
    inputs.cratons.validate(mesh)?;
    inputs.basins.validate(mesh)?;
    inputs.geological_elevation.validate(mesh)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ElevationEffectDiagnostics, GeologicalElevationDiagnostics, GeologyInputError,
        SedimentaryBasin,
        test_support::{empty_basins, empty_cratons, mesh},
    };
    use procgen_core::fingerprint;
    use procgen_tectonics::{CoarseElevationConfig, StageInputError};

    #[derive(Clone)]
    struct Fixture {
        mesh: SphereMesh,
        cell_birth: Vec<Option<i32>>,
        boundaries: BoundaryClassification,
        cratons: CratonField,
        basins: SedimentaryBasinField,
        elevation: GeologicalElevation,
    }

    impl Fixture {
        fn new(cell_count: usize) -> Self {
            let mesh = mesh(cell_count);
            Self {
                boundaries: BoundaryClassification {
                    edge_classes: vec![BoundaryClass::Interior; mesh.edge_count()],
                    edge_normal_speeds: vec![[0.0; 2]; mesh.edge_count()],
                    edge_shear: vec![0.0; mesh.edge_count()],
                },
                cell_birth: vec![None; cell_count],
                cratons: empty_cratons(cell_count),
                basins: empty_basins(cell_count),
                elevation: GeologicalElevation {
                    cell_elevations: vec![0.65; cell_count],
                    sea_level: CoarseElevationConfig::default().sea_level,
                    diagnostics: GeologicalElevationDiagnostics::default(),
                },
                mesh,
            }
        }

        fn derive(
            &self,
            config: IsostaticAdjustmentConfig,
        ) -> Result<IsostaticAdjustment, GeologyStageError> {
            derive_isostatic_adjustment(
                &self.mesh,
                IsostaticAdjustmentInputs {
                    crust: CellCrust {
                        cell_birth: &self.cell_birth,
                    },
                    boundaries: &self.boundaries,
                    cratons: &self.cratons,
                    basins: &self.basins,
                    geological_elevation: &self.elevation,
                },
                config,
            )
        }
    }

    #[test]
    fn adjustment_is_deterministic_and_preserves_every_input() {
        let mut fixture = Fixture::new(32);
        fixture.boundaries.edge_classes[0] = BoundaryClass::Convergent;
        fixture.boundaries.edge_classes[1] = BoundaryClass::Divergent;
        fixture.cratons.cell_strengths[4] = 1.0;
        fixture.elevation.cell_elevations[5] = 0.9;
        let original = fixture.clone();

        let first = fixture
            .derive(IsostaticAdjustmentConfig::default())
            .unwrap();
        assert_eq!(
            first,
            fixture
                .derive(IsostaticAdjustmentConfig::default())
                .unwrap()
        );
        assert_eq!(fixture.cell_birth, original.cell_birth);
        assert_eq!(fixture.boundaries, original.boundaries);
        assert_eq!(fixture.cratons, original.cratons);
        assert_eq!(fixture.basins, original.basins);
        assert_eq!(fixture.elevation, original.elevation);
        assert_eq!(
            fingerprint(
                first
                    .cell_support
                    .iter()
                    .map(|value| u64::from(value.to_bits()))
            ),
            16_615_095_888_973_175_191
        );
    }

    /// The datum belongs to the field, not to a config the stage never sees,
    /// so an adjusted field answers land questions the way its input did.
    #[test]
    fn the_adjusted_field_carries_the_datum_it_started_from() {
        let mut fixture = Fixture::new(16);
        fixture.elevation.sea_level = 0.42;
        // An oceanic cell keeps its elevation, so the only thing deciding
        // whether it is land is the datum the field carries.
        fixture.cell_birth[0] = Some(0);
        fixture.elevation.cell_elevations[0] = 0.41;

        let adjusted = fixture
            .derive(IsostaticAdjustmentConfig::default())
            .unwrap();

        assert_eq!(adjusted.sea_level, 0.42);
        assert!(!adjusted.field().is_land(0));
        assert!(adjusted.field().is_land(1));
    }

    #[test]
    fn convergence_and_cratons_raise_support_while_divergence_lowers_it() {
        let mut convergent = Fixture::new(16);
        convergent.boundaries.edge_classes[0] = BoundaryClass::Convergent;
        let boundary_cell = convergent.mesh.edges[0].cells[0];
        convergent.cratons.cell_strengths[boundary_cell] = 1.0;
        convergent.elevation.cell_elevations[boundary_cell] = 0.5;
        let risen = convergent
            .derive(IsostaticAdjustmentConfig::default())
            .unwrap();

        let mut divergent = convergent.clone();
        divergent.boundaries.edge_classes[0] = BoundaryClass::Divergent;
        divergent.cratons.cell_strengths[boundary_cell] = 0.0;
        divergent.elevation.cell_elevations[boundary_cell] = 0.8;
        let sunk = divergent
            .derive(IsostaticAdjustmentConfig::default())
            .unwrap();

        assert!(risen.cell_support[boundary_cell] > 0.65);
        assert!(risen.cell_elevations[boundary_cell] > 0.5);
        assert!(sunk.cell_support[boundary_cell] < 0.65);
        assert!(sunk.cell_elevations[boundary_cell] < 0.8);
        assert!(risen.diagnostics.adjustment.rise.affected_cell_count > 0);
        assert!(sunk.diagnostics.adjustment.sink.affected_cell_count > 0);

        let mut cratonic = Fixture::new(8);
        cratonic.cratons.cell_strengths[0] = 1.0;
        cratonic.elevation.cell_elevations[0] = 0.5;
        let cratonic = cratonic
            .derive(IsostaticAdjustmentConfig::default())
            .unwrap();
        assert_eq!(cratonic.cell_support[0], 0.75);
        assert!(cratonic.cell_elevations[0] > 0.5);
    }

    #[test]
    fn boundary_proximity_has_an_inclusive_bounded_decay() {
        let mut fixture = Fixture::new(16);
        fixture.boundaries.edge_classes[0] = BoundaryClass::Convergent;
        let source_cells = fixture.mesh.edges[0].cells;
        let outside_cell = (0..fixture.mesh.cell_count())
            .find(|cell| !source_cells.contains(cell))
            .unwrap();

        let result = fixture
            .derive(IsostaticAdjustmentConfig {
                maximum_boundary_distance: 0,
                ..Default::default()
            })
            .unwrap();

        for cell in source_cells {
            assert_eq!(result.cell_support[cell], 0.9);
        }
        assert_eq!(result.cell_support[outside_cell], 0.65);
    }

    #[test]
    fn oceanic_cells_and_basin_floors_are_unchanged() {
        let mut fixture = Fixture::new(8);
        fixture.cell_birth[0] = Some(0);
        fixture.elevation.cell_elevations[0] = 0.2;
        fixture.elevation.cell_elevations[1] = 0.42;
        fixture.basins.cell_basins[1] = Some(0);
        fixture.basins.basins.push(SedimentaryBasin {
            root_cell: 1,
            cell_count: 1,
            ocean_perimeter_fraction: 0.0,
            minimum_elevation: 0.42,
        });

        let result = fixture
            .derive(IsostaticAdjustmentConfig::default())
            .unwrap();
        assert_eq!(result.cell_support[0], 0.2);
        assert_eq!(result.cell_elevations[0], 0.2);
        assert_eq!(result.cell_support[1], 0.42);
        assert_eq!(result.cell_elevations[1], 0.42);
        assert_eq!(result.diagnostics.oceanic_cell_count, 1);
        assert_eq!(result.diagnostics.preserved_basin_cell_count, 1);
    }

    #[test]
    fn zero_strength_returns_an_identical_but_separate_elevation_field() {
        let mut fixture = Fixture::new(8);
        fixture.elevation.cell_elevations = (0..8).map(|cell| cell as f32 / 7.0).collect();
        let result = fixture
            .derive(IsostaticAdjustmentConfig {
                adjustment_strength: 0.0,
                ..Default::default()
            })
            .unwrap();

        assert_eq!(result.cell_elevations, fixture.elevation.cell_elevations);
        assert_eq!(
            result.diagnostics.adjustment.rise,
            ElevationEffectDiagnostics::default()
        );
        assert_eq!(
            result.diagnostics.adjustment.sink,
            ElevationEffectDiagnostics::default()
        );
        assert_ne!(result.cell_support, fixture.elevation.cell_elevations);
    }

    #[test]
    fn clamps_extreme_support_and_rejects_invalid_inputs() {
        let mut fixture = Fixture::new(4);
        fixture.cratons.cell_strengths.fill(1.0);
        let result = fixture
            .derive(IsostaticAdjustmentConfig {
                continental_support: 1.0,
                craton_support_bonus: 1.0,
                adjustment_strength: 1.0,
                ..Default::default()
            })
            .unwrap();
        assert!(result.cell_support.iter().all(|&value| value == 1.0));
        assert!(result.cell_elevations.iter().all(|&value| value == 1.0));

        fixture.boundaries.edge_classes.pop();
        assert_eq!(
            fixture.derive(IsostaticAdjustmentConfig::default()),
            Err(GeologyStageError::Input(StageInputError::Boundaries))
        );

        let mut fixture = Fixture::new(4);
        fixture.elevation.cell_elevations.pop();
        assert_eq!(
            fixture.derive(IsostaticAdjustmentConfig::default()),
            Err(GeologyStageError::Geology(GeologyInputError::Elevation))
        );

        let fixture = Fixture::new(4);
        assert_eq!(
            fixture.derive(IsostaticAdjustmentConfig {
                adjustment_strength: f32::NAN,
                ..Default::default()
            }),
            Err(GeologyStageError::InvalidConfig)
        );
    }
}
