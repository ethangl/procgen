use crate::{
    BoundaryClass, BoundaryClassification, CrustClass, CrustClassification, PlatePartition,
};
use procgen_sphere_mesh::SphereMesh;
use std::{collections::VecDeque, fmt};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryEffect {
    /// Signed elevation offset at the boundary cell.
    pub offset: f32,
    /// Mesh hops the effect propagates within the owning plate.
    pub depth: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoarseElevationConfig {
    pub oceanic_base: f32,
    pub continental_base: f32,
    pub convergent: BoundaryEffect,
    pub divergent: BoundaryEffect,
    pub transform: BoundaryEffect,
    /// Continental side of a mixed-crust convergent boundary.
    pub collision: BoundaryEffect,
    /// Oceanic side of a mixed-crust convergent boundary.
    pub trench: BoundaryEffect,
    /// Motion magnitude at which a boundary effect reaches its full offset.
    pub saturation_speed: f32,
    pub smoothing_passes: usize,
    pub smoothing_weight: f32,
}

impl Default for CoarseElevationConfig {
    fn default() -> Self {
        Self {
            oceanic_base: 0.15,
            continental_base: 0.65,
            convergent: BoundaryEffect {
                offset: 0.4,
                depth: 3,
            },
            divergent: BoundaryEffect {
                offset: -0.4,
                depth: 3,
            },
            transform: BoundaryEffect {
                offset: 0.4,
                depth: 3,
            },
            collision: BoundaryEffect {
                offset: 0.5,
                depth: 5,
            },
            trench: BoundaryEffect {
                offset: -0.2,
                depth: 1,
            },
            saturation_speed: 2.0,
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

impl CoarseElevationDiagnostics {
    fn summarize(elevations: &[f32], source_cells: usize, affected_cells: usize) -> Self {
        let Some((&first, rest)) = elevations.split_first() else {
            return Self {
                boundary_source_cell_count: source_cells,
                boundary_affected_cell_count: affected_cells,
                ..Self::default()
            };
        };
        let (minimum, maximum, total) = rest.iter().fold(
            (first, first, f64::from(first)),
            |(minimum, maximum, total), &value| {
                (
                    minimum.min(value),
                    maximum.max(value),
                    total + f64::from(value),
                )
            },
        );
        Self {
            minimum,
            maximum,
            mean: (total / elevations.len() as f64) as f32,
            boundary_source_cell_count: source_cells,
            boundary_affected_cell_count: affected_cells,
        }
    }
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
                "elevation values must be finite, saturation speed must be positive, and bases and smoothing weight must be between 0 and 1",
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
    let sources = collect_boundary_sources(mesh, partition, crust, boundaries, &config);

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

    let diagnostics = CoarseElevationDiagnostics::summarize(
        &elevation,
        boundary_source_cell_count,
        boundary_affected_cell_count,
    );

    Ok(CoarseElevation {
        cell_elevations: elevation,
        diagnostics,
    })
}

fn collect_boundary_sources(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    boundaries: &BoundaryClassification,
    config: &CoarseElevationConfig,
) -> Vec<Option<BoundaryEffect>> {
    let mut sources = vec![None; mesh.cell_count()];
    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        let class = boundaries.edge_classes[edge_index];
        let Some(strength) = boundaries.strength(edge_index) else {
            continue;
        };

        let classes = edge
            .cells
            .map(|cell| crust.plate_classes[partition.cell_plates[cell]]);
        let scale = (strength / config.saturation_speed).min(1.0);
        for side in 0..2 {
            let effect = boundary_effect(config, class, classes[side], classes[1 - side]);
            retain_stronger_source(
                &mut sources[edge.cells[side]],
                BoundaryEffect {
                    offset: effect.offset * scale,
                    ..effect
                },
            );
        }
    }
    sources
}

fn boundary_effect(
    config: &CoarseElevationConfig,
    class: BoundaryClass,
    own: CrustClass,
    other: CrustClass,
) -> BoundaryEffect {
    match (class, own, other) {
        (BoundaryClass::Convergent, CrustClass::Continental, CrustClass::Oceanic) => {
            config.collision
        }
        (BoundaryClass::Convergent, CrustClass::Oceanic, CrustClass::Continental) => config.trench,
        (BoundaryClass::Convergent, _, _) => config.convergent,
        (BoundaryClass::Divergent, _, _) => config.divergent,
        (BoundaryClass::Transform, _, _) => config.transform,
        (BoundaryClass::Interior, _, _) => unreachable!("interior edges are skipped"),
    }
}

fn validate_inputs(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    boundaries: &BoundaryClassification,
    config: CoarseElevationConfig,
) -> Result<(), CoarseElevationError> {
    let effects = [
        config.convergent,
        config.divergent,
        config.transform,
        config.collision,
        config.trench,
    ];
    if effects.iter().any(|effect| !effect.offset.is_finite())
        || !config.saturation_speed.is_finite()
        || config.saturation_speed <= 0.0
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

fn retain_stronger_source(slot: &mut Option<BoundaryEffect>, candidate: BoundaryEffect) {
    if candidate.offset != 0.0
        && slot.is_none_or(|current| candidate.offset.abs() > current.offset.abs())
    {
        *slot = Some(candidate);
    }
}

fn propagate_boundary_effects(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    sources: &[Option<BoundaryEffect>],
) -> Vec<f32> {
    let mut effects = vec![0.0_f32; mesh.cell_count()];
    let mut seen_at = vec![usize::MAX; mesh.cell_count()];
    let mut queue = VecDeque::new();

    // Cell order provides a stable tie break when equal-magnitude sources overlap.
    for (source_cell, source) in sources.iter().enumerate() {
        let Some(source) = source else { continue };
        let source_plate = partition.cell_plates[source_cell];
        queue.clear();
        queue.push_back((source_cell, 0_usize));
        seen_at[source_cell] = source_cell;

        while let Some((cell, depth)) = queue.pop_front() {
            let decay = 1.0 - depth as f32 / (source.depth as f32 + 1.0);
            let effect = source.offset * decay;
            if effect.abs() > effects[cell].abs() {
                effects[cell] = effect;
            }
            if depth == source.depth {
                continue;
            }

            for corner in mesh.cell_corners(cell) {
                let neighbor = corner.neighbor;
                if seen_at[neighbor] == source_cell
                    || partition.cell_plates[neighbor] != source_plate
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
    use crate::test_support::{
        empty_boundaries, fingerprint, reference_partition, two_plate_boundary_partition,
    };
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

        let fingerprint = fingerprint(
            first
                .cell_elevations
                .iter()
                .map(|value| value.to_bits() as u64),
        );
        assert_eq!(fingerprint, 4_072_787_338_629_474_632);
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
            collision: BoundaryEffect {
                depth: 0,
                ..CoarseElevationConfig::default().collision
            },
            trench: BoundaryEffect {
                depth: 0,
                ..CoarseElevationConfig::default().trench
            },
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
    fn transform_uses_shear_explicit_saturation_and_zero_depth() {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Continental; 2],
        };
        let mut boundaries = empty_boundaries(&mesh);
        boundaries.edge_classes[edge_index] = BoundaryClass::Transform;
        boundaries.edge_shear[edge_index] = 1.0;
        let config = CoarseElevationConfig {
            transform: BoundaryEffect {
                depth: 0,
                ..CoarseElevationConfig::default().transform
            },
            saturation_speed: 4.0,
            smoothing_passes: 0,
            ..Default::default()
        };

        let result =
            derive_coarse_elevation(&mesh, &partition, &crust, &boundaries, config).unwrap();

        assert_eq!(result.diagnostics.boundary_source_cell_count, 2);
        assert_eq!(result.diagnostics.boundary_affected_cell_count, 2);
        for cell in 0..mesh.cell_count() {
            let expected = if edge.cells.contains(&cell) {
                config.continental_base + config.transform.offset / config.saturation_speed
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
        assert_eq!(
            derive_coarse_elevation(
                &mesh,
                &partition,
                &crust,
                &boundaries,
                CoarseElevationConfig {
                    saturation_speed: 0.0,
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

        let short_crust = CrustClassification {
            plate_classes: crust.plate_classes[..crust.plate_classes.len() - 1].to_vec(),
        };
        assert_eq!(
            derive_coarse_elevation(
                &mesh,
                &partition,
                &short_crust,
                &boundaries,
                CoarseElevationConfig::default()
            ),
            Err(CoarseElevationError::PlateCountMismatch)
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
}
