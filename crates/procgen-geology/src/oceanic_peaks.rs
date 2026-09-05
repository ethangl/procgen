use crate::{
    HotspotField,
    field::{GeologyInputError, MaxWinsField},
};
use procgen_core::{
    RandomStream, Vec3,
    random_streams::{OCEANIC_PEAK_POSITION, OCEANIC_PEAK_PRESENCE},
};
use procgen_sphere_mesh::SphereMesh;
use procgen_tectonics::{FieldSummary, SeafloorAge, StageInputError};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OceanicPeakFieldConfig {
    /// Oldest nonzero hop age eligible for abyssal hills.
    pub maximum_young_age: usize,
    /// Maximum per-cell seamount candidate density.
    pub seamount_density_scale: f32,
    /// Maximum per-cell abyssal-hill candidate density.
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
            maximum_young_age: 4,
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

        if (1..=config.maximum_young_age).contains(&age) {
            let age_strength =
                (config.maximum_young_age + 1 - age) as f32 / config.maximum_young_age as f32;
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
    let peaks: Vec<_> = (0..mesh.cell_count())
        .filter_map(|cell| {
            let kind = cell_kinds[cell]?;
            let strength = cell_densities[cell];
            (presence.unit_f32(cell as u64, 0) < strength).then(|| OceanicPeak {
                cell,
                kind,
                position: position_in_cell(mesh, cell, positions, config.maximum_position_offset),
                strength,
                height: strength
                    * match kind {
                        OceanicPeakKind::Seamount => config.maximum_seamount_height,
                        OceanicPeakKind::AbyssalHill => config.maximum_abyssal_hill_height,
                    },
            })
        })
        .collect();
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
    if config.maximum_young_age == 0 {
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

fn position_in_cell(
    mesh: &SphereMesh,
    cell: usize,
    stream: RandomStream,
    maximum_offset: f32,
) -> Vec3 {
    let corners = mesh.cell_corners(cell);
    let corner_index = stream.sample_u64(cell as u64, 0) as usize % corners.len();
    let mut first_weight = stream.unit_f32(cell as u64, 1);
    let mut second_weight = stream.unit_f32(cell as u64, 2);
    if first_weight + second_weight > 1.0 {
        first_weight = 1.0 - first_weight;
        second_weight = 1.0 - second_weight;
    }
    first_weight *= maximum_offset;
    second_weight *= maximum_offset;
    mesh.interpolate_cell_triangle(
        cell,
        corner_index,
        [
            1.0 - first_weight - second_weight,
            first_weight,
            second_weight,
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_core::fingerprint;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::build_sphere_mesh;

    fn mesh() -> SphereMesh {
        build_sphere_mesh(
            fibonacci_sphere(FibonacciConfig {
                count: 256,
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
            diagnostics: Default::default(),
        };
        let seafloor_age = SeafloorAge {
            cell_ages: (0..mesh.cell_count())
                .map(|cell| (cell % 7 != 0).then_some(cell % 8))
                .collect(),
            diagnostics: Default::default(),
        };
        (hotspots, seafloor_age)
    }

    #[test]
    fn field_is_deterministic_seeded_sparse_and_position_bounded() {
        let mesh = mesh();
        let (hotspots, ages) = inputs(&mesh);
        let config = OceanicPeakFieldConfig::new(19);
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
            derive_oceanic_peak_field(&mesh, &hotspots, &ages, OceanicPeakFieldConfig::new(19))
                .unwrap();
        let values = field.peaks.iter().flat_map(|peak| {
            [
                peak.cell as u64,
                match peak.kind {
                    OceanicPeakKind::Seamount => 0,
                    OceanicPeakKind::AbyssalHill => 1,
                },
                u64::from(peak.position.x.to_bits()),
                u64::from(peak.position.y.to_bits()),
                u64::from(peak.position.z.to_bits()),
                u64::from(peak.strength.to_bits()),
                u64::from(peak.height.to_bits()),
            ]
        });

        assert_eq!(fingerprint(values), 12_689_882_900_264_755_237);
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
        ages.cell_ages[1] = Some(1);
        ages.cell_ages[2] = Some(2);
        ages.cell_ages[3] = Some(0);
        ages.cell_ages[4] = Some(5);
        let config = OceanicPeakFieldConfig {
            seamount_density_scale: 0.75,
            abyssal_hill_density_scale: 0.75,
            ..OceanicPeakFieldConfig::new(7)
        };
        let field = derive_oceanic_peak_field(&mesh, &hotspots, &ages, config).unwrap();

        assert_eq!(
            field.cell_kinds[0], None,
            "hotspots require oceanic age data"
        );
        assert_eq!(field.cell_kinds[1], Some(OceanicPeakKind::AbyssalHill));
        assert_eq!(field.cell_densities[1], 0.75);
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
        hotspots.cell_intensities[0] = 1.0;
        ages.cell_ages[0] = Some(1);
        let config = OceanicPeakFieldConfig {
            seamount_density_scale: 1.0,
            abyssal_hill_density_scale: 1.0,
            maximum_seamount_height: 0.8,
            ..OceanicPeakFieldConfig::new(1)
        };
        let field = derive_oceanic_peak_field(&mesh, &hotspots, &ages, config).unwrap();

        assert_eq!(field.cell_kinds[0], Some(OceanicPeakKind::Seamount));
        let peak = field.peaks.iter().find(|peak| peak.cell == 0).unwrap();
        assert_eq!(peak.strength, 1.0);
        assert_eq!(peak.height, 0.8);
    }

    #[test]
    fn empty_inputs_and_invalid_edges_are_handled() {
        let mesh = mesh();
        let (mut hotspots, mut ages) = inputs(&mesh);
        hotspots.cell_intensities.fill(0.0);
        ages.cell_ages.fill(None);
        let empty =
            derive_oceanic_peak_field(&mesh, &hotspots, &ages, OceanicPeakFieldConfig::new(7))
                .unwrap();
        assert!(empty.peaks.is_empty());
        assert!(empty.cell_kinds.iter().all(Option::is_none));
        assert_eq!(empty.diagnostics, OceanicPeakDiagnostics::default());

        assert_eq!(
            derive_oceanic_peak_field(
                &mesh,
                &hotspots,
                &ages,
                OceanicPeakFieldConfig {
                    maximum_young_age: 0,
                    ..OceanicPeakFieldConfig::new(7)
                }
            ),
            Err(OceanicPeakFieldError::EmptyYoungAgeRange)
        );
        hotspots.cell_intensities.pop();
        assert_eq!(
            derive_oceanic_peak_field(&mesh, &hotspots, &ages, OceanicPeakFieldConfig::new(7)),
            Err(OceanicPeakFieldError::Geology(GeologyInputError::Hotspots))
        );

        let (hotspots, mut ages) = inputs(&mesh);
        ages.cell_ages.pop();
        assert_eq!(
            derive_oceanic_peak_field(&mesh, &hotspots, &ages, OceanicPeakFieldConfig::new(7)),
            Err(OceanicPeakFieldError::Input(StageInputError::SeafloorAge))
        );
    }
}
