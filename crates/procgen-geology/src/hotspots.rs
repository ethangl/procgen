use crate::field::MaxWinsField;
use procgen_core::{RandomStream, Vec3, random_streams::HOTSPOT_POSITION};
use procgen_sphere_mesh::SphereMesh;
use procgen_tectonics::{PlateKinematics, PlatePartition, StageInputError};
use std::fmt;

const STATIONARY_SPEED_SQUARED: f32 = 1.0e-12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotspotFieldConfig {
    pub hotspot_count: usize,
    /// Maximum cells in each trail, including its source cell.
    pub maximum_trail_cells: usize,
    pub seed: u64,
}

impl HotspotFieldConfig {
    pub const fn new(seed: u64) -> Self {
        Self {
            hotspot_count: 20,
            maximum_trail_cells: 8,
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
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HotspotDiagnostics {
    pub trail_cell_count: usize,
    pub affected_cell_count: usize,
    pub overlap_cell_count: usize,
    pub stationary_source_count: usize,
    pub shortest_trail_cells: usize,
    pub longest_trail_cells: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HotspotField {
    pub hotspots: Vec<Hotspot>,
    /// Max-wins aggregate intensity, independent of elevation.
    pub cell_intensities: Vec<f32>,
    /// Winning hotspot for each affected cell. Equal intensities resolve to
    /// the lower hotspot index.
    pub cell_hotspots: Vec<Option<usize>>,
    pub diagnostics: HotspotDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotspotFieldError {
    Input(StageInputError),
    EmptyTrail,
}

impl fmt::Display for HotspotFieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => error.fmt(formatter),
            Self::EmptyTrail => formatter.write_str("maximum trail cells must be at least one"),
        }
    }
}

impl std::error::Error for HotspotFieldError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            Self::EmptyTrail => None,
        }
    }
}

impl From<StageInputError> for HotspotFieldError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
    }
}

/// Generates fixed mantle hotspots and present-day trails opposite each
/// source plate's local motion. Trail walking is constrained to final plate
/// ownership and never modifies tectonic elevation.
pub fn generate_hotspot_field(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    kinematics: &PlateKinematics,
    config: HotspotFieldConfig,
) -> Result<HotspotField, HotspotFieldError> {
    validate_inputs(mesh, plates, kinematics, config)?;

    let positions = RandomStream::new(config.seed, HOTSPOT_POSITION);
    let mut hotspots = Vec::with_capacity(config.hotspot_count);
    let mut aggregate = MaxWinsField::new(mesh.cell_count());
    let mut stationary_source_count = 0;

    for hotspot_index in 0..config.hotspot_count {
        let mantle_position = positions.unit_vector(hotspot_index as u64) * mesh.radius;
        let source_cell = nearest_cell(mesh, mantle_position);
        let plate = plates.cell_plates[source_cell];
        let (trail, source_is_stationary) =
            trace_trail(mesh, plates, kinematics, plate, source_cell, config);
        stationary_source_count += usize::from(source_is_stationary);
        for point in &trail {
            aggregate.claim(point.cell, point.intensity, hotspot_index);
        }
        hotspots.push(Hotspot {
            mantle_position,
            source_cell,
            plate,
            trail,
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

    let (cell_intensities, cell_hotspots) = aggregate.into_parts();
    Ok(HotspotField {
        hotspots,
        cell_intensities,
        cell_hotspots,
        diagnostics: HotspotDiagnostics {
            trail_cell_count,
            affected_cell_count,
            overlap_cell_count,
            stationary_source_count,
            shortest_trail_cells,
            longest_trail_cells,
        },
    })
}

fn validate_inputs(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    kinematics: &PlateKinematics,
    config: HotspotFieldConfig,
) -> Result<(), HotspotFieldError> {
    if config.maximum_trail_cells == 0 {
        return Err(HotspotFieldError::EmptyTrail);
    }
    plates.validate(mesh)?;
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
    config: HotspotFieldConfig,
) -> (Vec<HotspotTrailCell>, bool) {
    let mut trail = Vec::with_capacity(config.maximum_trail_cells);
    let mut current = source_cell;
    let mut source_is_stationary = false;

    for step in 0..config.maximum_trail_cells {
        trail.push(HotspotTrailCell {
            cell: current,
            intensity: 1.0 - step as f32 / config.maximum_trail_cells as f32,
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
    use procgen_core::fingerprint;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::build_sphere_mesh;
    use procgen_tectonics::{
        PlateKinematicsConfig, PlatePartitionConfig, generate_plate_kinematics, partition_plates,
    };

    fn fixture(cell_count: usize) -> (SphereMesh, PlatePartition, PlateKinematics) {
        let mesh = build_sphere_mesh(
            fibonacci_sphere(FibonacciConfig {
                count: cell_count,
                jitter: 0.5,
                seed: 7,
            })
            .unwrap(),
            1.0,
        )
        .unwrap();
        let plates = partition_plates(
            &mesh,
            PlatePartitionConfig {
                major_plate_count: 4,
                minor_plate_count: 4,
                major_head_start_rounds: 1,
                seed: 11,
            },
        )
        .unwrap();
        let kinematics =
            generate_plate_kinematics(plates.plate_count, PlateKinematicsConfig::new(13)).unwrap();
        (mesh, plates, kinematics)
    }

    fn reference_config() -> HotspotFieldConfig {
        HotspotFieldConfig {
            hotspot_count: 24,
            maximum_trail_cells: 7,
            seed: 17,
        }
    }

    #[test]
    fn field_is_deterministic_and_seeded() {
        let (mesh, plates, kinematics) = fixture(512);
        let first =
            generate_hotspot_field(&mesh, &plates, &kinematics, reference_config()).unwrap();
        assert_eq!(
            first,
            generate_hotspot_field(&mesh, &plates, &kinematics, reference_config()).unwrap()
        );
        assert_ne!(
            first,
            generate_hotspot_field(
                &mesh,
                &plates,
                &kinematics,
                HotspotFieldConfig {
                    seed: 18,
                    ..reference_config()
                },
            )
            .unwrap()
        );
        assert!(
            first
                .hotspots
                .iter()
                .all(|hotspot| { (hotspot.mantle_position.length() - mesh.radius).abs() < 1.0e-6 })
        );
    }

    #[test]
    fn reference_field_has_stable_fingerprint() {
        let (mesh, plates, kinematics) = fixture(512);
        let field =
            generate_hotspot_field(&mesh, &plates, &kinematics, reference_config()).unwrap();
        let values = field.hotspots.iter().flat_map(|hotspot| {
            [
                u64::from(hotspot.mantle_position.x.to_bits()),
                u64::from(hotspot.mantle_position.y.to_bits()),
                u64::from(hotspot.mantle_position.z.to_bits()),
                hotspot.source_cell as u64,
                hotspot.plate as u64,
                hotspot.trail.len() as u64,
            ]
            .into_iter()
            .chain(
                hotspot
                    .trail
                    .iter()
                    .flat_map(|point| [point.cell as u64, u64::from(point.intensity.to_bits())]),
            )
        });

        assert_eq!(fingerprint(values), 12_311_312_604_747_609_208);
    }

    #[test]
    fn trails_are_bounded_decaying_motion_opposed_and_owner_constrained() {
        let (mesh, plates, kinematics) = fixture(512);
        let config = reference_config();
        let field = generate_hotspot_field(&mesh, &plates, &kinematics, config).unwrap();

        for hotspot in &field.hotspots {
            assert!(!hotspot.trail.is_empty());
            assert!(hotspot.trail.len() <= config.maximum_trail_cells);
            assert_eq!(hotspot.trail[0].cell, hotspot.source_cell);
            assert_eq!(hotspot.trail[0].intensity, 1.0);
            for (step, point) in hotspot.trail.iter().enumerate() {
                assert_eq!(plates.cell_plates[point.cell], hotspot.plate);
                assert_eq!(
                    point.intensity,
                    1.0 - step as f32 / config.maximum_trail_cells as f32
                );
            }
            for pair in hotspot.trail.windows(2) {
                let current = pair[0].cell;
                let next = pair[1].cell;
                let velocity = kinematics.velocity_at(hotspot.plate, mesh.cell_centers[current]);
                let step = mesh.cell_centers[next] - mesh.cell_centers[current];
                assert!(step.dot(velocity) < 0.0);
                assert!(
                    mesh.cell_corners(current)
                        .iter()
                        .any(|corner| corner.neighbor == next)
                );
            }
        }
    }

    #[test]
    fn overlaps_use_max_intensity_then_lowest_hotspot_index() {
        let (mesh, plates, kinematics) = fixture(32);
        let field = generate_hotspot_field(
            &mesh,
            &plates,
            &kinematics,
            HotspotFieldConfig {
                hotspot_count: 64,
                maximum_trail_cells: 5,
                seed: 23,
            },
        )
        .unwrap();
        assert!(field.diagnostics.overlap_cell_count > 0);

        for cell in 0..mesh.cell_count() {
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
        let (mesh, plates, _) = fixture(128);
        let kinematics = PlateKinematics {
            angular_velocities: vec![Vec3::ZERO; plates.plate_count],
        };
        let config = HotspotFieldConfig {
            hotspot_count: 12,
            ..reference_config()
        };
        let field = generate_hotspot_field(&mesh, &plates, &kinematics, config).unwrap();

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
        let (mesh, plates, kinematics) = fixture(32);
        let field = generate_hotspot_field(
            &mesh,
            &plates,
            &kinematics,
            HotspotFieldConfig {
                hotspot_count: 0,
                ..reference_config()
            },
        )
        .unwrap();

        assert!(field.hotspots.is_empty());
        assert!(field.cell_intensities.iter().all(|&value| value == 0.0));
        assert!(field.cell_hotspots.iter().all(Option::is_none));
        assert_eq!(field.diagnostics, HotspotDiagnostics::default());
    }

    #[test]
    fn rejects_invalid_configuration_and_inputs() {
        let (mesh, plates, kinematics) = fixture(32);
        assert_eq!(
            generate_hotspot_field(
                &mesh,
                &plates,
                &kinematics,
                HotspotFieldConfig {
                    maximum_trail_cells: 0,
                    ..reference_config()
                },
            ),
            Err(HotspotFieldError::EmptyTrail)
        );

        let mut invalid_cells = plates.clone();
        invalid_cells.cell_plates.pop();
        assert_eq!(
            generate_hotspot_field(&mesh, &invalid_cells, &kinematics, reference_config()),
            Err(HotspotFieldError::Input(StageInputError::Cells))
        );

        let invalid_kinematics = PlateKinematics {
            angular_velocities: Vec::new(),
        };
        assert_eq!(
            generate_hotspot_field(&mesh, &plates, &invalid_kinematics, reference_config()),
            Err(HotspotFieldError::Input(StageInputError::Plates))
        );

        let mut invalid_ownership = plates.clone();
        invalid_ownership.cell_plates[0] = plates.plate_count;
        assert_eq!(
            generate_hotspot_field(&mesh, &invalid_ownership, &kinematics, reference_config()),
            Err(HotspotFieldError::Input(StageInputError::PlateOwnership))
        );
    }
}
