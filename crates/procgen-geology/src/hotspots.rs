//! Mantle plumes: a point trail behind every hotspot, and a broad flood-basalt
//! province around the continental ones.
//!
//! A trail is the few cells a fixed plume has burned through the plate moving
//! over it, decaying with age. A province is the other thing a plume does, and
//! [`crate::provinces`] owns its shape: only a hashed subset of
//! continental-source hotspots erupt one. This module decides which plumes
//! there are, where their trails run, and which of them erupt.

use crate::{
    field::{GeologyInputError, MaxWinsField},
    provinces::{ProvinceProfile, trace_province},
};
use procgen_core::{
    RandomStream, Vec3,
    random_streams::{HOTSPOT_POSITION, HOTSPOT_PROVINCE},
};
use procgen_sphere_mesh::{SphereMesh, default_hop_length, hops};
use procgen_tectonics::{CellCrust, CrustClass, PlateKinematics, PlatePartition, StageInputError};
use std::fmt;

const STATIONARY_SPEED_SQUARED: f32 = 1.0e-12;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HotspotFieldConfig {
    pub hotspot_count: usize,
    /// Longest track a plume can burn through the plate above it, as a model
    /// length on the unit sphere. The default of eight default hops is about
    /// 710 km at Earth radius.
    pub maximum_trail_length: f32,
    /// Fraction of the hotspots with a continental source that erupt a flood
    /// basalt province, drawn per hotspot from its own hashed stream.
    pub province_fraction: f32,
    /// Distance at which a province's rim would reach zero, as a model length
    /// on the unit sphere. That ring is already outside, so a province reaches
    /// one hop less. The default of five default hops is about 440 km at Earth
    /// radius, so a plateau spans roughly 900 km.
    pub province_radius: f32,
    /// Width of the outer province over which the plateau slopes back down
    /// toward nothing, as a model length. Everything further in stands at full
    /// weight, and a zero width is a hard edge.
    pub province_rim: f32,
    pub seed: u64,
}

impl HotspotFieldConfig {
    pub fn new(seed: u64) -> Self {
        Self {
            hotspot_count: 20,
            maximum_trail_length: 8.0 * default_hop_length(),
            province_fraction: 0.4,
            province_radius: 5.0 * default_hop_length(),
            province_rim: 2.0 * default_hop_length(),
            seed,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HotspotTrailCell {
    pub cell: usize,
    /// Unitless diagnostic intensity: one at the source and linearly decaying
    /// toward the configured trail bound.
    pub intensity: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hotspot {
    /// Seeded fixed mantle-plume position on the sphere surface.
    pub mantle_position: Vec3,
    pub source_cell: usize,
    pub plate: usize,
    /// Youngest-to-oldest cells, beginning at `source_cell`.
    pub trail: Vec<HotspotTrailCell>,
    /// Cells this hotspot's flood basalt province covers, zero for a hotspot
    /// that erupted none.
    pub province_cell_count: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HotspotDiagnostics {
    pub trail_cell_count: usize,
    pub affected_cell_count: usize,
    pub overlap_cell_count: usize,
    pub stationary_source_count: usize,
    pub shortest_trail_cells: usize,
    pub longest_trail_cells: usize,
    pub province_count: usize,
    /// Cells covered by at least one province, however many overlap there.
    pub province_cell_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HotspotField {
    pub hotspots: Vec<Hotspot>,
    /// Max-wins aggregate intensity, independent of elevation.
    pub cell_intensities: Vec<f32>,
    /// Winning hotspot for each affected cell. Equal intensities resolve to
    /// the lower hotspot index.
    pub cell_hotspots: Vec<Option<usize>>,
    /// Largest province weight covering each cell, one on a plateau top and
    /// sloping down across its rim. Every province cell is positive here,
    /// since the ring where the rim reaches zero lies outside the province.
    /// Overlapping provinces resolve by maximum, as intensities do.
    pub cell_plateau: Vec<f32>,
    pub diagnostics: HotspotDiagnostics,
}

impl HotspotField {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), GeologyInputError> {
        if self.cell_intensities.len() != mesh.cell_count()
            || self.cell_hotspots.len() != mesh.cell_count()
            || self.cell_plateau.len() != mesh.cell_count()
            || self
                .cell_hotspots
                .iter()
                .flatten()
                .any(|&hotspot| hotspot >= self.hotspots.len())
            || self.hotspots.iter().any(|hotspot| {
                hotspot.source_cell >= mesh.cell_count()
                    || hotspot
                        .trail
                        .iter()
                        .any(|trail| trail.cell >= mesh.cell_count())
            })
        {
            return Err(GeologyInputError::Hotspots);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotspotFieldError {
    Input(StageInputError),
    EmptyTrail,
    InvalidProvinceFraction,
    ProvinceRimExceedsRadius,
}

impl fmt::Display for HotspotFieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => error.fmt(formatter),
            Self::EmptyTrail => {
                formatter.write_str("maximum trail length must be finite and positive")
            }
            Self::InvalidProvinceFraction => {
                formatter.write_str("province fraction must be finite and between zero and one")
            }
            Self::ProvinceRimExceedsRadius => {
                formatter.write_str(
                    "province radius and rim must be finite, and the rim must be between zero and the radius",
                )
            }
        }
    }
}

impl std::error::Error for HotspotFieldError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            Self::EmptyTrail | Self::InvalidProvinceFraction | Self::ProvinceRimExceedsRadius => {
                None
            }
        }
    }
}

impl From<StageInputError> for HotspotFieldError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
    }
}

/// Generates fixed mantle hotspots, present-day trails opposite each source
/// plate's local motion, and flood basalt provinces around the hashed subset
/// of hotspots whose source sits on continental crust. Trail walking is
/// constrained to final plate ownership and neither field modifies tectonic
/// elevation.
pub fn generate_hotspot_field(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    cell_crust: CellCrust<'_>,
    kinematics: &PlateKinematics,
    config: HotspotFieldConfig,
) -> Result<HotspotField, HotspotFieldError> {
    validate_inputs(mesh, plates, cell_crust, kinematics, config)?;

    // The two conversions: a trail's bound and a province's shape are model
    // lengths, and both walk the mesh in cells.
    let trail_cells = hops(mesh.cell_count(), config.maximum_trail_length);
    let province_profile = ProvinceProfile::new(
        mesh.cell_count(),
        config.province_radius,
        config.province_rim,
    );
    let positions = RandomStream::new(config.seed, HOTSPOT_POSITION);
    let provinces = RandomStream::new(config.seed, HOTSPOT_PROVINCE);
    let mut hotspots = Vec::with_capacity(config.hotspot_count);
    let mut aggregate = MaxWinsField::new(mesh.cell_count());
    let mut cell_plateau = vec![0.0; mesh.cell_count()];
    let mut province_covered = vec![false; mesh.cell_count()];
    let mut stationary_source_count = 0;

    for hotspot_index in 0..config.hotspot_count {
        let mantle_position = positions.unit_vector(hotspot_index as u64) * mesh.radius;
        let source_cell = nearest_cell(mesh, mantle_position);
        let plate = plates.cell_plates[source_cell];
        let (trail, source_is_stationary) =
            trace_trail(mesh, plates, kinematics, plate, source_cell, trail_cells);
        stationary_source_count += usize::from(source_is_stationary);
        for point in &trail {
            aggregate.claim(point.cell, point.intensity, hotspot_index);
        }

        let erupts = cell_crust.class(source_cell) == CrustClass::Continental
            && provinces.unit_f32(hotspot_index as u64, 0) < config.province_fraction;
        let province = if erupts {
            trace_province(mesh, plates, cell_crust, source_cell, province_profile)
        } else {
            Vec::new()
        };
        for point in &province {
            cell_plateau[point.cell] = f32::max(cell_plateau[point.cell], point.weight);
            province_covered[point.cell] = true;
        }

        hotspots.push(Hotspot {
            mantle_position,
            source_cell,
            plate,
            trail,
            province_cell_count: province.len(),
        });
    }

    let trail_cell_count = hotspots.iter().map(|hotspot| hotspot.trail.len()).sum();
    let affected_cell_count = aggregate.affected_cell_count();
    let overlap_cell_count = aggregate.overlap_cell_count();
    let shortest_trail_cells = hotspots
        .iter()
        .map(|hotspot| hotspot.trail.len())
        .min()
        .unwrap_or(0);
    let longest_trail_cells = hotspots
        .iter()
        .map(|hotspot| hotspot.trail.len())
        .max()
        .unwrap_or(0);
    let province_count = hotspots
        .iter()
        .filter(|hotspot| hotspot.province_cell_count > 0)
        .count();
    let province_cell_count = province_covered.iter().filter(|&&covered| covered).count();

    let (cell_intensities, cell_hotspots) = aggregate.into_parts();
    Ok(HotspotField {
        hotspots,
        cell_intensities,
        cell_hotspots,
        cell_plateau,
        diagnostics: HotspotDiagnostics {
            trail_cell_count,
            affected_cell_count,
            overlap_cell_count,
            stationary_source_count,
            shortest_trail_cells,
            longest_trail_cells,
            province_count,
            province_cell_count,
        },
    })
}

fn validate_inputs(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    cell_crust: CellCrust<'_>,
    kinematics: &PlateKinematics,
    config: HotspotFieldConfig,
) -> Result<(), HotspotFieldError> {
    if !config.maximum_trail_length.is_finite() || config.maximum_trail_length <= 0.0 {
        return Err(HotspotFieldError::EmptyTrail);
    }
    if !config.province_fraction.is_finite() || !(0.0..=1.0).contains(&config.province_fraction) {
        return Err(HotspotFieldError::InvalidProvinceFraction);
    }
    if !config.province_radius.is_finite()
        || !config.province_rim.is_finite()
        || config.province_rim < 0.0
        || config.province_rim > config.province_radius
    {
        return Err(HotspotFieldError::ProvinceRimExceedsRadius);
    }
    plates.validate(mesh)?;
    cell_crust.validate(mesh)?;
    kinematics.validate(plates)?;
    Ok(())
}

fn nearest_cell(mesh: &SphereMesh, position: Vec3) -> usize {
    mesh.cell_centers
        .iter()
        .enumerate()
        .min_by(|(left_cell, left), (right_cell, right)| {
            left.distance_squared(position)
                .total_cmp(&right.distance_squared(position))
                .then_with(|| left_cell.cmp(right_cell))
        })
        .map(|(cell, _)| cell)
        .expect("sphere meshes contain cells")
}

fn trace_trail(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    kinematics: &PlateKinematics,
    plate: usize,
    source_cell: usize,
    trail_cells: usize,
) -> (Vec<HotspotTrailCell>, bool) {
    let mut trail = Vec::with_capacity(trail_cells);
    let mut current = source_cell;
    let mut source_is_stationary = false;

    for step in 0..trail_cells {
        trail.push(HotspotTrailCell {
            cell: current,
            intensity: 1.0 - step as f32 / trail_cells as f32,
        });

        let velocity = kinematics.velocity_at(plate, mesh.cell_centers[current]);
        let stationary = velocity.length_squared() <= STATIONARY_SPEED_SQUARED;
        source_is_stationary |= step == 0 && stationary;
        if stationary {
            break;
        }
        let trail_direction = -velocity;
        let next = mesh
            .cell_corners(current)
            .iter()
            .map(|corner| corner.neighbor)
            .filter(|&neighbor| {
                plates.cell_plates[neighbor] == plate
                    && !trail.iter().any(|point| point.cell == neighbor)
            })
            .filter_map(|neighbor| {
                let direction = mesh.cell_centers[neighbor] - mesh.cell_centers[current];
                let alignment = direction.dot(trail_direction);
                (alignment > 0.0).then_some((neighbor, alignment))
            })
            .max_by(|(left_cell, left), (right_cell, right)| {
                left.total_cmp(right)
                    .then_with(|| right_cell.cmp(left_cell))
            })
            .map(|(cell, _)| cell);
        let Some(next) = next else {
            break;
        };
        current = next;
    }
    (trail, source_is_stationary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        HOTSPOT_CELL_COUNT, PlateFixture, hotspot_fixture, hotspot_province_config,
        hotspot_reference_config,
    };
    use procgen_core::fingerprint;
    use procgen_sphere_mesh::{DEFAULT_CELL_COUNT, hop_length};

    fn generate(
        fixture: &PlateFixture,
        config: HotspotFieldConfig,
    ) -> Result<HotspotField, HotspotFieldError> {
        generate_hotspot_field(
            &fixture.mesh,
            &fixture.plates,
            fixture.crust(),
            &fixture.kinematics,
            config,
        )
    }

    /// The trail bound is a model length now, and this is the assertion that
    /// it still means the eight cells it was written as.
    #[test]
    fn the_default_trail_bound_is_eight_cells_of_the_default_mesh() {
        assert_eq!(
            hops(
                DEFAULT_CELL_COUNT,
                HotspotFieldConfig::new(17).maximum_trail_length
            ),
            8
        );
    }

    #[test]
    fn field_is_deterministic_and_seeded() {
        let fixture = hotspot_fixture(HOTSPOT_CELL_COUNT);
        let first = generate(&fixture, hotspot_province_config()).unwrap();
        assert_eq!(
            first,
            generate(&fixture, hotspot_province_config()).unwrap()
        );
        assert_ne!(
            first,
            generate(
                &fixture,
                HotspotFieldConfig {
                    seed: 18,
                    ..hotspot_province_config()
                }
            )
            .unwrap()
        );
        assert!(first.hotspots.iter().all(|hotspot| {
            (hotspot.mantle_position.length() - fixture.mesh.radius).abs() < 1.0e-6
        }));
    }

    /// Cells, plates, and counts only.
    ///
    /// The mantle position is the one float a hotspot carries, and it names
    /// `source_cell` and nothing else, so the cell is the integer fact it
    /// produces and the position itself never enters the pin. Trail intensity
    /// is `1 - step / maximum_trail_cells`, which the trail length already
    /// determines and
    /// `trails_are_bounded_decaying_motion_opposed_and_owner_constrained`
    /// asserts point by point.
    ///
    /// `nearest_cell` still reads a position that `RandomStream::unit_vector`
    /// built from a sine and a cosine. That leaves one libm call deciding an
    /// integer here, as `docs/plate-movement.md` records for plate motion; it
    /// is the first thing to replace if this pin ever splits between the two
    /// development machines.
    #[test]
    fn reference_field_has_stable_fingerprint() {
        let fixture = hotspot_fixture(HOTSPOT_CELL_COUNT);
        let field = generate(&fixture, hotspot_reference_config()).unwrap();
        let values = field.hotspots.iter().flat_map(|hotspot| {
            [
                hotspot.source_cell as u64,
                hotspot.plate as u64,
                hotspot.trail.len() as u64,
            ]
            .into_iter()
            .chain(hotspot.trail.iter().map(|point| point.cell as u64))
        });

        assert_eq!(fingerprint(values), 16_868_552_496_129_063_622);
    }

    /// A zero province fraction leaves every hotspot and its trail exactly
    /// where they were before provinces existed and adds nothing to the
    /// plateau field, which is why the fingerprint above reads a config that
    /// erupts none.
    fn no_province_field_matches_the_pre_slice_field(field: &HotspotField) {
        assert!(field.cell_plateau.iter().all(|&weight| weight == 0.0));
        assert!(
            field
                .hotspots
                .iter()
                .all(|hotspot| hotspot.province_cell_count == 0)
        );
        assert_eq!(field.diagnostics.province_count, 0);
        assert_eq!(field.diagnostics.province_cell_count, 0);
    }

    #[test]
    fn zero_province_fraction_reproduces_the_field_without_provinces() {
        let fixture = hotspot_fixture(HOTSPOT_CELL_COUNT);
        let field = generate(&fixture, hotspot_reference_config()).unwrap();
        no_province_field_matches_the_pre_slice_field(&field);

        // Every other part of the field is untouched by the province radius
        // and rim, which only a nonzero fraction can reach.
        let widened = generate(
            &fixture,
            HotspotFieldConfig {
                province_radius: hop_length(HOTSPOT_CELL_COUNT, 9.0),
                province_rim: hop_length(HOTSPOT_CELL_COUNT, 4.0),
                ..hotspot_reference_config()
            },
        )
        .unwrap();
        assert_eq!(field, widened);
    }

    #[test]
    fn trails_are_bounded_decaying_motion_opposed_and_owner_constrained() {
        let fixture = hotspot_fixture(HOTSPOT_CELL_COUNT);
        let config = hotspot_province_config();
        let trail_cells = hops(HOTSPOT_CELL_COUNT, config.maximum_trail_length);
        let field = generate(&fixture, config).unwrap();

        for hotspot in &field.hotspots {
            assert!(!hotspot.trail.is_empty());
            assert!(hotspot.trail.len() <= trail_cells);
            assert_eq!(hotspot.trail[0].cell, hotspot.source_cell);
            assert_eq!(hotspot.trail[0].intensity, 1.0);
            for (step, point) in hotspot.trail.iter().enumerate() {
                assert_eq!(fixture.plates.cell_plates[point.cell], hotspot.plate);
                assert_eq!(point.intensity, 1.0 - step as f32 / trail_cells as f32);
            }
            for pair in hotspot.trail.windows(2) {
                let current = pair[0].cell;
                let next = pair[1].cell;
                let velocity = fixture
                    .kinematics
                    .velocity_at(hotspot.plate, fixture.mesh.cell_centers[current]);
                let step = fixture.mesh.cell_centers[next] - fixture.mesh.cell_centers[current];
                assert!(step.dot(velocity) < 0.0);
                assert!(
                    fixture
                        .mesh
                        .cell_corners(current)
                        .iter()
                        .any(|corner| corner.neighbor == next)
                );
            }
        }
    }

    #[test]
    fn overlaps_use_max_intensity_then_lowest_hotspot_index() {
        let fixture = hotspot_fixture(32);
        let field = generate(
            &fixture,
            HotspotFieldConfig {
                hotspot_count: 64,
                maximum_trail_length: hop_length(32, 5.0),
                province_fraction: 0.0,
                ..HotspotFieldConfig::new(23)
            },
        )
        .unwrap();
        assert!(field.diagnostics.overlap_cell_count > 0);

        for cell in 0..fixture.mesh.cell_count() {
            let expected = field
                .hotspots
                .iter()
                .enumerate()
                .flat_map(|(hotspot, data)| {
                    data.trail
                        .iter()
                        .filter(move |point| point.cell == cell)
                        .map(move |point| (hotspot, point.intensity))
                })
                .max_by(|(left_hotspot, left), (right_hotspot, right)| {
                    left.total_cmp(right)
                        .then_with(|| right_hotspot.cmp(left_hotspot))
                });
            assert_eq!(field.cell_hotspots[cell], expected.map(|value| value.0));
            assert_eq!(
                field.cell_intensities[cell],
                expected.map_or(0.0, |value| value.1)
            );
        }
    }

    #[test]
    fn zero_motion_produces_source_only_trails() {
        let mut fixture = hotspot_fixture(128);
        fixture.kinematics = PlateKinematics {
            angular_velocities: vec![Vec3::ZERO; fixture.plates.plate_count],
            base_speeds: vec![0.0; fixture.plates.plate_count],
        };
        let config = HotspotFieldConfig {
            hotspot_count: 12,
            ..hotspot_reference_config()
        };
        let field = generate(&fixture, config).unwrap();

        assert!(
            field
                .hotspots
                .iter()
                .all(|hotspot| hotspot.trail.len() == 1)
        );
        assert_eq!(
            field.diagnostics.stationary_source_count,
            config.hotspot_count
        );
    }

    #[test]
    fn zero_hotspots_produces_an_empty_field() {
        let fixture = hotspot_fixture(32);
        let field = generate(
            &fixture,
            HotspotFieldConfig {
                hotspot_count: 0,
                ..hotspot_province_config()
            },
        )
        .unwrap();

        assert!(field.hotspots.is_empty());
        assert!(field.cell_intensities.iter().all(|&value| value == 0.0));
        assert!(field.cell_hotspots.iter().all(Option::is_none));
        no_province_field_matches_the_pre_slice_field(&field);
        assert_eq!(field.diagnostics, HotspotDiagnostics::default());
    }

    #[test]
    fn rejects_invalid_configuration_and_inputs() {
        let fixture = hotspot_fixture(32);
        assert_eq!(
            generate(
                &fixture,
                HotspotFieldConfig {
                    maximum_trail_length: 0.0,
                    ..hotspot_reference_config()
                }
            ),
            Err(HotspotFieldError::EmptyTrail)
        );
        assert_eq!(
            generate(
                &fixture,
                HotspotFieldConfig {
                    province_fraction: f32::NAN,
                    ..hotspot_reference_config()
                }
            ),
            Err(HotspotFieldError::InvalidProvinceFraction)
        );
        assert_eq!(
            generate(
                &fixture,
                HotspotFieldConfig {
                    province_fraction: 1.5,
                    ..hotspot_reference_config()
                }
            ),
            Err(HotspotFieldError::InvalidProvinceFraction)
        );
        assert_eq!(
            generate(
                &fixture,
                HotspotFieldConfig {
                    province_radius: hop_length(HOTSPOT_CELL_COUNT, 2.0),
                    province_rim: hop_length(HOTSPOT_CELL_COUNT, 3.0),
                    ..hotspot_reference_config()
                }
            ),
            Err(HotspotFieldError::ProvinceRimExceedsRadius)
        );

        let mut invalid_cells = fixture.plates.clone();
        invalid_cells.cell_plates.pop();
        assert_eq!(
            generate_hotspot_field(
                &fixture.mesh,
                &invalid_cells,
                fixture.crust(),
                &fixture.kinematics,
                hotspot_reference_config()
            ),
            Err(HotspotFieldError::Input(StageInputError::Cells))
        );

        let mut short_birth = fixture.cell_birth.clone();
        short_birth.pop();
        assert_eq!(
            generate_hotspot_field(
                &fixture.mesh,
                &fixture.plates,
                CellCrust {
                    cell_birth: &short_birth
                },
                &fixture.kinematics,
                hotspot_reference_config()
            ),
            Err(HotspotFieldError::Input(StageInputError::CrustBirth))
        );

        let invalid_kinematics = PlateKinematics {
            angular_velocities: Vec::new(),
            base_speeds: Vec::new(),
        };
        assert_eq!(
            generate_hotspot_field(
                &fixture.mesh,
                &fixture.plates,
                fixture.crust(),
                &invalid_kinematics,
                hotspot_reference_config()
            ),
            Err(HotspotFieldError::Input(StageInputError::Plates))
        );

        let mut invalid_ownership = fixture.plates.clone();
        invalid_ownership.cell_plates[0] = fixture.plates.plate_count;
        assert_eq!(
            generate_hotspot_field(
                &fixture.mesh,
                &invalid_ownership,
                fixture.crust(),
                &fixture.kinematics,
                hotspot_reference_config()
            ),
            Err(HotspotFieldError::Input(StageInputError::PlateOwnership))
        );
    }

    #[test]
    fn validation_reports_misaligned_aggregate_fields() {
        let fixture = hotspot_fixture(32);
        let mut field = generate(&fixture, hotspot_reference_config()).unwrap();
        field.cell_hotspots.pop();
        assert_eq!(
            field.validate(&fixture.mesh),
            Err(GeologyInputError::Hotspots)
        );

        let mut field = generate(&fixture, hotspot_reference_config()).unwrap();
        field.cell_plateau.pop();
        assert_eq!(
            field.validate(&fixture.mesh),
            Err(GeologyInputError::Hotspots)
        );
        assert_eq!(
            GeologyInputError::Hotspots.to_string(),
            "hotspot aggregate field is inconsistent with the mesh"
        );
    }
}
