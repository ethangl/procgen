//! Broad base elevation: what a cell would stand at with no boundary near it.
//!
//! Oceanic crust follows a square-root cooling curve from a ridge down to the
//! deep floor. Continental crust starts from one configured base, tapered to
//! `margin_edge_elevation` over the outermost `margin_width_hops` cells so
//! that a continent ends in a shelf rather than a cliff. Neither varies over a
//! plate interior, so a plate wider than the few cells a boundary deforms
//! would be flat by construction; the two interior relief fields are the
//! relief Earth has and those lack.
//!
//! The margin taper is what makes the sea-level datum do visible work. Hop
//! distance to the nearest oceanic cell is measured against the final cell
//! crust and restricted by nothing, so a shelf may cross a plate boundary and
//! the ocean a rift opened during the run gets shelves on both of its sides,
//! the same as any other coast. That is why the taper reads [`CellCrust`]
//! rather than the initial classification.
//!
//! The two interior relief terms live in [`crate::interior_relief`] and add on
//! top of both, so a shelf carries the same relief as the plateau behind it.
//!

use crate::{
    CellCrust, CrustClass, DEFAULT_STEP_DURATION, FieldSummary, FlowField, SeafloorAge,
    StageInputError,
    interior_relief::{basement_field, dynamic_topography_field, validate_interior_relief},
};
use procgen_noise::{OctaveConfig, Validated};
use procgen_sphere_mesh::{SphereMesh, multi_source_distances};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaseElevationConfig {
    /// Seed of the continental basement field. Nothing else here is hashed.
    pub seed: u64,
    pub continental_base: f32,
    /// Elevation of age-zero oceanic crust at a ridge.
    pub ridge_elevation: f32,
    /// Minimum elevation reached by sufficiently old oceanic crust.
    pub deep_ocean_elevation: f32,
    /// Seafloor age, as model time, at which oceanic crust reaches the
    /// deep-ocean floor. Age spans one step, for crust born at a ridge during
    /// the run, to the prior's hop age plus the whole run for crust that
    /// predates it, so this belongs near the top of that span: below it the
    /// whole ocean sits on the deep floor and only crust made during the run
    /// carries any gradient.
    ///
    /// It is a time rather than a step count, so a run sliced more finely
    /// cools its floor to the same depth over the same span of model time.
    pub cooling_age: f32,
    /// Swell raised where the flow field's divergence is one root-mean-square
    /// of its own, and the sag where it converges by the same. Applies to
    /// every cell. See the module documentation for why this and
    /// `basement_amplitude` are bounded where they are.
    pub dynamic_topography_amplitude: f32,
    /// Relief of the continental basement at a full octave sum, on
    /// continental crust only. Three octaves rarely align, so the relief the
    /// field actually reaches is about half of this. Bounded with the dynamic
    /// term; see the module documentation.
    pub basement_amplitude: f32,
    /// Lattice frequency of the basement's first octave, in cycles per unit
    /// direction. A lattice feature spans about two cells, so the default's
    /// longest wavelength is `2 / 3` of a model unit — a tenth of a great
    /// circle, or some 48 cells of the default mesh — and its shortest, two
    /// octaves up, is a quarter of that, about a dozen cells.
    pub basement_frequency: f32,
    /// Continental cells within this many hops of the ocean carry the margin
    /// taper instead of the flat `continental_base`. Zero disables the taper
    /// and restores the cliff a continent's edge was before it existed.
    pub margin_width_hops: usize,
    /// Base elevation of the outermost continental cell, the shelf's seaward
    /// edge. Must be within the unit range and not above `continental_base`.
    ///
    /// This and `margin_width_hops` are chosen together against the default
    /// sea level of 0.5. With `continental_base` 0.65, three hops, and an edge
    /// at 0.46, the taper puts the three margin cells at 0.46, `0.46 + 0.19/3`
    /// which is 0.5233, and `0.46 + 0.38/3` which is 0.5867. At the datum the
    /// outermost is flooded and the other two stand; raising it to 0.55 floods
    /// the outer two; lowering it to 0.45 exposes all three, so the coast and
    /// the crust boundary coincide. [`crate::is_land`] is strict, so a cell
    /// exactly at the datum is ocean.
    pub margin_edge_elevation: f32,
}

impl Default for BaseElevationConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            continental_base: 0.65,
            ridge_elevation: 0.30,
            deep_ocean_elevation: 0.08,
            // Forty default steps, which is where the step count this was
            // tuned as still puts it.
            cooling_age: 40.0 * DEFAULT_STEP_DURATION,
            dynamic_topography_amplitude: 0.03,
            basement_amplitude: 0.05,
            basement_frequency: 3.0,
            margin_width_hops: 3,
            margin_edge_elevation: 0.46,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BaseElevationDiagnostics {
    pub summary: FieldSummary,
    pub oceanic: FieldSummary,
    /// The dynamic topography term over every cell, which is where it applies.
    pub dynamic_topography: FieldSummary,
    /// The basement term over continental cells, which is where it applies.
    /// It is exactly zero everywhere else.
    pub basement: FieldSummary,
    /// How far the margin taper lowered each margin cell below
    /// `continental_base`, over the margin cells alone. Its maximum is the
    /// full drop to `margin_edge_elevation`, which the outermost cells take.
    pub margin_depth: FieldSummary,
    pub oceanic_cell_count: usize,
    pub continental_cell_count: usize,
    /// Continental cells the taper reached, which is every continental cell
    /// within `margin_width_hops` of the ocean.
    pub margin_cell_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BaseElevation {
    pub cell_elevations: Vec<f32>,
    pub diagnostics: BaseElevationDiagnostics,
}

impl BaseElevation {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), StageInputError> {
        if self.cell_elevations.len() != mesh.cell_count() {
            return Err(StageInputError::BaseElevation);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseElevationError {
    InvalidConfig,
    InvalidMarginEdge,
    InvalidInteriorAmplitude,
    InvalidBasementFrequency,
    Input(StageInputError),
}

impl fmt::Display for BaseElevationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str(
                "base elevations must be finite and between 0 and 1, ridge elevation must not be below deep-ocean elevation, and cooling age must be positive",
            ),
            Self::InvalidMarginEdge => formatter.write_str(
                "margin edge elevation must be finite, between 0 and 1, and not above the continental base",
            ),
            Self::InvalidInteriorAmplitude => formatter.write_str(
                "dynamic topography and basement amplitudes must be finite and non-negative",
            ),
            Self::InvalidBasementFrequency => formatter.write_str(
                "basement frequency must be finite, positive, and leave its octaves finite",
            ),
            Self::Input(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for BaseElevationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StageInputError> for BaseElevationError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
    }
}

/// Derives normalized per-cell base elevation from final seafloor age, final
/// cell crust, and the flow field the plates were fitted to.
///
/// The oceanic cooling curve runs from ridge elevation to deep-ocean elevation
/// with the square root of age over `cooling_age`, and ages at or above it use
/// the deep floor; continental crust starts from `continental_base`, tapered
/// to `margin_edge_elevation` over the outermost `margin_width_hops` cells.
/// Crust that predates the run carries the prior's age plus the time the run
/// took, so an old ocean reaches the deep floor and crust born at a ridge
/// during the run does not. Onto that go the dynamic topography of the flow
/// field's divergence, everywhere, and the continental basement's octaves, on
/// continental crust alone, so a shelf carries the same interior relief as the
/// plateau behind it.
pub fn derive_base_elevation(
    mesh: &SphereMesh,
    seafloor_age: &SeafloorAge,
    cell_crust: CellCrust<'_>,
    flow_field: &FlowField,
    config: BaseElevationConfig,
) -> Result<BaseElevation, BaseElevationError> {
    let octaves = validate_config(config)?;
    seafloor_age.validate(mesh)?;
    cell_crust.validate(mesh)?;

    let ContinentalBase {
        cell_bases,
        margin_cells,
    } = continental_base_field(mesh, cell_crust, config);
    let dynamic_topography = dynamic_topography_field(mesh, flow_field, config);
    let basement = basement_field(mesh, cell_crust, config, octaves);
    let cell_elevations: Vec<_> = seafloor_age
        .cell_ages
        .iter()
        .zip(&cell_bases)
        .zip(&dynamic_topography)
        .zip(&basement)
        .map(|(((age, &continental_base), &dynamic), &basement)| {
            // The composition stage clamps for the same reason: elevation is
            // normalized and nothing downstream reads a value outside it.
            // Only the deep end of the oceanic curve can reach it, where the
            // floor at 0.08 leaves the dynamic term under three times its own
            // amplitude of room and the field's peak divergence asks for
            // more; the continental base at 0.65 has room for both terms
            // several times over. `docs/plate-movement.md` counts the cells
            // that clamp.
            (cooled(*age, continental_base, config) + dynamic + basement).clamp(0.0, 1.0)
        })
        .collect();

    let oceanic_elevations: Vec<_> = seafloor_age
        .cell_ages
        .iter()
        .zip(&cell_elevations)
        .filter_map(|(age, &elevation)| age.map(|_| elevation))
        .collect();
    let continental_basement: Vec<_> = (0..mesh.cell_count())
        .filter(|&cell| cell_crust.class(cell) == CrustClass::Continental)
        .map(|cell| basement[cell])
        .collect();
    let margin_depth: Vec<_> = margin_cells
        .iter()
        .map(|&cell| config.continental_base - cell_bases[cell])
        .collect();
    let oceanic_cell_count = oceanic_elevations.len();
    let diagnostics = BaseElevationDiagnostics {
        summary: FieldSummary::from_values(&cell_elevations),
        oceanic: FieldSummary::from_values(&oceanic_elevations),
        dynamic_topography: FieldSummary::from_values(&dynamic_topography),
        basement: FieldSummary::from_values(&continental_basement),
        margin_depth: FieldSummary::from_values(&margin_depth),
        oceanic_cell_count,
        continental_cell_count: cell_elevations.len() - oceanic_cell_count,
        margin_cell_count: margin_cells.len(),
    };
    Ok(BaseElevation {
        cell_elevations,
        diagnostics,
    })
}

/// The cooling curve's own answer for one cell, before interior relief.
/// `None` is continental crust, which the curve does not describe; that cell
/// takes the margin taper's `continental_base` instead.
fn cooled(age: Option<f32>, continental_base: f32, config: BaseElevationConfig) -> f32 {
    match age {
        None => continental_base,
        Some(age) if age >= config.cooling_age => config.deep_ocean_elevation,
        Some(age) => {
            let progress = (age / config.cooling_age).sqrt();
            config.ridge_elevation
                + (config.deep_ocean_elevation - config.ridge_elevation) * progress
        }
    }
}

/// The per-cell continental base the margin taper leaves, and the cells it
/// reached. Oceanic cells hold `continental_base` and nothing reads them.
struct ContinentalBase {
    cell_bases: Vec<f32>,
    /// Continental cells within `margin_width_hops` of the ocean, in cell
    /// order. Empty when the width is zero.
    margin_cells: Vec<usize>,
}

/// Tapers `continental_base` down to `margin_edge_elevation` over the
/// outermost continental cells, so that a continent ends in a shelf.
///
/// Hop distance comes from every oceanic cell at once and crosses anything,
/// because a shelf is not a feature of one plate: the ocean a rift opened
/// during the run is measured against the same as any other. A continental
/// cell at hop `h` in `1..=margin_width_hops` stands at
/// `edge + (base - edge) * (h - 1) / width`, so the outermost cell sits at the
/// edge elevation and the step from one hop to the next is one `width`th of
/// the drop. Cells further inland keep `continental_base`. The interpolation
/// is a rational of two small integers, so the result is exact.
fn continental_base_field(
    mesh: &SphereMesh,
    cell_crust: CellCrust<'_>,
    config: BaseElevationConfig,
) -> ContinentalBase {
    let mut cell_bases = vec![config.continental_base; mesh.cell_count()];
    let mut margin_cells = Vec::new();
    // A zero width disables the taper, and the hop distances would go unread.
    if config.margin_width_hops > 0 {
        let oceanic: Vec<_> = (0..mesh.cell_count())
            .filter(|&cell| cell_crust.class(cell) == CrustClass::Oceanic)
            .collect();
        let hops = multi_source_distances(mesh, &oceanic, |_, _| true);
        let drop = config.continental_base - config.margin_edge_elevation;
        let margin = 1..=config.margin_width_hops;
        for (cell, hops) in hops.iter().enumerate() {
            // Every oceanic cell is a source, so hop zero is exactly the ocean
            // and the range excludes it without a second reading of the crust.
            // `None` is a world with no ocean at all, whose continent has no
            // coast and so no margin.
            let Some(hops) = hops.filter(|hops| margin.contains(hops)) else {
                continue;
            };
            let inland = (hops - 1) as f32 / config.margin_width_hops as f32;
            cell_bases[cell] = config.margin_edge_elevation + drop * inland;
            margin_cells.push(cell);
        }
    }
    ContinentalBase {
        cell_bases,
        margin_cells,
    }
}

/// Validates the elevations and the taper here and the interior relief terms
/// in their own module, and returns the octave progression the basement field
/// samples.
fn validate_config(
    config: BaseElevationConfig,
) -> Result<Validated<OctaveConfig>, BaseElevationError> {
    let elevations = [
        config.continental_base,
        config.ridge_elevation,
        config.deep_ocean_elevation,
    ];
    if elevations
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        || config.ridge_elevation < config.deep_ocean_elevation
        || !config.cooling_age.is_finite()
        || config.cooling_age <= 0.0
    {
        return Err(BaseElevationError::InvalidConfig);
    }
    if !config.margin_edge_elevation.is_finite()
        || !(0.0..=1.0).contains(&config.margin_edge_elevation)
        || config.margin_edge_elevation > config.continental_base
    {
        return Err(BaseElevationError::InvalidMarginEdge);
    }
    validate_interior_relief(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        BaseElevationFixture, base_elevation_fixture, base_elevation_fixture_with_crust, mesh,
        no_interior_relief, quantized_fingerprint, reference_base_elevation_config,
        reference_crust_config,
    };
    use crate::{
        CoarseElevationConfig, CrustClassificationConfig, PlateKinematicsConfig,
        SeafloorAgeDiagnostics, is_land,
    };

    /// `continental_fraction` before the margin taper retuned it. The
    /// pre-slice base elevation is pinned over the world that target grew, so
    /// reproducing it means reproducing that world too.
    const PRE_MARGIN_CONTINENTAL_FRACTION: f32 = 0.3;

    /// No taper, which is the pre-slice continental base exactly.
    const NO_MARGIN: usize = 0;

    /// The reference world at the target area the pre-slice field was pinned
    /// over, which is the only place that fingerprint still holds.
    fn pre_margin_fixture() -> BaseElevationFixture {
        base_elevation_fixture_with_crust(CrustClassificationConfig {
            continental_fraction: PRE_MARGIN_CONTINENTAL_FRACTION,
            ..reference_crust_config()
        })
    }

    /// The exact base the taper gives a continental cell at `hops` from the
    /// ocean, written the way the config documentation states it rather than
    /// the way the field computes it.
    fn tapered_base(hops: usize, config: BaseElevationConfig) -> f32 {
        config.margin_edge_elevation
            + (config.continental_base - config.margin_edge_elevation) * (hops - 1) as f32
                / config.margin_width_hops as f32
    }

    /// Hops from the nearest oceanic cell for every cell of a fixture, the
    /// distance the taper is defined against.
    fn ocean_hops(fixture: &BaseElevationFixture) -> Vec<Option<usize>> {
        let oceanic: Vec<_> = (0..fixture.mesh.cell_count())
            .filter(|&cell| fixture.age.cell_ages[cell].is_some())
            .collect();
        multi_source_distances(&fixture.mesh, &oceanic, |_, _| true)
    }

    /// Elevations only, for the fixtures whose crust is chosen rather than
    /// evolved. Ages and births agree because both come from one birth field.
    fn from_ages(ages: Vec<Option<f32>>) -> (SeafloorAge, Vec<Option<f32>>) {
        let cell_birth = ages.iter().map(|age| age.map(|_| 0.0)).collect();
        (
            SeafloorAge {
                cell_ages: ages,
                diagnostics: SeafloorAgeDiagnostics::default(),
            },
            cell_birth,
        )
    }

    #[test]
    fn zero_amplitudes_and_no_taper_reproduce_the_curve_alone_deterministically() {
        let fixture = pre_margin_fixture();
        let config = BaseElevationConfig {
            margin_width_hops: NO_MARGIN,
            ..no_interior_relief()
        };
        let first = fixture.derive(config);

        assert_eq!(first, fixture.derive(config));
        assert_eq!(
            first.diagnostics.oceanic_cell_count + first.diagnostics.continental_cell_count,
            fixture.age.cell_ages.len()
        );
        assert_eq!(
            first.diagnostics.dynamic_topography,
            FieldSummary::default()
        );
        assert_eq!(first.diagnostics.basement, FieldSummary::default());
        assert_eq!(first.diagnostics.margin_cell_count, 0);
        assert_eq!(first.diagnostics.margin_depth, FieldSummary::default());
        // Switching both interior-relief terms off and the taper's width to
        // zero leaves the age curve alone, which is what this pin holds. It
        // moves whenever the reference run's crust or its ages do: last with
        // ages becoming model time, which re-dated every cell and scaled the
        // cooling age these fixtures read to match.
        //
        // The curve is add, multiply, divide, and square root over a model
        // time, so it is bit-identical on every machine; the grid is what
        // keeps the pin exact anyway, and it is fine enough to separate the
        // two oldest ages, whose elevations sit 0.0028 apart.
        assert_eq!(
            quantized_fingerprint(first.cell_elevations.iter().copied()),
            8_282_608_213_790_981_193
        );
    }

    #[test]
    fn the_taper_lowers_continental_cells_by_their_hops_from_the_ocean() {
        let fixture = base_elevation_fixture();
        let config = no_interior_relief();
        let hops = ocean_hops(&fixture);
        let base = fixture.derive(config);

        let mut margin_counts = vec![0; config.margin_width_hops + 1];
        for (cell, &hops) in hops.iter().enumerate() {
            let elevation = base.cell_elevations[cell];
            let Some(hops) = hops else {
                panic!("cell {cell} has no ocean to measure against")
            };
            match hops {
                0 => assert_eq!(
                    elevation,
                    cooled(fixture.age.cell_ages[cell], config.continental_base, config),
                    "oceanic cell {cell} left the cooling curve"
                ),
                hops if hops <= config.margin_width_hops => {
                    assert_eq!(
                        elevation,
                        tapered_base(hops, config),
                        "margin cell {cell} at {hops} hops"
                    );
                    margin_counts[hops] += 1;
                }
                _ => assert_eq!(
                    elevation, config.continental_base,
                    "interior cell {cell} at {hops} hops"
                ),
            }
        }

        assert_eq!(
            base.diagnostics.margin_cell_count,
            margin_counts.iter().sum::<usize>()
        );
        assert!(
            margin_counts[1..].iter().all(|&count| count > 0),
            "some hop carries no margin cell: {margin_counts:?}"
        );
        // Strictly increasing in hops, which is what makes the shelf a slope
        // rather than a step: the outermost cell alone sits at the edge.
        assert_eq!(tapered_base(1, config), config.margin_edge_elevation);
        for hops in 2..=config.margin_width_hops {
            assert!(
                tapered_base(hops - 1, config) < tapered_base(hops, config),
                "hop {hops} does not rise above its neighbor"
            );
        }
        assert_eq!(
            base.diagnostics.margin_depth.maximum,
            config.continental_base - config.margin_edge_elevation
        );
    }

    #[test]
    fn sea_level_floods_the_shelf_one_hop_at_a_time() {
        let fixture = base_elevation_fixture();
        let config = no_interior_relief();
        let hops = ocean_hops(&fixture);
        let base = fixture.derive(config);
        let continental = base.diagnostics.continental_cell_count;
        let at_hops = |wanted: usize| hops.iter().filter(|hops| **hops == Some(wanted)).count();
        let land_cells = |sea_level: f32| {
            base.cell_elevations
                .iter()
                .filter(|&&elevation| is_land(elevation, sea_level))
                .count()
        };

        // The three defaults put the margin at 0.46, 0.5233 and 0.5867, so
        // each of these datums drowns one more hop of it and nothing else.
        assert_eq!(land_cells(0.45), continental);
        assert_eq!(land_cells(0.50), continental - at_hops(1));
        assert_eq!(land_cells(0.55), continental - at_hops(1) - at_hops(2));
    }

    #[test]
    fn crust_born_during_the_run_grows_shelves_on_its_own_margins() {
        let fixture = base_elevation_fixture();
        let hops = ocean_hops(&fixture);
        let width = reference_base_elevation_config().margin_width_hops;
        let margin = |cell: usize| hops[cell].is_some_and(|hops| (1..=width).contains(&hops));

        let mut checked = 0;
        for (cell, &birth) in fixture.cell_birth.iter().enumerate() {
            // Crust the run made, rather than crust the prior placed at or
            // before step zero: a rift's new ocean floor.
            if birth.is_none_or(|birth| birth <= 0.0) {
                continue;
            }
            for corner in fixture.mesh.cell_corners(cell) {
                if fixture.cell_birth[corner.neighbor].is_none() {
                    assert!(
                        margin(corner.neighbor),
                        "continental cell {} beside crust born at {birth:?} is not a margin cell",
                        corner.neighbor
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 0, "the reference run made no new ocean floor");
    }

    #[test]
    fn interior_relief_is_deterministic_and_moves_the_field() {
        let fixture = base_elevation_fixture();
        let config = reference_base_elevation_config();
        let first = fixture.derive(config);

        assert_eq!(first, fixture.derive(config));
        assert_ne!(first, fixture.derive(no_interior_relief()));
        assert_ne!(
            first,
            fixture.derive(BaseElevationConfig { seed: 1, ..config })
        );
        assert_ne!(
            first,
            fixture.derive(BaseElevationConfig {
                margin_width_hops: NO_MARGIN,
                ..config
            })
        );
    }

    #[test]
    fn age_option_selects_continental_base_or_oceanic_cooling_curve() {
        let mesh = mesh(32);
        let (age, cell_birth) = from_ages(
            (0..mesh.cell_count())
                .map(|cell| match cell {
                    0 => None,
                    1 => Some(0.0),
                    2 => Some(2.0),
                    3 => Some(8.0),
                    _ => Some(20.0),
                })
                .collect(),
        );
        let config = BaseElevationConfig {
            continental_base: 0.7,
            ridge_elevation: 0.3,
            deep_ocean_elevation: 0.1,
            cooling_age: 8.0,
            // The one continental cell here is surrounded by ocean, so a
            // taper would put it on the shelf edge rather than the base this
            // test is about.
            margin_width_hops: NO_MARGIN,
            ..no_interior_relief()
        };

        let base = BaseElevationFixture {
            mesh,
            age,
            cell_birth,
            flow: FlowField::new(&PlateKinematicsConfig::new(7)),
        }
        .derive(config);
        assert_eq!(base.cell_elevations[0], config.continental_base);
        assert_eq!(base.cell_elevations[1], config.ridge_elevation);
        assert!((base.cell_elevations[2] - 0.2).abs() < f32::EPSILON);
        assert_eq!(base.cell_elevations[3], config.deep_ocean_elevation);
        assert_eq!(base.cell_elevations[4], config.deep_ocean_elevation);
        assert_eq!(base.diagnostics.continental_cell_count, 1);
    }

    #[test]
    fn crust_older_than_the_cooling_age_sits_on_the_deep_floor() {
        let mesh = mesh(32);
        let (age, cell_birth) = from_ages(vec![Some(13.0); mesh.cell_count()]);
        let config = BaseElevationConfig {
            cooling_age: 8.0,
            ..no_interior_relief()
        };
        let base = BaseElevationFixture {
            mesh,
            age,
            cell_birth,
            flow: FlowField::new(&PlateKinematicsConfig::new(7)),
        }
        .derive(config);

        assert!(
            base.cell_elevations
                .iter()
                .all(|&value| value == config.deep_ocean_elevation)
        );
        assert_eq!(base.diagnostics.continental_cell_count, 0);
    }

    #[test]
    fn the_default_field_is_normalized_and_leaves_every_continental_interior_above_sea_level() {
        let fixture = base_elevation_fixture();
        let config = reference_base_elevation_config();
        let base = fixture.derive(config);
        let hops = ocean_hops(&fixture);
        let sea_level = CoarseElevationConfig::default().sea_level;

        let mut flooded_margin = 0;
        for (cell, &elevation) in base.cell_elevations.iter().enumerate() {
            assert!((0.0..=1.0).contains(&elevation), "cell {cell}: {elevation}");
            if fixture.age.cell_ages[cell].is_some() {
                continue;
            }
            if hops[cell].is_some_and(|hops| hops <= config.margin_width_hops) {
                flooded_margin += usize::from(!is_land(elevation, sea_level));
            } else {
                // Interior relief is bounded against the untapered base, so
                // only the shelf may go under.
                assert!(
                    is_land(elevation, sea_level),
                    "continental interior cell {cell}: {elevation}"
                );
            }
        }
        assert!(
            flooded_margin > 0,
            "no shelf cell is under the default datum"
        );
        assert!(base.diagnostics.summary.minimum < base.diagnostics.summary.maximum);
    }

    #[test]
    fn rejects_invalid_configuration() {
        let fixture = base_elevation_fixture();
        let base = BaseElevationConfig::default();
        let cases = [
            (
                BaseElevationConfig {
                    cooling_age: 0.0,
                    ..base
                },
                BaseElevationError::InvalidConfig,
            ),
            (
                BaseElevationConfig {
                    ridge_elevation: 0.0,
                    deep_ocean_elevation: 0.5,
                    ..base
                },
                BaseElevationError::InvalidConfig,
            ),
            (
                BaseElevationConfig {
                    margin_edge_elevation: f32::NAN,
                    ..base
                },
                BaseElevationError::InvalidMarginEdge,
            ),
            (
                BaseElevationConfig {
                    margin_edge_elevation: 1.5,
                    ..base
                },
                BaseElevationError::InvalidMarginEdge,
            ),
            (
                BaseElevationConfig {
                    margin_edge_elevation: base.continental_base + 0.01,
                    ..base
                },
                BaseElevationError::InvalidMarginEdge,
            ),
            (
                BaseElevationConfig {
                    dynamic_topography_amplitude: -0.1,
                    ..base
                },
                BaseElevationError::InvalidInteriorAmplitude,
            ),
            (
                BaseElevationConfig {
                    basement_amplitude: f32::NAN,
                    ..base
                },
                BaseElevationError::InvalidInteriorAmplitude,
            ),
            (
                BaseElevationConfig {
                    basement_frequency: 0.0,
                    ..base
                },
                BaseElevationError::InvalidBasementFrequency,
            ),
            (
                BaseElevationConfig {
                    basement_frequency: f32::INFINITY,
                    ..base
                },
                BaseElevationError::InvalidBasementFrequency,
            ),
        ];

        for (config, expected) in cases {
            assert_eq!(
                derive_base_elevation(
                    &fixture.mesh,
                    &fixture.age,
                    CellCrust {
                        cell_birth: &fixture.cell_birth,
                    },
                    &fixture.flow,
                    config,
                ),
                Err(expected),
                "{config:?}"
            );
        }
    }

    #[test]
    fn rejects_mismatched_inputs() {
        let fixture = base_elevation_fixture();
        let config = BaseElevationConfig::default();
        let crust = CellCrust {
            cell_birth: &fixture.cell_birth,
        };

        assert_eq!(
            derive_base_elevation(
                &fixture.mesh,
                &SeafloorAge {
                    cell_ages: fixture.age.cell_ages[1..].to_vec(),
                    diagnostics: fixture.age.diagnostics,
                },
                crust,
                &fixture.flow,
                config,
            ),
            Err(BaseElevationError::Input(StageInputError::SeafloorAge))
        );
        assert_eq!(
            derive_base_elevation(
                &fixture.mesh,
                &fixture.age,
                CellCrust {
                    cell_birth: &fixture.cell_birth[1..],
                },
                &fixture.flow,
                config,
            ),
            Err(BaseElevationError::Input(StageInputError::CrustBirth))
        );
    }
}
