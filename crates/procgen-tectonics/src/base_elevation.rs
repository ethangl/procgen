//! Broad base elevation: what a cell would stand at with no boundary near it.
//!
//! Three fields add. Oceanic crust follows a square-root cooling curve from a
//! ridge down to the deep floor, and continental crust starts from one
//! configured base. Neither varies over a plate interior, so a plate wider
//! than the few cells a boundary deforms would be flat by construction; the
//! other two fields are the interior relief Earth has and those two lack.
//!
//! Dynamic topography is the surface's response to the flow underneath it:
//! where the flow converges the surface sags and where it diverges it swells.
//! The [`FlowField`] the kinematics fit reads stands in for mantle flow, so
//! its divergence over the mesh, negated and normalised, is that response, and
//! it applies to oceanic and continental crust alike.
//!
//! The continental basement is the thickness contrast between old shields and
//! younger provinces: three octaves of the fixed-polynomial gradient noise,
//! spanning a tenth of a great circle down to about a dozen cells of the
//! default mesh, on continental crust only.
//!
//! Both new fields are broad and gentle by design, so that interior relief
//! cannot drown continental crust on its own: with both terms nominally full
//! against it a continental cell stands at `0.65 - 0.03 - 0.05`, which is 0.57
//! and still above the 0.5 of [`crate::SEA_LEVEL`]. That nominal is not a
//! bound, because the dynamic term is normalised by the divergence field's
//! root-mean-square rather than its peak; the measured margin is thinner, and
//! `docs/plate-movement.md` records it. A coast therefore moves only through
//! the oceanic side, which the dynamic term alone touches, and what a
//! continental interior gets is the slope erosion needs and never had.

use crate::{CellCrust, CrustClass, FieldSummary, FlowField, SeafloorAge, StageInputError};
use procgen_core::{RandomStream, random_streams::PLATE_BASEMENT};
use procgen_noise::{
    OctaveConfig, OctaveGain, Validated, amplitude_sum, fbm_3d, fold_seed_u64_to_u32,
};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

/// Root-mean-square of the flow field's mesh divergence, which the dynamic
/// topography term divides by so that its amplitude is the swell a typical
/// divergence raises rather than an arbitrary scale.
///
/// Measured once over the viewer's default mesh and flow field — 65,536 cells,
/// jitter 0.8, sampling seed 7, motion seed 7, flow frequency 1.0 — where the
/// divergence field spans -3.38 to 5.53 with an RMS of 1.206, rounded here.
/// The peak is 4.6 times the RMS, so the term reaches about four and a half
/// times its amplitude at the single most divergent cell.
const DIVERGENCE_SCALE: f32 = 1.2;

/// Octaves of the basement field. Three at halving amplitude and doubling
/// frequency reach a quarter of the configured wavelength, which is where a
/// wavelength stops being basement and starts being terrain detail.
const BASEMENT_OCTAVES: u32 = 3;
const BASEMENT_LACUNARITY: f32 = 2.0;
const BASEMENT_OCTAVE_GAIN: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaseElevationConfig {
    /// Seed of the continental basement field. Nothing else here is hashed.
    pub seed: u64,
    pub continental_base: f32,
    /// Elevation of age-zero oceanic crust at a ridge.
    pub ridge_elevation: f32,
    /// Minimum elevation reached by sufficiently old oceanic crust.
    pub deep_ocean_elevation: f32,
    /// Seafloor age in evolution steps at which oceanic crust reaches the
    /// deep-ocean floor. Age spans one step, for crust born at a ridge during
    /// the run, to the prior's hop age plus the whole run for crust that
    /// predates it, so this belongs near the top of that span: below it the
    /// whole ocean sits on the deep floor and only crust made during the run
    /// carries any gradient.
    pub cooling_age: usize,
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
}

impl Default for BaseElevationConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            continental_base: 0.65,
            ridge_elevation: 0.30,
            deep_ocean_elevation: 0.08,
            cooling_age: 40,
            dynamic_topography_amplitude: 0.03,
            basement_amplitude: 0.05,
            basement_frequency: 3.0,
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
    pub oceanic_cell_count: usize,
    pub continental_cell_count: usize,
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
/// the deep floor; continental crust starts from `continental_base`. Crust
/// that predates the run carries the prior's age plus the steps the run took,
/// so an old ocean reaches the deep floor and crust born at a ridge during the
/// run does not. Onto that go the dynamic topography of the flow field's
/// divergence, everywhere, and the continental basement's octaves, on
/// continental crust alone.
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

    let dynamic_topography = dynamic_topography_field(mesh, flow_field, config);
    let basement = basement_field(mesh, cell_crust, config, octaves);
    let cell_elevations: Vec<_> = seafloor_age
        .cell_ages
        .iter()
        .zip(&dynamic_topography)
        .zip(&basement)
        .map(|((age, &dynamic), &basement)| {
            // The composition stage clamps for the same reason: elevation is
            // normalized and nothing downstream reads a value outside it.
            // Only the deep end of the oceanic curve can reach it, where the
            // floor at 0.08 leaves the dynamic term under three times its own
            // amplitude of room and the field's peak divergence asks for
            // more; the continental base at 0.65 has room for both terms
            // several times over. `docs/plate-movement.md` counts the cells
            // that clamp.
            (cooled(*age, config) + dynamic + basement).clamp(0.0, 1.0)
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
    let oceanic_cell_count = oceanic_elevations.len();
    let diagnostics = BaseElevationDiagnostics {
        summary: FieldSummary::from_values(&cell_elevations),
        oceanic: FieldSummary::from_values(&oceanic_elevations),
        dynamic_topography: FieldSummary::from_values(&dynamic_topography),
        basement: FieldSummary::from_values(&continental_basement),
        oceanic_cell_count,
        continental_cell_count: cell_elevations.len() - oceanic_cell_count,
    };
    Ok(BaseElevation {
        cell_elevations,
        diagnostics,
    })
}

/// The cooling curve's own answer for one cell, before interior relief.
/// `None` is continental crust, which the curve does not describe.
fn cooled(age: Option<usize>, config: BaseElevationConfig) -> f32 {
    match age {
        None => config.continental_base,
        Some(age) if age >= config.cooling_age => config.deep_ocean_elevation,
        Some(age) => {
            let progress = (age as f32 / config.cooling_age as f32).sqrt();
            config.ridge_elevation
                + (config.deep_ocean_elevation - config.ridge_elevation) * progress
        }
    }
}

/// Sag where the flow field converges and swell where it diverges, scaled so
/// that the amplitude is what a typical divergence raises.
fn dynamic_topography_field(
    mesh: &SphereMesh,
    flow_field: &FlowField,
    config: BaseElevationConfig,
) -> Vec<f32> {
    let mut field = mesh.cell_divergence(|direction| flow_field.velocity_at(direction));
    let scale = config.dynamic_topography_amplitude / DIVERGENCE_SCALE;
    field
        .iter_mut()
        // Convergent flow sags the surface, so a positive divergence lifts it.
        .for_each(|value| *value *= -scale);
    field
}

/// The continental basement's octave sum, normalized by its amplitude sum and
/// scaled by the configured relief. Exactly zero on oceanic crust, whose
/// elevation is the cooling curve's to set.
fn basement_field(
    mesh: &SphereMesh,
    cell_crust: CellCrust<'_>,
    config: BaseElevationConfig,
    octaves: Validated<OctaveConfig>,
) -> Vec<f32> {
    let key = fold_seed_u64_to_u32(RandomStream::new(config.seed, PLATE_BASEMENT).sample_u64(0, 0));
    let gain = basement_gain();
    let normalization = config.basement_amplitude / amplitude_sum(octaves, gain);
    (0..mesh.cell_count())
        .map(|cell| match cell_crust.class(cell) {
            CrustClass::Oceanic => 0.0,
            CrustClass::Continental => {
                let direction = mesh.cell_centers[cell].normalized();
                fbm_3d(key, direction, octaves, gain).value * normalization
            }
        })
        .collect()
}

/// Amplitudes halve each octave. Constructed from a constant that the octave
/// count is chosen against, so the conversion cannot fail.
fn basement_gain() -> OctaveGain {
    OctaveGain::new(BASEMENT_OCTAVE_GAIN).expect("a halving gain is in the unit interval")
}

/// Validates the config once and returns the octave progression the basement
/// field samples, so the frequency check and the progression the hot loop uses
/// are the same arithmetic.
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
        || config.cooling_age == 0
    {
        return Err(BaseElevationError::InvalidConfig);
    }
    let amplitudes = [
        config.dynamic_topography_amplitude,
        config.basement_amplitude,
    ];
    if amplitudes
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(BaseElevationError::InvalidInteriorAmplitude);
    }
    OctaveConfig {
        octaves: BASEMENT_OCTAVES,
        frequency: config.basement_frequency,
        lacunarity: BASEMENT_LACUNARITY,
    }
    .validate()
    .map_err(|_| BaseElevationError::InvalidBasementFrequency)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{final_state_fixture, fingerprint, reference_evolution_config};
    use crate::{
        PlateKinematicsConfig, SeafloorAgeDiagnostics, derive_seafloor_age, is_land,
        test_support::mesh,
    };

    /// A mesh, the reference world's final age and crust over it, and the
    /// default flow field.
    struct Fixture {
        mesh: SphereMesh,
        age: SeafloorAge,
        cell_birth: Vec<Option<i32>>,
        flow: FlowField,
    }

    impl Fixture {
        fn derive(&self, config: BaseElevationConfig) -> BaseElevation {
            derive_base_elevation(
                &self.mesh,
                &self.age,
                CellCrust {
                    cell_birth: &self.cell_birth,
                },
                &self.flow,
                config,
            )
            .unwrap()
        }
    }

    fn reference_fixture() -> Fixture {
        let (mesh, _, evolution) = final_state_fixture();
        let age = derive_seafloor_age(&mesh, &evolution, reference_evolution_config().step_count)
            .unwrap();
        Fixture {
            mesh,
            age,
            cell_birth: evolution.cell_birth,
            flow: FlowField::new(&PlateKinematicsConfig::new(7)),
        }
    }

    /// Interior relief switched off, which is the pre-slice field exactly.
    fn no_interior_relief() -> BaseElevationConfig {
        BaseElevationConfig {
            dynamic_topography_amplitude: 0.0,
            basement_amplitude: 0.0,
            ..BaseElevationConfig::default()
        }
    }

    /// Elevations only, for the fixtures whose crust is chosen rather than
    /// evolved. Ages and births agree because both come from one birth field.
    fn from_ages(ages: Vec<Option<usize>>) -> (SeafloorAge, Vec<Option<i32>>) {
        let cell_birth = ages.iter().map(|age| age.map(|_| 0)).collect();
        (
            SeafloorAge {
                cell_ages: ages,
                diagnostics: SeafloorAgeDiagnostics::default(),
            },
            cell_birth,
        )
    }

    #[test]
    fn zero_amplitudes_reproduce_the_curve_alone_deterministically() {
        let fixture = reference_fixture();
        let config = no_interior_relief();
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
        // The fingerprint pinned before interior relief existed: switching
        // both terms off has to leave the field bit for bit as it was.
        assert_eq!(
            fingerprint(
                first
                    .cell_elevations
                    .iter()
                    .map(|value| value.to_bits() as u64)
            ),
            18_424_986_082_501_235_738
        );
    }

    #[test]
    fn interior_relief_is_deterministic_and_moves_the_field() {
        let fixture = reference_fixture();
        let config = BaseElevationConfig::default();
        let first = fixture.derive(config);

        assert_eq!(first, fixture.derive(config));
        assert_ne!(first, fixture.derive(no_interior_relief()));
        assert_ne!(
            first,
            fixture.derive(BaseElevationConfig { seed: 1, ..config })
        );
    }

    #[test]
    fn age_option_selects_continental_base_or_oceanic_cooling_curve() {
        let mesh = mesh(32);
        let (age, cell_birth) = from_ages(
            (0..mesh.cell_count())
                .map(|cell| match cell {
                    0 => None,
                    1 => Some(0),
                    2 => Some(2),
                    3 => Some(8),
                    _ => Some(20),
                })
                .collect(),
        );
        let config = BaseElevationConfig {
            continental_base: 0.7,
            ridge_elevation: 0.3,
            deep_ocean_elevation: 0.1,
            cooling_age: 8,
            ..no_interior_relief()
        };

        let base = Fixture {
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
        let (age, cell_birth) = from_ages(vec![Some(13); mesh.cell_count()]);
        let config = BaseElevationConfig {
            cooling_age: 8,
            ..no_interior_relief()
        };
        let base = Fixture {
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
    fn the_basement_reaches_continental_crust_alone() {
        let fixture = reference_fixture();
        let config = BaseElevationConfig {
            dynamic_topography_amplitude: 0.0,
            ..BaseElevationConfig::default()
        };
        let base = fixture.derive(config);
        let curve = fixture.derive(no_interior_relief());

        let mut moved = 0;
        for (cell, (&with, &without)) in base
            .cell_elevations
            .iter()
            .zip(&curve.cell_elevations)
            .enumerate()
        {
            match fixture.age.cell_ages[cell] {
                Some(_) => assert_eq!(with, without, "cell {cell} is oceanic"),
                None => moved += usize::from(with != without),
            }
        }
        assert!(moved > 0, "no continental cell carries a basement");
        assert!(base.diagnostics.basement.minimum < 0.0);
        assert!(base.diagnostics.basement.maximum > 0.0);
        assert!(base.diagnostics.basement.maximum <= config.basement_amplitude);
    }

    #[test]
    fn dynamic_topography_opposes_divergence_everywhere_and_scales_with_its_amplitude() {
        let fixture = reference_fixture();
        // A third of the default amplitude, so that even the peak divergence
        // of this mesh cannot push the deep floor at 0.08 into the clamp and
        // every cell's whole shift is visible. Doubling it stays clear too.
        let config = BaseElevationConfig {
            dynamic_topography_amplitude: 0.01,
            basement_amplitude: 0.0,
            ..BaseElevationConfig::default()
        };
        let base = fixture.derive(config);
        let doubled = fixture.derive(BaseElevationConfig {
            dynamic_topography_amplitude: config.dynamic_topography_amplitude * 2.0,
            ..config
        });
        let curve = fixture.derive(no_interior_relief());
        let divergence = fixture
            .mesh
            .cell_divergence(|direction| fixture.flow.velocity_at(direction));

        let mut sagged = 0;
        let mut swelled = 0;
        for (cell, &divergence) in divergence.iter().enumerate() {
            let shift = base.cell_elevations[cell] - curve.cell_elevations[cell];
            assert!(
                shift * divergence <= 0.0,
                "cell {cell} moved {shift} where the flow diverges by {divergence}"
            );
            assert!(
                (doubled.cell_elevations[cell] - curve.cell_elevations[cell] - 2.0 * shift).abs()
                    < 1.0e-6,
                "cell {cell} does not scale with the amplitude"
            );
            sagged += usize::from(shift < 0.0);
            swelled += usize::from(shift > 0.0);
        }
        assert!(
            sagged > 0 && swelled > 0,
            "{sagged} sagged, {swelled} swelled"
        );
        assert!(base.diagnostics.dynamic_topography.minimum < 0.0);
        assert!(base.diagnostics.dynamic_topography.maximum > 0.0);
    }

    #[test]
    fn the_default_field_is_normalized_and_leaves_every_continent_above_sea_level() {
        let fixture = reference_fixture();
        let base = fixture.derive(BaseElevationConfig::default());

        for (cell, &elevation) in base.cell_elevations.iter().enumerate() {
            assert!((0.0..=1.0).contains(&elevation), "cell {cell}: {elevation}");
            if fixture.age.cell_ages[cell].is_none() {
                assert!(is_land(elevation), "continental cell {cell}: {elevation}");
            }
        }
        assert!(base.diagnostics.summary.minimum < base.diagnostics.summary.maximum);
    }

    #[test]
    fn rejects_invalid_configuration() {
        let fixture = reference_fixture();
        let base = BaseElevationConfig::default();
        let cases = [
            (
                BaseElevationConfig {
                    cooling_age: 0,
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
        let fixture = reference_fixture();
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
