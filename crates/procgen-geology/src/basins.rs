use crate::field::GeologyInputError;
use procgen_sphere_mesh::{SphereMesh, connected_components};
use procgen_tectonics::{
    CoarseElevation, CrustClass, CrustClassification, PlatePartition, SEA_LEVEL, StageInputError,
};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SedimentaryBasinFieldConfig {
    /// Candidate continental land must lie strictly below this normalized elevation.
    pub maximum_elevation: f32,
    pub minimum_cell_count: usize,
    /// Maximum fraction of external component-neighbor incidences that may face ocean.
    pub maximum_ocean_perimeter_fraction: f32,
}

impl Default for SedimentaryBasinFieldConfig {
    fn default() -> Self {
        Self {
            maximum_elevation: 0.61,
            minimum_cell_count: 3,
            maximum_ocean_perimeter_fraction: 0.5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SedimentaryBasin {
    /// Lowest-indexed cell in the connected component.
    pub root_cell: usize,
    pub cell_count: usize,
    pub ocean_perimeter_fraction: f32,
    pub minimum_elevation: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SedimentaryBasinDiagnostics {
    pub candidate_cell_count: usize,
    pub component_count: usize,
    pub basin_count: usize,
    pub basin_cell_count: usize,
    pub rejected_small_component_count: usize,
    pub rejected_ocean_exposed_component_count: usize,
    pub basin_cell_count_range: Option<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SedimentaryBasinField {
    /// Retained basin ownership. IDs index `basins` directly.
    pub cell_basins: Vec<Option<usize>>,
    pub basins: Vec<SedimentaryBasin>,
    pub diagnostics: SedimentaryBasinDiagnostics,
}

impl SedimentaryBasinField {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), GeologyInputError> {
        if self.cell_basins.len() != mesh.cell_count() {
            return Err(GeologyInputError::Basins);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SedimentaryBasinFieldError {
    Input(StageInputError),
    InvalidMaximumElevation,
    EmptyMinimumBasin,
    InvalidOceanPerimeterFraction,
}

impl fmt::Display for SedimentaryBasinFieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => error.fmt(formatter),
            Self::InvalidMaximumElevation => formatter
                .write_str("basin maximum elevation must be finite and between sea level and 1"),
            Self::EmptyMinimumBasin => {
                formatter.write_str("minimum basin cell count must be at least one")
            }
            Self::InvalidOceanPerimeterFraction => formatter
                .write_str("maximum ocean perimeter fraction must be finite and between 0 and 1"),
        }
    }
}

impl std::error::Error for SedimentaryBasinFieldError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StageInputError> for SedimentaryBasinFieldError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
    }
}

/// Identifies connected, low-lying continental-land components and retains
/// sufficiently large, sufficiently enclosed components as sedimentary basins.
///
/// IDs follow ascending root-cell order and compactly index `basins`. The
/// elevation field is read without being modified.
pub fn derive_sedimentary_basin_field(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: &CrustClassification,
    elevation: &CoarseElevation,
    config: SedimentaryBasinFieldConfig,
) -> Result<SedimentaryBasinField, SedimentaryBasinFieldError> {
    validate_inputs(mesh, plates, crust, elevation, config)?;

    let candidates: Vec<_> = (0..mesh.cell_count())
        .map(|cell| {
            crust.cell_class(plates, cell) == CrustClass::Continental
                && elevation.is_land(cell)
                && elevation.cell_elevations[cell] < config.maximum_elevation
        })
        .collect();
    let candidate_cell_count = candidates.iter().filter(|&&candidate| candidate).count();
    let components = connected_components(mesh, |cell| candidates[cell], |_, _| true);
    let component_count = components.len();
    let mut cell_basins = vec![None; mesh.cell_count()];
    let mut basins = Vec::new();
    let mut rejected_small_component_count = 0;
    let mut rejected_ocean_exposed_component_count = 0;

    for component in components {
        let basin = summarize_component(mesh, &candidates, elevation, &component);
        match reject_reason(&basin, config) {
            Some(Rejection::TooSmall) => rejected_small_component_count += 1,
            Some(Rejection::OceanExposed) => rejected_ocean_exposed_component_count += 1,
            None => {
                let id = basins.len();
                for cell in component {
                    cell_basins[cell] = Some(id);
                }
                basins.push(basin);
            }
        }
    }

    let basin_cell_count = basins.iter().map(|basin| basin.cell_count).sum();
    let minimum_basin_cell_count = basins.iter().map(|basin| basin.cell_count).min();
    let maximum_basin_cell_count = basins.iter().map(|basin| basin.cell_count).max();
    Ok(SedimentaryBasinField {
        cell_basins,
        diagnostics: SedimentaryBasinDiagnostics {
            candidate_cell_count,
            component_count,
            basin_count: basins.len(),
            basin_cell_count,
            rejected_small_component_count,
            rejected_ocean_exposed_component_count,
            basin_cell_count_range: minimum_basin_cell_count.zip(maximum_basin_cell_count),
        },
        basins,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rejection {
    TooSmall,
    OceanExposed,
}

fn reject_reason(
    basin: &SedimentaryBasin,
    config: SedimentaryBasinFieldConfig,
) -> Option<Rejection> {
    if basin.cell_count < config.minimum_cell_count {
        Some(Rejection::TooSmall)
    } else if basin.ocean_perimeter_fraction > config.maximum_ocean_perimeter_fraction {
        Some(Rejection::OceanExposed)
    } else {
        None
    }
}

fn summarize_component(
    mesh: &SphereMesh,
    candidates: &[bool],
    elevation: &CoarseElevation,
    cells: &[usize],
) -> SedimentaryBasin {
    let mut perimeter_count = 0;
    let mut ocean_perimeter_count = 0;
    let mut minimum_elevation = f32::INFINITY;
    for &cell in cells {
        minimum_elevation = minimum_elevation.min(elevation.cell_elevations[cell]);
        for corner in mesh.cell_corners(cell) {
            let neighbor = corner.neighbor;
            if !candidates[neighbor] {
                perimeter_count += 1;
                ocean_perimeter_count += usize::from(!elevation.is_land(neighbor));
            }
        }
    }

    let ocean_perimeter_fraction = if perimeter_count == 0 {
        0.0
    } else {
        ocean_perimeter_count as f32 / perimeter_count as f32
    };
    SedimentaryBasin {
        root_cell: cells[0],
        cell_count: cells.len(),
        ocean_perimeter_fraction,
        minimum_elevation,
    }
}

fn validate_inputs(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: &CrustClassification,
    elevation: &CoarseElevation,
    config: SedimentaryBasinFieldConfig,
) -> Result<(), SedimentaryBasinFieldError> {
    if !config.maximum_elevation.is_finite()
        || config.maximum_elevation < SEA_LEVEL
        || config.maximum_elevation > 1.0
    {
        return Err(SedimentaryBasinFieldError::InvalidMaximumElevation);
    }
    if config.minimum_cell_count == 0 {
        return Err(SedimentaryBasinFieldError::EmptyMinimumBasin);
    }
    if !config.maximum_ocean_perimeter_fraction.is_finite()
        || !(0.0..=1.0).contains(&config.maximum_ocean_perimeter_fraction)
    {
        return Err(SedimentaryBasinFieldError::InvalidOceanPerimeterFraction);
    }
    plates.validate(mesh)?;
    crust.validate(plates)?;
    elevation.validate(mesh)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_core::fingerprint;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::{build_sphere_mesh, multi_source_distances};
    use procgen_tectonics::{PlatePartitionConfig, partition_plates};

    fn fixture(cell_count: usize) -> (SphereMesh, PlatePartition, CrustClassification) {
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
        let plates = partition_plates(&mesh, PlatePartitionConfig::new(4, 4)).unwrap();
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Continental; plates.plate_count],
        };
        (mesh, plates, crust)
    }

    fn elevation(values: Vec<f32>) -> CoarseElevation {
        CoarseElevation {
            cell_elevations: values,
            diagnostics: Default::default(),
        }
    }

    fn connected_cells(mesh: &SphereMesh, count: usize) -> Vec<usize> {
        connected_components(mesh, |_| true, |_, _| true)
            .into_iter()
            .next()
            .unwrap()
            .into_iter()
            .take(count)
            .collect()
    }

    #[test]
    fn field_is_deterministic_connected_compact_and_preserves_elevation() {
        let (mesh, plates, crust) = fixture(256);
        let mut values = vec![0.7; mesh.cell_count()];
        for (index, cell) in connected_cells(&mesh, 12).into_iter().enumerate() {
            values[cell] = 0.52 + index as f32 * 0.001;
        }
        let elevation = elevation(values);
        let original_elevation = elevation.clone();
        let config = SedimentaryBasinFieldConfig::default();
        let first =
            derive_sedimentary_basin_field(&mesh, &plates, &crust, &elevation, config).unwrap();

        assert_eq!(
            first,
            derive_sedimentary_basin_field(&mesh, &plates, &crust, &elevation, config).unwrap()
        );
        assert_eq!(elevation, original_elevation);
        assert_eq!(first.basins.len(), 1);
        assert_eq!(first.basins[0].root_cell, 0);
        assert_eq!(first.diagnostics.basin_cell_count, 12);
        let reached_count =
            multi_source_distances(&mesh, &[first.basins[0].root_cell], |_, neighbor| {
                first.cell_basins[neighbor] == Some(0)
            })
            .iter()
            .flatten()
            .count();
        assert_eq!(reached_count, first.basins[0].cell_count);
        let ids = first
            .cell_basins
            .iter()
            .map(|id| id.map_or(0, |id| id as u64 + 1));
        assert_eq!(fingerprint(ids), 11_938_203_854_786_127_761);
    }

    #[test]
    fn size_and_ocean_perimeter_filters_are_independent() {
        let (mesh, plates, crust) = fixture(64);
        let component = connected_cells(&mesh, 4);
        let mut enclosed = vec![0.7; mesh.cell_count()];
        for &cell in &component {
            enclosed[cell] = 0.55;
        }
        let config = SedimentaryBasinFieldConfig {
            minimum_cell_count: 5,
            ..Default::default()
        };
        let small =
            derive_sedimentary_basin_field(&mesh, &plates, &crust, &elevation(enclosed), config)
                .unwrap();
        assert!(small.basins.is_empty());
        assert_eq!(small.diagnostics.rejected_small_component_count, 1);
        assert_eq!(small.diagnostics.rejected_ocean_exposed_component_count, 0);

        let mut exposed = vec![0.2; mesh.cell_count()];
        for &cell in &component {
            exposed[cell] = 0.55;
        }
        let exposed = derive_sedimentary_basin_field(
            &mesh,
            &plates,
            &crust,
            &elevation(exposed),
            SedimentaryBasinFieldConfig {
                minimum_cell_count: 1,
                maximum_ocean_perimeter_fraction: 0.5,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(exposed.basins.is_empty());
        assert_eq!(exposed.diagnostics.rejected_small_component_count, 0);
        assert_eq!(
            exposed.diagnostics.rejected_ocean_exposed_component_count,
            1
        );
    }

    #[test]
    fn disconnected_components_receive_compact_ids_in_root_order() {
        let (mesh, plates, crust) = fixture(64);
        let first = 0;
        let second = (1..mesh.cell_count())
            .find(|&cell| {
                !mesh
                    .cell_corners(first)
                    .iter()
                    .any(|corner| corner.neighbor == cell)
            })
            .unwrap();
        let mut values = vec![0.7; mesh.cell_count()];
        values[first] = 0.53;
        values[second] = 0.54;

        let field = derive_sedimentary_basin_field(
            &mesh,
            &plates,
            &crust,
            &elevation(values),
            SedimentaryBasinFieldConfig {
                minimum_cell_count: 1,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(field.cell_basins[first], Some(0));
        assert_eq!(field.cell_basins[second], Some(1));
        assert_eq!(
            field
                .basins
                .iter()
                .map(|basin| basin.root_cell)
                .collect::<Vec<_>>(),
            vec![first, second]
        );
        assert_eq!(field.diagnostics.basin_cell_count_range, Some((1, 1)));
    }

    #[test]
    fn eligibility_respects_land_crust_and_strict_elevation_edges() {
        let (mesh, plates, mut crust) = fixture(32);
        let cell = 0;
        let plate = plates.cell_plates[cell];
        let config = SedimentaryBasinFieldConfig {
            minimum_cell_count: 1,
            maximum_ocean_perimeter_fraction: 1.0,
            ..Default::default()
        };
        for value in [SEA_LEVEL, config.maximum_elevation] {
            let mut values = vec![0.7; mesh.cell_count()];
            values[cell] = value;
            let field =
                derive_sedimentary_basin_field(&mesh, &plates, &crust, &elevation(values), config)
                    .unwrap();
            assert!(field.basins.is_empty());
        }

        let mut values = vec![0.7; mesh.cell_count()];
        values[cell] = SEA_LEVEL + 0.01;
        crust.plate_classes[plate] = CrustClass::Oceanic;
        let field =
            derive_sedimentary_basin_field(&mesh, &plates, &crust, &elevation(values), config)
                .unwrap();
        assert!(field.basins.is_empty());
        assert_eq!(field.diagnostics.candidate_cell_count, 0);
    }

    #[test]
    fn empty_fields_and_invalid_configuration_are_explicit() {
        let (mesh, plates, crust) = fixture(32);
        let elevation = elevation(vec![0.7; mesh.cell_count()]);
        let empty = derive_sedimentary_basin_field(
            &mesh,
            &plates,
            &crust,
            &elevation,
            SedimentaryBasinFieldConfig::default(),
        )
        .unwrap();
        assert!(empty.basins.is_empty());
        assert_eq!(empty.cell_basins, vec![None; mesh.cell_count()]);
        assert_eq!(empty.diagnostics, SedimentaryBasinDiagnostics::default());

        let invalid_cases = [
            (
                SedimentaryBasinFieldConfig {
                    maximum_elevation: 1.01,
                    ..Default::default()
                },
                SedimentaryBasinFieldError::InvalidMaximumElevation,
            ),
            (
                SedimentaryBasinFieldConfig {
                    minimum_cell_count: 0,
                    ..Default::default()
                },
                SedimentaryBasinFieldError::EmptyMinimumBasin,
            ),
            (
                SedimentaryBasinFieldConfig {
                    maximum_ocean_perimeter_fraction: f32::NAN,
                    ..Default::default()
                },
                SedimentaryBasinFieldError::InvalidOceanPerimeterFraction,
            ),
        ];
        for (config, expected) in invalid_cases {
            assert_eq!(
                derive_sedimentary_basin_field(&mesh, &plates, &crust, &elevation, config),
                Err(expected)
            );
        }
    }

    #[test]
    fn rejection_policy_prioritizes_size_before_ocean_exposure() {
        let basin = SedimentaryBasin {
            root_cell: 0,
            cell_count: 1,
            ocean_perimeter_fraction: 1.0,
            minimum_elevation: 0.55,
        };
        let config = SedimentaryBasinFieldConfig {
            minimum_cell_count: 2,
            maximum_ocean_perimeter_fraction: 0.5,
            ..Default::default()
        };
        assert_eq!(reject_reason(&basin, config), Some(Rejection::TooSmall));
        assert_eq!(
            reject_reason(
                &basin,
                SedimentaryBasinFieldConfig {
                    minimum_cell_count: 1,
                    ..config
                }
            ),
            Some(Rejection::OceanExposed)
        );
        assert_eq!(
            reject_reason(
                &basin,
                SedimentaryBasinFieldConfig {
                    minimum_cell_count: 1,
                    maximum_ocean_perimeter_fraction: 1.0,
                    ..config
                }
            ),
            None
        );
    }
}
