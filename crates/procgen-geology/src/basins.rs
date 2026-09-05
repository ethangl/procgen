use procgen_sphere_mesh::SphereMesh;
use procgen_tectonics::{
    CoarseElevation, CrustClass, CrustClassification, PlatePartition, SEA_LEVEL, StageInputError,
};
use std::{collections::VecDeque, fmt};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SedimentaryBasinFieldConfig {
    /// Candidate continental land must lie strictly below this normalized elevation.
    pub maximum_elevation: f32,
    pub minimum_cell_count: usize,
    /// Maximum fraction of external component-neighbor incidences that may face ocean.
    pub maximum_ocean_perimeter_fraction: f32,
    /// Added to a retained component's minimum elevation to describe its basin floor.
    pub floor_offset: f32,
}

impl Default for SedimentaryBasinFieldConfig {
    fn default() -> Self {
        Self {
            maximum_elevation: 0.61,
            minimum_cell_count: 3,
            maximum_ocean_perimeter_fraction: 0.5,
            floor_offset: 0.02,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SedimentaryBasin {
    /// Compact zero-based ID, stable under deterministic mesh and input ordering.
    pub id: usize,
    /// Lowest-indexed cell in the connected component.
    pub root_cell: usize,
    pub cell_count: usize,
    pub ocean_perimeter_fraction: f32,
    pub minimum_elevation: f32,
    /// Metadata only; this stage does not flatten or otherwise mutate elevation.
    pub floor_elevation: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SedimentaryBasinDiagnostics {
    pub candidate_cell_count: usize,
    pub component_count: usize,
    pub basin_count: usize,
    pub basin_cell_count: usize,
    pub rejected_small_component_count: usize,
    pub rejected_ocean_exposed_component_count: usize,
    pub smallest_basin_cell_count: usize,
    pub largest_basin_cell_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SedimentaryBasinField {
    /// Retained basin ownership. IDs index `basins` directly.
    pub cell_basins: Vec<Option<usize>>,
    pub basins: Vec<SedimentaryBasin>,
    pub diagnostics: SedimentaryBasinDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SedimentaryBasinFieldError {
    Input(StageInputError),
    InvalidConfig,
}

impl fmt::Display for SedimentaryBasinFieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => error.fmt(formatter),
            Self::InvalidConfig => formatter.write_str(
                "basin maximum elevation must be finite and at or above sea level, minimum cell count must be positive, ocean perimeter fraction must be finite and between 0 and 1, and floor offset must be finite and nonnegative",
            ),
        }
    }
}

impl std::error::Error for SedimentaryBasinFieldError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            Self::InvalidConfig => None,
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
/// IDs follow ascending root-cell order and compactly index `basins`. Basin
/// floors are deterministic metadata derived from present-day coarse elevation;
/// the elevation field is never modified.
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
    let mut visited = vec![false; mesh.cell_count()];
    let mut cell_basins = vec![None; mesh.cell_count()];
    let mut basins = Vec::new();
    let mut component_count = 0;
    let mut rejected_small_component_count = 0;
    let mut rejected_ocean_exposed_component_count = 0;

    for root_cell in 0..mesh.cell_count() {
        if !candidates[root_cell] || visited[root_cell] {
            continue;
        }
        component_count += 1;
        let component = collect_component(mesh, &candidates, elevation, &mut visited, root_cell);
        let too_small = component.cells.len() < config.minimum_cell_count;
        let too_exposed =
            component.ocean_perimeter_fraction > config.maximum_ocean_perimeter_fraction;
        rejected_small_component_count += usize::from(too_small);
        rejected_ocean_exposed_component_count += usize::from(!too_small && too_exposed);
        if too_small || too_exposed {
            continue;
        }

        let id = basins.len();
        for &cell in &component.cells {
            cell_basins[cell] = Some(id);
        }
        basins.push(SedimentaryBasin {
            id,
            root_cell,
            cell_count: component.cells.len(),
            ocean_perimeter_fraction: component.ocean_perimeter_fraction,
            minimum_elevation: component.minimum_elevation,
            floor_elevation: (component.minimum_elevation + config.floor_offset).clamp(0.0, 1.0),
        });
    }

    let basin_cell_count = basins.iter().map(|basin| basin.cell_count).sum();
    let smallest_basin_cell_count = basins
        .iter()
        .map(|basin| basin.cell_count)
        .min()
        .unwrap_or(0);
    let largest_basin_cell_count = basins
        .iter()
        .map(|basin| basin.cell_count)
        .max()
        .unwrap_or(0);
    Ok(SedimentaryBasinField {
        cell_basins,
        diagnostics: SedimentaryBasinDiagnostics {
            candidate_cell_count,
            component_count,
            basin_count: basins.len(),
            basin_cell_count,
            rejected_small_component_count,
            rejected_ocean_exposed_component_count,
            smallest_basin_cell_count,
            largest_basin_cell_count,
        },
        basins,
    })
}

struct Component {
    cells: Vec<usize>,
    ocean_perimeter_fraction: f32,
    minimum_elevation: f32,
}

fn collect_component(
    mesh: &SphereMesh,
    candidates: &[bool],
    elevation: &CoarseElevation,
    visited: &mut [bool],
    root_cell: usize,
) -> Component {
    let mut cells = Vec::new();
    let mut queue = VecDeque::from([root_cell]);
    let mut perimeter_count = 0;
    let mut ocean_perimeter_count = 0;
    let mut minimum_elevation = f32::INFINITY;
    visited[root_cell] = true;

    while let Some(cell) = queue.pop_front() {
        cells.push(cell);
        minimum_elevation = minimum_elevation.min(elevation.cell_elevations[cell]);
        for corner in mesh.cell_corners(cell) {
            let neighbor = corner.neighbor;
            if candidates[neighbor] {
                if !visited[neighbor] {
                    visited[neighbor] = true;
                    queue.push_back(neighbor);
                }
            } else {
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
    Component {
        cells,
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
        || config.minimum_cell_count == 0
        || !config.maximum_ocean_perimeter_fraction.is_finite()
        || !(0.0..=1.0).contains(&config.maximum_ocean_perimeter_fraction)
        || !config.floor_offset.is_finite()
        || config.floor_offset < 0.0
    {
        return Err(SedimentaryBasinFieldError::InvalidConfig);
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
    use procgen_sphere_mesh::build_sphere_mesh;
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
        let mut cells = Vec::new();
        let mut seen = vec![false; mesh.cell_count()];
        let mut queue = VecDeque::from([0]);
        seen[0] = true;
        while let Some(cell) = queue.pop_front() {
            cells.push(cell);
            if cells.len() == count {
                break;
            }
            for corner in mesh.cell_corners(cell) {
                if !seen[corner.neighbor] {
                    seen[corner.neighbor] = true;
                    queue.push_back(corner.neighbor);
                }
            }
        }
        cells
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
        assert_eq!(first.basins[0].id, 0);
        assert_eq!(first.basins[0].root_cell, 0);
        assert!((first.basins[0].floor_elevation - 0.54).abs() < f32::EPSILON);
        assert_eq!(first.diagnostics.basin_cell_count, 12);
        let mut reached = vec![false; mesh.cell_count()];
        let mut queue = VecDeque::from([first.basins[0].root_cell]);
        reached[first.basins[0].root_cell] = true;
        let mut reached_count = 0;
        while let Some(cell) = queue.pop_front() {
            reached_count += 1;
            for corner in mesh.cell_corners(cell) {
                if first.cell_basins[corner.neighbor] == Some(0) && !reached[corner.neighbor] {
                    reached[corner.neighbor] = true;
                    queue.push_back(corner.neighbor);
                }
            }
        }
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
                .map(|basin| basin.id)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(
            field
                .basins
                .iter()
                .map(|basin| basin.root_cell)
                .collect::<Vec<_>>(),
            vec![first, second]
        );
        assert_eq!(field.diagnostics.smallest_basin_cell_count, 1);
        assert_eq!(field.diagnostics.largest_basin_cell_count, 1);
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

        assert_eq!(
            derive_sedimentary_basin_field(
                &mesh,
                &plates,
                &crust,
                &elevation,
                SedimentaryBasinFieldConfig {
                    minimum_cell_count: 0,
                    ..Default::default()
                },
            ),
            Err(SedimentaryBasinFieldError::InvalidConfig)
        );
    }
}
