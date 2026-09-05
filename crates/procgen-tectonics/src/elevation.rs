use crate::{
    BoundaryClass, BoundaryClassification, CrustClass, CrustClassification, PlatePartition,
};
use procgen_sphere_mesh::SphereMesh;
use std::{collections::VecDeque, fmt};

const BOUNDARY_SPEED_SCALE: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoarseElevationConfig {
    pub oceanic_base: f32,
    pub continental_base: f32,
    pub convergent_lift: f32,
    pub divergent_drop: f32,
    pub transform_lift: f32,
    pub propagation_depth: usize,
    pub continental_convergent_lift: f32,
    pub oceanic_trench_drop: f32,
    pub continental_convergent_depth: usize,
    pub oceanic_trench_depth: usize,
    pub smoothing_passes: usize,
    pub smoothing_weight: f32,
}

impl Default for CoarseElevationConfig {
    fn default() -> Self {
        Self {
            oceanic_base: 0.15,
            continental_base: 0.65,
            convergent_lift: 0.4,
            divergent_drop: -0.4,
            transform_lift: 0.4,
            propagation_depth: 3,
            continental_convergent_lift: 0.5,
            oceanic_trench_drop: -0.2,
            continental_convergent_depth: 5,
            oceanic_trench_depth: 1,
            smoothing_passes: 2,
            smoothing_weight: 0.2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CoarseElevationDiagnostics {
    pub minimum: f32,
    pub maximum: f32,
    pub mean: f32,
    pub boundary_source_cell_count: usize,
    pub boundary_affected_cell_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CoarseElevation {
    pub cell_elevations: Vec<f32>,
    pub diagnostics: CoarseElevationDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoarseElevationError {
    InvalidConfig,
    CellCountMismatch,
    PlateCountMismatch,
    BoundaryCountMismatch,
}

impl fmt::Display for CoarseElevationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str(
                "elevation values must be finite, bases and smoothing weight must be between 0 and 1",
            ),
            Self::CellCountMismatch => {
                formatter.write_str("plate assignments must match the mesh cell count")
            }
            Self::PlateCountMismatch => {
                formatter.write_str("plate classes must match the partition plate count")
            }
            Self::BoundaryCountMismatch => {
                formatter.write_str("boundary arrays must match the mesh edge count")
            }
        }
    }
}

impl std::error::Error for CoarseElevationError {}

#[derive(Clone, Copy)]
struct BoundarySource {
    effect: f32,
    maximum_depth: usize,
    plate: usize,
}

/// Derives one normalized elevation value per cell from final tectonic state.
///
/// Base elevation follows each cell's current plate owner. Final boundary
/// effects are selected with a deterministic maximum-magnitude rule, propagate
/// only within that owner for a bounded number of mesh hops, and are applied
/// once before simultaneous smoothing. This stage neither reads nor produces
/// ownership history or inter-step elevation state.
pub fn derive_coarse_elevation(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    boundaries: &BoundaryClassification,
    config: CoarseElevationConfig,
) -> Result<CoarseElevation, CoarseElevationError> {
    validate_inputs(mesh, partition, crust, boundaries, config)?;

    let mut elevation: Vec<_> = partition
        .cell_plates
        .iter()
        .map(|&plate| match crust.plate_classes[plate] {
            CrustClass::Oceanic => config.oceanic_base,
            CrustClass::Continental => config.continental_base,
        })
        .collect();
    let mut sources = vec![None; mesh.cell_count()];

    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        let class = boundaries.edge_classes[edge_index];
        if class == BoundaryClass::Interior {
            continue;
        }

        let plates = edge.cells.map(|cell| partition.cell_plates[cell]);
        let classes = plates.map(|plate| crust.plate_classes[plate]);
        let scale = (boundaries.convergence(edge_index).abs() / BOUNDARY_SPEED_SCALE).min(1.0);

        if class == BoundaryClass::Convergent && classes[0] != classes[1] {
            for side in 0..2 {
                let (effect, maximum_depth) = match classes[side] {
                    CrustClass::Oceanic => (
                        config.oceanic_trench_drop * scale,
                        config.oceanic_trench_depth,
                    ),
                    CrustClass::Continental => (
                        config.continental_convergent_lift * scale,
                        config.continental_convergent_depth,
                    ),
                };
                retain_stronger_source(
                    &mut sources[edge.cells[side]],
                    BoundarySource {
                        effect,
                        maximum_depth,
                        plate: plates[side],
                    },
                );
            }
            continue;
        }

        let effect = match class {
            BoundaryClass::Convergent => config.convergent_lift,
            BoundaryClass::Divergent => config.divergent_drop,
            BoundaryClass::Transform => config.transform_lift,
            BoundaryClass::Interior => unreachable!(),
        } * scale;
        for side in 0..2 {
            retain_stronger_source(
                &mut sources[edge.cells[side]],
                BoundarySource {
                    effect,
                    maximum_depth: config.propagation_depth,
                    plate: plates[side],
                },
            );
        }
    }

    let boundary_source_cell_count = sources.iter().flatten().count();
    let effects = propagate_boundary_effects(mesh, partition, &sources);
    let boundary_affected_cell_count = effects.iter().filter(|&&effect| effect != 0.0).count();
    for (value, effect) in elevation.iter_mut().zip(effects) {
        *value += effect;
    }

    smooth(
        mesh,
        &mut elevation,
        config.smoothing_passes,
        config.smoothing_weight,
    );
    elevation
        .iter_mut()
        .for_each(|value| *value = value.clamp(0.0, 1.0));

    let minimum = elevation
        .iter()
        .copied()
        .min_by(f32::total_cmp)
        .unwrap_or(0.0);
    let maximum = elevation
        .iter()
        .copied()
        .max_by(f32::total_cmp)
        .unwrap_or(0.0);
    let mean = elevation.iter().map(|&value| value as f64).sum::<f64>() / elevation.len() as f64;

    Ok(CoarseElevation {
        cell_elevations: elevation,
        diagnostics: CoarseElevationDiagnostics {
            minimum,
            maximum,
            mean: mean as f32,
            boundary_source_cell_count,
            boundary_affected_cell_count,
        },
    })
}

fn validate_inputs(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    boundaries: &BoundaryClassification,
    config: CoarseElevationConfig,
) -> Result<(), CoarseElevationError> {
    let values = [
        config.oceanic_base,
        config.continental_base,
        config.convergent_lift,
        config.divergent_drop,
        config.transform_lift,
        config.continental_convergent_lift,
        config.oceanic_trench_drop,
        config.smoothing_weight,
    ];
    if values.iter().any(|value| !value.is_finite())
        || !(0.0..=1.0).contains(&config.oceanic_base)
        || !(0.0..=1.0).contains(&config.continental_base)
        || !(0.0..=1.0).contains(&config.smoothing_weight)
    {
        return Err(CoarseElevationError::InvalidConfig);
    }
    if partition.cell_plates.len() != mesh.cell_count() {
        return Err(CoarseElevationError::CellCountMismatch);
    }
    if crust.plate_classes.len() != partition.plate_count {
        return Err(CoarseElevationError::PlateCountMismatch);
    }
    if !boundaries.matches_edge_count(mesh.edge_count()) {
        return Err(CoarseElevationError::BoundaryCountMismatch);
    }
    Ok(())
}

fn retain_stronger_source(slot: &mut Option<BoundarySource>, candidate: BoundarySource) {
    if candidate.effect != 0.0
        && slot.is_none_or(|current| candidate.effect.abs() > current.effect.abs())
    {
        *slot = Some(candidate);
    }
}

fn propagate_boundary_effects(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    sources: &[Option<BoundarySource>],
) -> Vec<f32> {
    let mut effects = vec![0.0_f32; mesh.cell_count()];
    let mut seen_at = vec![usize::MAX; mesh.cell_count()];
    let mut queue = VecDeque::new();

    // Cell order provides a stable tie break when equal-magnitude sources overlap.
    for (source_cell, source) in sources.iter().enumerate() {
        let Some(source) = source else { continue };
        queue.clear();
        queue.push_back((source_cell, 0_usize));
        seen_at[source_cell] = source_cell;

        while let Some((cell, depth)) = queue.pop_front() {
            let decay = 1.0 - depth as f32 / (source.maximum_depth as f32 + 1.0);
            let effect = source.effect * decay;
            if effect.abs() > effects[cell].abs() {
                effects[cell] = effect;
            }
            if depth == source.maximum_depth {
                continue;
            }

            for corner in mesh.cell_corners(cell) {
                let neighbor = corner.neighbor;
                if seen_at[neighbor] == source_cell
                    || partition.cell_plates[neighbor] != source.plate
                {
                    continue;
                }
                seen_at[neighbor] = source_cell;
                queue.push_back((neighbor, depth + 1));
            }
        }
    }
    effects
}

fn smooth(mesh: &SphereMesh, elevation: &mut Vec<f32>, passes: usize, weight: f32) {
    let mut next = vec![0.0; elevation.len()];
    for _ in 0..passes {
        for cell in 0..mesh.cell_count() {
            let neighbors = mesh.cell_corners(cell);
            let average = neighbors
                .iter()
                .map(|corner| elevation[corner.neighbor])
                .sum::<f32>()
                / neighbors.len() as f32;
            next[cell] = elevation[cell] + weight * (average - elevation[cell]);
        }
        std::mem::swap(elevation, &mut next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{reference_partition, two_plate_boundary_partition};
    use crate::{
        CrustClassificationConfig, PlateEvolutionConfig, PlateKinematicsConfig, classify_crust,
        evolve_plate_ownership, generate_plate_kinematics,
    };

    fn final_fixture() -> (
        SphereMesh,
        PlatePartition,
        CrustClassification,
        BoundaryClassification,
    ) {
        let (mesh, initial) = reference_partition();
        let crust = classify_crust(&mesh, &initial, CrustClassificationConfig::new(17)).unwrap();
        let kinematics =
            generate_plate_kinematics(initial.plate_count, PlateKinematicsConfig::new(7)).unwrap();
        let evolution = evolve_plate_ownership(
            &mesh,
            &initial,
            &crust,
            &kinematics,
            PlateEvolutionConfig::default(),
        )
        .unwrap();
        (mesh, evolution.partition, crust, evolution.boundaries)
    }

    #[test]
    fn coarse_elevation_is_deterministic_normalized_and_stable() {
        let (mesh, partition, crust, boundaries) = final_fixture();
        let config = CoarseElevationConfig::default();
        let first =
            derive_coarse_elevation(&mesh, &partition, &crust, &boundaries, config).unwrap();

        assert_eq!(
            first,
            derive_coarse_elevation(&mesh, &partition, &crust, &boundaries, config).unwrap()
        );
        assert!(
            first
                .cell_elevations
                .iter()
                .all(|value| (0.0..=1.0).contains(value))
        );
        assert!(first.diagnostics.boundary_source_cell_count > 0);
        assert!(
            first.diagnostics.boundary_affected_cell_count
                >= first.diagnostics.boundary_source_cell_count
        );

        let fingerprint = first
            .cell_elevations
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, value| {
                (hash ^ value.to_bits() as u64).wrapping_mul(0x0000_0100_0000_01b3)
            });
        assert_eq!(fingerprint, 331_613_464_443_760_569);
    }

    #[test]
    fn base_elevation_follows_current_owner_crust() {
        let (mesh, _, mut partition) = two_plate_boundary_partition();
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Oceanic, CrustClass::Continental],
        };
        let boundaries = empty_boundaries(&mesh);
        let config = CoarseElevationConfig {
            smoothing_passes: 0,
            ..Default::default()
        };

        let original =
            derive_coarse_elevation(&mesh, &partition, &crust, &boundaries, config).unwrap();
        let cell = mesh.edges[0].cells[0];
        assert_eq!(original.cell_elevations[cell], config.oceanic_base);

        partition.cell_plates[cell] = 1;
        let changed =
            derive_coarse_elevation(&mesh, &partition, &crust, &boundaries, config).unwrap();
        assert_eq!(changed.cell_elevations[cell], config.continental_base);
    }

    #[test]
    fn mixed_convergence_creates_continental_lift_and_oceanic_trench() {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Continental, CrustClass::Oceanic],
        };
        let mut boundaries = empty_boundaries(&mesh);
        boundaries.edge_classes[edge_index] = BoundaryClass::Convergent;
        boundaries.edge_normal_speeds[edge_index] = [1.0, 1.0];
        let config = CoarseElevationConfig {
            continental_convergent_depth: 0,
            oceanic_trench_depth: 0,
            smoothing_passes: 0,
            ..Default::default()
        };

        let result =
            derive_coarse_elevation(&mesh, &partition, &crust, &boundaries, config).unwrap();

        assert!(result.cell_elevations[edge.cells[0]] > config.continental_base);
        assert!(result.cell_elevations[edge.cells[1]] < config.oceanic_base);
        assert_eq!(result.diagnostics.boundary_source_cell_count, 2);
        assert_eq!(result.diagnostics.boundary_affected_cell_count, 2);
    }

    #[test]
    fn zero_propagation_depth_limits_effects_to_boundary_cells() {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Continental; 2],
        };
        let mut boundaries = empty_boundaries(&mesh);
        boundaries.edge_classes[edge_index] = BoundaryClass::Transform;
        boundaries.edge_normal_speeds[edge_index] = [1.0, 0.0];
        let config = CoarseElevationConfig {
            propagation_depth: 0,
            smoothing_passes: 0,
            ..Default::default()
        };

        let result =
            derive_coarse_elevation(&mesh, &partition, &crust, &boundaries, config).unwrap();

        assert_eq!(result.diagnostics.boundary_source_cell_count, 2);
        assert_eq!(result.diagnostics.boundary_affected_cell_count, 2);
        for cell in 0..mesh.cell_count() {
            let expected = if edge.cells.contains(&cell) {
                config.continental_base + config.transform_lift * 0.5
            } else {
                config.continental_base
            };
            assert_eq!(result.cell_elevations[cell], expected);
        }
    }

    #[test]
    fn rejects_invalid_configuration_and_mismatched_inputs() {
        let (mesh, partition, crust, boundaries) = final_fixture();
        assert_eq!(
            derive_coarse_elevation(
                &mesh,
                &partition,
                &crust,
                &boundaries,
                CoarseElevationConfig {
                    smoothing_weight: 1.1,
                    ..Default::default()
                }
            ),
            Err(CoarseElevationError::InvalidConfig)
        );

        let mut short_partition = partition.clone();
        short_partition.cell_plates.pop();
        assert_eq!(
            derive_coarse_elevation(
                &mesh,
                &short_partition,
                &crust,
                &boundaries,
                CoarseElevationConfig::default()
            ),
            Err(CoarseElevationError::CellCountMismatch)
        );

        let mut short_boundaries = boundaries.clone();
        short_boundaries.edge_classes.pop();
        assert_eq!(
            derive_coarse_elevation(
                &mesh,
                &partition,
                &crust,
                &short_boundaries,
                CoarseElevationConfig::default()
            ),
            Err(CoarseElevationError::BoundaryCountMismatch)
        );
    }

    fn empty_boundaries(mesh: &SphereMesh) -> BoundaryClassification {
        BoundaryClassification {
            edge_classes: vec![BoundaryClass::Interior; mesh.edge_count()],
            edge_normal_speeds: vec![[0.0; 2]; mesh.edge_count()],
            edge_shear: vec![0.0; mesh.edge_count()],
        }
    }
}
