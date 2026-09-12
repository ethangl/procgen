use crate::{
    HotspotField,
    field::{GeologyInputError, MaxWinsField, peak_count_in_cell, position_in_cell},
};
use procgen_core::{
    RandomStream, Vec3,
    random_streams::{OCEANIC_PEAK_POSITION, OCEANIC_PEAK_PRESENCE},
};
use procgen_sphere_mesh::{SphereMesh, default_cell_area};
use procgen_tectonics::{DEFAULT_STEP_DURATION, FieldSummary, SeafloorAge, StageInputError};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OceanicPeakFieldConfig {
    /// Oldest seafloor age, as model time since birth, eligible for abyssal
    /// hills. Hills form at the ridge and fade under sediment over a long
    /// stretch of a floor's life rather than the youngest few percent of it,
    /// so the default is a quarter of `BaseElevationConfig::cooling_age`.
    ///
    /// It is a time, so the window it opens is the same stretch of a floor's
    /// life whatever step an evolution ran at.
    pub maximum_young_age: f32,
    /// Maximum seamount candidate density, per cell of the default mesh. A
    /// cell of another size carries the same peaks per unit area, so it draws
    /// in proportion to its own area.
    pub seamount_density_scale: f32,
    /// Maximum abyssal-hill candidate density, per cell of the default mesh,
    /// read the same way as `seamount_density_scale`.
    pub abyssal_hill_density_scale: f32,
    /// Largest convex offset from the cell center toward a pair of corners.
    pub maximum_position_offset: f32,
    /// Unitless peak height at full seamount strength.
    pub maximum_seamount_height: f32,
    /// Unitless peak height at full abyssal-hill strength.
    pub maximum_abyssal_hill_height: f32,
    pub seed: u64,
}

impl OceanicPeakFieldConfig {
    pub const fn new(seed: u64) -> Self {
        Self {
            maximum_young_age: 10.0 * DEFAULT_STEP_DURATION,
            seamount_density_scale: 0.75,
            abyssal_hill_density_scale: 0.35,
            maximum_position_offset: 0.8,
            maximum_seamount_height: 1.0,
            maximum_abyssal_hill_height: 0.25,
            seed,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OceanicPeakKind {
    Seamount,
    AbyssalHill,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OceanicPeak {
    pub cell: usize,
    pub kind: OceanicPeakKind,
    /// Seeded surface position guaranteed to remain inside `cell`.
    pub position: Vec3,
    /// Winning normalized density at this cell.
    pub strength: f32,
    /// Unitless diagnostic height scaled from `strength`; no elevation is mutated.
    pub height: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OceanicPeakDiagnostics {
    pub oceanic_cell_count: usize,
    pub hotspot_candidate_cell_count: usize,
    pub young_seafloor_candidate_cell_count: usize,
    pub overlap_cell_count: usize,
    pub density: FieldSummary,
    pub peak_count: usize,
    pub seamount_peak_count: usize,
    pub abyssal_hill_peak_count: usize,
    pub height: FieldSummary,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OceanicPeakField {
    /// Deterministic max-wins candidate density. Equal density favors seamounts.
    pub cell_densities: Vec<f32>,
    pub cell_kinds: Vec<Option<OceanicPeakKind>>,
    /// Sparse, ascending-cell peak candidates independent of elevation and rendering.
    pub peaks: Vec<OceanicPeak>,
    pub diagnostics: OceanicPeakDiagnostics,
}

impl OceanicPeakField {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), GeologyInputError> {
        if self.cell_densities.len() != mesh.cell_count()
            || self.cell_kinds.len() != mesh.cell_count()
            || self.peaks.iter().any(|peak| peak.cell >= mesh.cell_count())
        {
            return Err(GeologyInputError::OceanicPeaks);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OceanicPeakFieldError {
    Input(StageInputError),
    Geology(GeologyInputError),
    EmptyYoungAgeRange,
    InvalidDensityScale,
    InvalidPositionOffset,
    InvalidHeight,
}

impl fmt::Display for OceanicPeakFieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => error.fmt(formatter),
            Self::Geology(error) => error.fmt(formatter),
            Self::EmptyYoungAgeRange => {
                formatter.write_str("maximum young seafloor age must be at least one")
            }
            Self::InvalidDensityScale => {
                formatter.write_str("density scales must be finite and between zero and one")
            }
            Self::InvalidPositionOffset => formatter
                .write_str("maximum position offset must be finite and between zero and one"),
            Self::InvalidHeight => {
                formatter.write_str("maximum peak heights must be finite and nonnegative")
            }
        }
    }
}

impl std::error::Error for OceanicPeakFieldError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            Self::Geology(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StageInputError> for OceanicPeakFieldError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
    }
}

impl From<GeologyInputError> for OceanicPeakFieldError {
    fn from(error: GeologyInputError) -> Self {
        Self::Geology(error)
    }
}

/// Derives sparse seamount and abyssal-hill peak candidates from existing
/// hotspot intensity and seafloor age. It never reads or mutates elevation.
pub fn derive_oceanic_peak_field(
    mesh: &SphereMesh,
    hotspots: &HotspotField,
    seafloor_age: &SeafloorAge,
    config: OceanicPeakFieldConfig,
) -> Result<OceanicPeakField, OceanicPeakFieldError> {
    validate_inputs(mesh, hotspots, seafloor_age, config)?;

    let mut aggregate = MaxWinsField::new(mesh.cell_count());
    let mut hotspot_candidate_cell_count = 0;
    let mut young_seafloor_candidate_cell_count = 0;
    let mut oceanic_cell_count = 0;

    for cell in 0..mesh.cell_count() {
        let Some(age) = seafloor_age.cell_ages[cell] else {
            continue;
        };
        oceanic_cell_count += 1;

        let seamount_density =
            (hotspots.cell_intensities[cell] * config.seamount_density_scale).clamp(0.0, 1.0);
        if seamount_density > 0.0 {
            hotspot_candidate_cell_count += 1;
            aggregate.claim(cell, seamount_density, OceanicPeakKind::Seamount);
        }

        if age > 0.0 && age <= config.maximum_young_age {
            // Full strength at the ridge, falling to nothing at the window's
            // edge. A cell exactly at the edge has no hills, which is what
            // makes the window closed rather than a step down to a floor.
            let age_strength = 1.0 - age / config.maximum_young_age;
            let hill_density = age_strength * config.abyssal_hill_density_scale;
            if hill_density > 0.0 {
                young_seafloor_candidate_cell_count += 1;
                aggregate.claim(cell, hill_density, OceanicPeakKind::AbyssalHill);
            }
        }
    }

    let overlap_cell_count = aggregate.overlap_cell_count();
    let (cell_densities, cell_kinds) = aggregate.into_parts();
    let presence = RandomStream::new(config.seed, OCEANIC_PEAK_PRESENCE);
    let positions = RandomStream::new(config.seed, OCEANIC_PEAK_POSITION);
    let mut peaks = Vec::new();
    for (cell, (&kind, &strength)) in cell_kinds.iter().zip(&cell_densities).enumerate() {
        let Some(kind) = kind else {
            continue;
        };
        // A density is per unit area, so a cell holds peaks in proportion to
        // the area it covers: on a mesh twice as fine each cell carries half
        // as many and the floor carries the same seamounts. The densities are
        // stated against one cell of the default mesh, so a cell of that size
        // carries exactly its own density.
        let expected = strength * mesh.unit_cell_area(cell) / default_cell_area();
        let height = strength
            * match kind {
                OceanicPeakKind::Seamount => config.maximum_seamount_height,
                OceanicPeakKind::AbyssalHill => config.maximum_abyssal_hill_height,
            };
        peaks.extend(
            (0..peak_count_in_cell(expected, presence, cell)).map(|peak| OceanicPeak {
                cell,
                kind,
                position: position_in_cell(
                    mesh,
                    cell,
                    positions,
                    peak,
                    config.maximum_position_offset,
                ),
                strength,
                height,
            }),
        );
    }
    let seamount_peak_count = peaks
        .iter()
        .filter(|peak| peak.kind == OceanicPeakKind::Seamount)
        .count();
    let abyssal_hill_peak_count = peaks.len() - seamount_peak_count;
    let heights: Vec<_> = peaks.iter().map(|peak| peak.height).collect();

    Ok(OceanicPeakField {
        diagnostics: OceanicPeakDiagnostics {
            oceanic_cell_count,
            hotspot_candidate_cell_count,
            young_seafloor_candidate_cell_count,
            overlap_cell_count,
            density: FieldSummary::from_values(&cell_densities),
            peak_count: peaks.len(),
            seamount_peak_count,
            abyssal_hill_peak_count,
            height: FieldSummary::from_values(&heights),
        },
        cell_densities,
        cell_kinds,
        peaks,
    })
}

fn validate_inputs(
    mesh: &SphereMesh,
    hotspots: &HotspotField,
    seafloor_age: &SeafloorAge,
    config: OceanicPeakFieldConfig,
) -> Result<(), OceanicPeakFieldError> {
    if !config.maximum_young_age.is_finite() || config.maximum_young_age <= 0.0 {
        return Err(OceanicPeakFieldError::EmptyYoungAgeRange);
    }
    if [
        config.seamount_density_scale,
        config.abyssal_hill_density_scale,
    ]
    .into_iter()
    .any(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return Err(OceanicPeakFieldError::InvalidDensityScale);
    }
    if !config.maximum_position_offset.is_finite()
        || !(0.0..=1.0).contains(&config.maximum_position_offset)
    {
        return Err(OceanicPeakFieldError::InvalidPositionOffset);
    }
    if [
        config.maximum_seamount_height,
        config.maximum_abyssal_hill_height,
    ]
    .into_iter()
    .any(|height| !height.is_finite() || height < 0.0)
    {
        return Err(OceanicPeakFieldError::InvalidHeight);
    }
    hotspots.validate(mesh)?;
    seafloor_age.validate(mesh)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::density_per_cell;
    use procgen_core::fingerprint;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::build_sphere_mesh;

    /// The fixture mesh's cell count. Its cells are 256 times wider than the
    /// default mesh's, so the densities below are stated against them.
    const CELL_COUNT: usize = 256;

    /// The default densities at this fixture's resolution: a cell here draws
    /// as often as a cell of the default mesh does at the crate's defaults.
    /// Stated per cell, because a mesh this coarse cannot express the peaks
    /// per unit area the defaults ask for.
    fn reference_config(seed: u64) -> OceanicPeakFieldConfig {
        let default = OceanicPeakFieldConfig::new(seed);
        OceanicPeakFieldConfig {
            seamount_density_scale: density_per_cell(CELL_COUNT, default.seamount_density_scale),
            abyssal_hill_density_scale: density_per_cell(
                CELL_COUNT,
                default.abyssal_hill_density_scale,
            ),
            ..default
        }
    }

    fn mesh() -> SphereMesh {
        build_sphere_mesh(
            fibonacci_sphere(FibonacciConfig {
                count: CELL_COUNT,
                jitter: 0.5,
                seed: 7,
            })
            .unwrap(),
            1.0,
        )
        .unwrap()
    }

    fn inputs(mesh: &SphereMesh) -> (HotspotField, SeafloorAge) {
        let mut intensities = vec![0.0; mesh.cell_count()];
        for (cell, intensity) in intensities.iter_mut().enumerate() {
            if cell % 3 == 0 {
                *intensity = (cell % 11 + 1) as f32 / 11.0;
            }
        }
        let hotspots = HotspotField {
            hotspots: Vec::new(),
            cell_intensities: intensities,
            cell_hotspots: vec![None; mesh.cell_count()],
            cell_plateau: vec![0.0; mesh.cell_count()],
            diagnostics: Default::default(),
        };
        // Ages a default step apart, so the spread sits inside the default
        // young-age window exactly as the step counts it replaces did.
        let seafloor_age = SeafloorAge {
            cell_ages: (0..mesh.cell_count())
                .map(|cell| (cell % 7 != 0).then_some((cell % 8) as f32 * DEFAULT_STEP_DURATION))
                .collect(),
            diagnostics: Default::default(),
        };
        (hotspots, seafloor_age)
    }

    /// A cell whose share of the density is more than one peak carries every
    /// peak it asks for. The rule drew once per cell before, so a mesh whose
    /// cells are larger than a peak's share of area could place only one and
    /// the count saturated at the candidate cell count instead of following
    /// the density.
    #[test]
    fn a_cell_carries_every_peak_its_area_asks_for() {
        let mesh = mesh();
        let (hotspots, ages) = inputs(&mesh);
        // The crate's own densities, which are stated per cell of the default
        // mesh. A cell here covers 256 of those, so most candidate cells ask
        // for many peaks.
        let config = OceanicPeakFieldConfig::new(19);
        let field = derive_oceanic_peak_field(&mesh, &hotspots, &ages, config).unwrap();

        let mut counts = vec![0usize; mesh.cell_count()];
        for peak in &field.peaks {
            counts[peak.cell] += 1;
        }
        let mut saturating_cells = 0;
        let mut expected_total = 0.0_f64;
        for (cell, (kind, &strength)) in field
            .cell_kinds
            .iter()
            .zip(&field.cell_densities)
            .enumerate()
        {
            if kind.is_none() {
                assert_eq!(counts[cell], 0, "cell {cell} has peaks with no kind");
                continue;
            }
            let expected = strength * mesh.unit_cell_area(cell) / default_cell_area();
            expected_total += f64::from(expected);
            saturating_cells += usize::from(expected > 1.0);
            // Exactly the whole part, and one more or not as the fraction
            // decides. Nothing else is a count this rule can produce.
            let whole = expected.floor() as usize;
            assert!(
                counts[cell] == whole || counts[cell] == whole + 1,
                "cell {cell} asked for {expected} and carries {}",
                counts[cell]
            );
        }
        // The old rule could not have placed these: one draw a cell caps the
        // count at the number of candidate cells.
        assert!(saturating_cells > 0, "the fixture must saturate somewhere");
        assert!(
            field.peaks.len() > field.diagnostics.oceanic_cell_count,
            "{} peaks over {} oceanic cells is not past the old cap",
            field.peaks.len(),
            field.diagnostics.oceanic_cell_count
        );
        // The total is the density's own answer, to within the one fractional
        // peak a cell rounds by.
        let slack = mesh.cell_count() as f64;
        assert!(
            (field.peaks.len() as f64 - expected_total).abs() < slack,
            "{} peaks against an expected {expected_total}",
            field.peaks.len()
        );

        // Two peaks of one cell are two peaks, so they sit at their own
        // positions rather than on top of each other.
        let crowded = (0..mesh.cell_count())
            .find(|&cell| counts[cell] > 1)
            .expect("a cell must carry more than one peak");
        let positions: Vec<_> = field
            .peaks
            .iter()
            .filter(|peak| peak.cell == crowded)
            .map(|peak| peak.position)
            .collect();
        for (index, position) in positions.iter().enumerate() {
            assert!(
                positions[index + 1..]
                    .iter()
                    .all(|other| (*other - *position).length() > 1.0e-6),
                "cell {crowded} stacked two peaks at one point"
            );
        }
    }

    #[test]
    fn field_is_deterministic_seeded_sparse_and_position_bounded() {
        let mesh = mesh();
        let (hotspots, ages) = inputs(&mesh);
        let config = reference_config(19);
        let first = derive_oceanic_peak_field(&mesh, &hotspots, &ages, config).unwrap();

        assert_eq!(
            first,
            derive_oceanic_peak_field(&mesh, &hotspots, &ages, config).unwrap()
        );
        let reseeded = derive_oceanic_peak_field(
            &mesh,
            &hotspots,
            &ages,
            OceanicPeakFieldConfig { seed: 20, ..config },
        )
        .unwrap();
        assert_eq!(first.cell_densities, reseeded.cell_densities);
        assert_eq!(first.cell_kinds, reseeded.cell_kinds);
        assert_ne!(first.peaks, reseeded.peaks);
        assert!(first.peaks.len() < first.diagnostics.oceanic_cell_count);
        // Ascending, and at this density one peak a cell at most, which is
        // all a cell of a mesh this coarse asks for once the density is
        // stated against its own area.
        assert!(
            first
                .peaks
                .windows(2)
                .all(|pair| pair[0].cell < pair[1].cell)
        );

        for peak in &first.peaks {
            assert!((peak.position.length() - mesh.radius).abs() < 1.0e-6);
            let center = mesh.cell_centers[peak.cell].normalized();
            for corner in mesh.cell_corners(peak.cell) {
                let neighbor = mesh.cell_centers[corner.neighbor].normalized();
                assert!(peak.position.dot(center) + 1.0e-6 >= peak.position.dot(neighbor));
            }
        }
    }

    #[test]
    fn reference_field_has_stable_fingerprint() {
        let mesh = mesh();
        let (hotspots, ages) = inputs(&mesh);
        let field =
            derive_oceanic_peak_field(&mesh, &hotspots, &ages, reference_config(19)).unwrap();
        let values = field.peaks.iter().flat_map(|peak| {
            [
                peak.cell as u64,
                match peak.kind {
                    OceanicPeakKind::Seamount => 0,
                    OceanicPeakKind::AbyssalHill => 1,
                },
            ]
        });

        // Moved a second time when the presence draw became a density per
        // unit area. A cell of exactly the mean area draws what it drew; every
        // other cell shifts by its own area over the mean, which over this
        // fixture's 201 candidates loses four peaks and gains five. That is
        // the whole of the change, and it is what makes a floor carry the same
        // seamounts per square kilometre on any mesh.
        assert_eq!(fingerprint(values), 11_561_668_630_552_071_035);
    }

    #[test]
    fn dependencies_and_max_overlap_are_explicit() {
        let mesh = mesh();
        let (mut hotspots, mut ages) = inputs(&mesh);
        hotspots.cell_intensities.fill(0.0);
        ages.cell_ages.fill(None);
        hotspots.cell_intensities[0] = 1.0;
        hotspots.cell_intensities[1] = 0.4;
        hotspots.cell_intensities[2] = 1.0;
        // A window of one and ages at halves of it, so every strength the
        // ramp produces here is exact.
        ages.cell_ages[1] = Some(0.5);
        ages.cell_ages[2] = Some(0.25);
        ages.cell_ages[3] = Some(0.0);
        ages.cell_ages[4] = Some(2.0);
        let config = OceanicPeakFieldConfig {
            maximum_young_age: 1.0,
            seamount_density_scale: 0.75,
            abyssal_hill_density_scale: 0.75,
            ..reference_config(7)
        };
        let field = derive_oceanic_peak_field(&mesh, &hotspots, &ages, config).unwrap();

        assert_eq!(
            field.cell_kinds[0], None,
            "hotspots require oceanic age data"
        );
        assert_eq!(field.cell_kinds[1], Some(OceanicPeakKind::AbyssalHill));
        assert_eq!(field.cell_densities[1], 0.375);
        assert_eq!(field.cell_kinds[2], Some(OceanicPeakKind::Seamount));
        assert_eq!(field.cell_densities[2], 0.75);
        assert_eq!(field.cell_kinds[3], None, "ridge age zero is excluded");
        assert_eq!(field.cell_kinds[4], None, "old seafloor is excluded");
        assert_eq!(field.diagnostics.overlap_cell_count, 2);
    }

    #[test]
    fn ties_favor_seamounts_and_heights_scale_with_strength() {
        let mesh = mesh();
        let (mut hotspots, mut ages) = inputs(&mesh);
        hotspots.cell_intensities.fill(0.0);
        ages.cell_ages.fill(None);
        // The ramp reaches full strength only at age zero, which is outside
        // the window, so the tie is arranged just inside it: a hotspot of
        // fifteen sixteenths against a floor a sixteenth of the way through.
        // It has to stay above this seed's presence draw for the peak to be
        // placed at all.
        hotspots.cell_intensities[0] = 0.937_5;
        ages.cell_ages[0] = Some(0.062_5);
        let config = OceanicPeakFieldConfig {
            maximum_young_age: 1.0,
            seamount_density_scale: 1.0,
            abyssal_hill_density_scale: 1.0,
            maximum_seamount_height: 0.5,
            ..reference_config(1)
        };
        let field = derive_oceanic_peak_field(&mesh, &hotspots, &ages, config).unwrap();

        assert_eq!(field.cell_kinds[0], Some(OceanicPeakKind::Seamount));
        let peak = field.peaks.iter().find(|peak| peak.cell == 0).unwrap();
        assert_eq!(peak.strength, 0.937_5);
        assert_eq!(peak.height, 0.468_75);
    }

    #[test]
    fn empty_inputs_and_invalid_edges_are_handled() {
        let mesh = mesh();
        let (mut hotspots, mut ages) = inputs(&mesh);
        hotspots.cell_intensities.fill(0.0);
        ages.cell_ages.fill(None);
        let empty =
            derive_oceanic_peak_field(&mesh, &hotspots, &ages, reference_config(7)).unwrap();
        assert!(empty.peaks.is_empty());
        assert!(empty.cell_kinds.iter().all(Option::is_none));
        assert_eq!(empty.diagnostics, OceanicPeakDiagnostics::default());

        assert_eq!(
            derive_oceanic_peak_field(
                &mesh,
                &hotspots,
                &ages,
                OceanicPeakFieldConfig {
                    maximum_young_age: 0.0,
                    ..reference_config(7)
                }
            ),
            Err(OceanicPeakFieldError::EmptyYoungAgeRange)
        );
        hotspots.cell_intensities.pop();
        assert_eq!(
            derive_oceanic_peak_field(&mesh, &hotspots, &ages, reference_config(7)),
            Err(OceanicPeakFieldError::Geology(GeologyInputError::Hotspots))
        );

        let (hotspots, mut ages) = inputs(&mesh);
        ages.cell_ages.pop();
        assert_eq!(
            derive_oceanic_peak_field(&mesh, &hotspots, &ages, reference_config(7)),
            Err(OceanicPeakFieldError::Input(StageInputError::SeafloorAge))
        );
    }
}
