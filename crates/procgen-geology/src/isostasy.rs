use crate::{CratonField, GeologicalElevation, GeologyInputError, SedimentaryBasinField};
use procgen_sphere_mesh::{SphereMesh, multi_source_distances};
use procgen_tectonics::{
    BoundaryClass, BoundaryClassification, CrustClass, CrustClassification, FieldSummary,
    PlatePartition, StageInputError,
};
use std::fmt;

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
    pub plates: &'a PlatePartition,
    pub crust: &'a CrustClassification,
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
    pub adjusted_cell_count: usize,
    pub rise_cell_count: usize,
    pub sink_cell_count: usize,
    /// Sum of positive actual deltas after clamping.
    pub total_rise: f64,
    /// Sum of absolute negative actual deltas after clamping.
    pub total_sink: f64,
    pub maximum_rise: f32,
    pub maximum_sink: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IsostaticAdjustment {
    pub cell_support: Vec<f32>,
    pub cell_elevations: Vec<f32>,
    pub diagnostics: IsostaticAdjustmentDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsostaticAdjustmentError {
    Input(StageInputError),
    Geology(GeologyInputError),
    InvalidConfig,
}

impl fmt::Display for IsostaticAdjustmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => error.fmt(formatter),
            Self::Geology(error) => error.fmt(formatter),
            Self::InvalidConfig => formatter.write_str(
                "isostatic adjustment strengths and support values must be finite and between zero and one",
            ),
        }
    }
}

impl std::error::Error for IsostaticAdjustmentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            Self::Geology(error) => Some(error),
            Self::InvalidConfig => None,
        }
    }
}

impl From<StageInputError> for IsostaticAdjustmentError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
    }
}

impl From<GeologyInputError> for IsostaticAdjustmentError {
    fn from(error: GeologyInputError) -> Self {
        Self::Geology(error)
    }
}

/// Derives per-cell support from present-day fields and returns a separately
/// adjusted, clamped elevation field. Oceanic cells and sedimentary-basin
/// floors are unchanged, and no input field is modified.
pub fn derive_isostatic_adjustment(
    mesh: &SphereMesh,
    inputs: IsostaticAdjustmentInputs<'_>,
    config: IsostaticAdjustmentConfig,
) -> Result<IsostaticAdjustment, IsostaticAdjustmentError> {
    validate_inputs(mesh, inputs, config)?;

    let convergent_distances =
        boundary_distances(mesh, inputs.boundaries, BoundaryClass::Convergent);
    let divergent_distances = boundary_distances(mesh, inputs.boundaries, BoundaryClass::Divergent);
    let mut cell_support = Vec::with_capacity(mesh.cell_count());
    let mut cell_elevations = Vec::with_capacity(mesh.cell_count());
    let mut diagnostics = IsostaticAdjustmentDiagnostics::default();

    for cell in 0..mesh.cell_count() {
        let elevation = inputs.geological_elevation.cell_elevations[cell];
        let oceanic = inputs.crust.cell_class(inputs.plates, cell) == CrustClass::Oceanic;
        let basin = inputs.basins.cell_basins[cell].is_some();

        if oceanic {
            diagnostics.oceanic_cell_count += 1;
            cell_support.push(elevation);
            cell_elevations.push(elevation);
            continue;
        }
        if basin {
            diagnostics.preserved_basin_cell_count += 1;
            cell_support.push(elevation);
            cell_elevations.push(elevation);
            continue;
        }

        let support = (config.continental_support
            + proximity(convergent_distances[cell], config.maximum_boundary_distance)
                * config.convergent_support_bonus
            + inputs.cratons.cell_strengths[cell] * config.craton_support_bonus
            - proximity(divergent_distances[cell], config.maximum_boundary_distance)
                * config.divergent_support_penalty)
            .clamp(0.0, 1.0);
        let adjusted =
            (elevation + config.adjustment_strength * (support - elevation)).clamp(0.0, 1.0);
        record_delta(&mut diagnostics, adjusted - elevation);
        cell_support.push(support);
        cell_elevations.push(adjusted);
    }

    diagnostics.support = FieldSummary::from_values(&cell_support);
    diagnostics.elevation = FieldSummary::from_values(&cell_elevations);
    Ok(IsostaticAdjustment {
        cell_support,
        cell_elevations,
        diagnostics,
    })
}

fn boundary_distances(
    mesh: &SphereMesh,
    boundaries: &BoundaryClassification,
    class: BoundaryClass,
) -> Vec<Option<usize>> {
    let mut sources = Vec::new();
    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        if boundaries.edge_classes[edge_index] == class {
            sources.extend(edge.cells);
        }
    }
    multi_source_distances(mesh, &sources, |_, _| true)
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

fn record_delta(diagnostics: &mut IsostaticAdjustmentDiagnostics, delta: f32) {
    if delta > 0.0 {
        diagnostics.adjusted_cell_count += 1;
        diagnostics.rise_cell_count += 1;
        diagnostics.total_rise += f64::from(delta);
        diagnostics.maximum_rise = diagnostics.maximum_rise.max(delta);
    } else if delta < 0.0 {
        let sink = -delta;
        diagnostics.adjusted_cell_count += 1;
        diagnostics.sink_cell_count += 1;
        diagnostics.total_sink += f64::from(sink);
        diagnostics.maximum_sink = diagnostics.maximum_sink.max(sink);
    }
}

fn validate_inputs(
    mesh: &SphereMesh,
    inputs: IsostaticAdjustmentInputs<'_>,
    config: IsostaticAdjustmentConfig,
) -> Result<(), IsostaticAdjustmentError> {
    let unit_interval = |value: f32| value.is_finite() && (0.0..=1.0).contains(&value);
    if !unit_interval(config.adjustment_strength)
        || !unit_interval(config.continental_support)
        || !unit_interval(config.convergent_support_bonus)
        || !unit_interval(config.divergent_support_penalty)
        || !unit_interval(config.craton_support_bonus)
    {
        return Err(IsostaticAdjustmentError::InvalidConfig);
    }
    inputs.plates.validate(mesh)?;
    inputs.crust.validate(inputs.plates)?;
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
        CratonDiagnostics, GeologicalElevationDiagnostics, SedimentaryBasin,
        SedimentaryBasinDiagnostics,
    };
    use procgen_core::fingerprint;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::build_sphere_mesh;

    #[derive(Clone)]
    struct Fixture {
        mesh: SphereMesh,
        plates: PlatePartition,
        crust: CrustClassification,
        boundaries: BoundaryClassification,
        cratons: CratonField,
        basins: SedimentaryBasinField,
        elevation: GeologicalElevation,
    }

    impl Fixture {
        fn new(cell_count: usize) -> Self {
            let mesh = build_sphere_mesh(
                fibonacci_sphere(FibonacciConfig::new(cell_count)).unwrap(),
                1.0,
            )
            .unwrap();
            let plates = PlatePartition {
                cell_plates: vec![0; cell_count],
                plate_count: 1,
            };
            Self {
                boundaries: BoundaryClassification {
                    edge_classes: vec![BoundaryClass::Interior; mesh.edge_count()],
                    edge_normal_speeds: vec![[0.0; 2]; mesh.edge_count()],
                    edge_shear: vec![0.0; mesh.edge_count()],
                },
                crust: CrustClassification {
                    plate_classes: vec![CrustClass::Continental],
                },
                cratons: CratonField {
                    cell_strengths: vec![0.0; cell_count],
                    diagnostics: CratonDiagnostics::default(),
                },
                basins: SedimentaryBasinField {
                    cell_basins: vec![None; cell_count],
                    basins: Vec::new(),
                    diagnostics: SedimentaryBasinDiagnostics::default(),
                },
                elevation: GeologicalElevation {
                    cell_elevations: vec![0.65; cell_count],
                    diagnostics: GeologicalElevationDiagnostics::default(),
                },
                mesh,
                plates,
            }
        }

        fn derive(
            &self,
            config: IsostaticAdjustmentConfig,
        ) -> Result<IsostaticAdjustment, IsostaticAdjustmentError> {
            derive_isostatic_adjustment(
                &self.mesh,
                IsostaticAdjustmentInputs {
                    plates: &self.plates,
                    crust: &self.crust,
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
        assert_eq!(fixture.plates, original.plates);
        assert_eq!(fixture.crust, original.crust);
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
        assert!(risen.diagnostics.rise_cell_count > 0);
        assert!(sunk.diagnostics.sink_cell_count > 0);

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
        fixture.plates.plate_count = 2;
        fixture.crust.plate_classes = vec![CrustClass::Continental, CrustClass::Oceanic];
        fixture.plates.cell_plates[0] = 1;
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
        assert_eq!(result.diagnostics.adjusted_cell_count, 0);
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
            Err(IsostaticAdjustmentError::Input(StageInputError::Boundaries))
        );

        let fixture = Fixture::new(4);
        assert_eq!(
            fixture.derive(IsostaticAdjustmentConfig {
                adjustment_strength: f32::NAN,
                ..Default::default()
            }),
            Err(IsostaticAdjustmentError::InvalidConfig)
        );
    }
}
