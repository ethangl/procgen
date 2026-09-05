use crate::{
    BoundaryClass, BoundaryClassification, CrustClass, CrustClassification, FieldSummary,
    PlatePartition,
    field::summarize_field,
    stage::{StageInputError, validate_boundaries, validate_ownership_and_crust},
};
use procgen_sphere_mesh::SphereMesh;
use std::{collections::VecDeque, fmt};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryEffect {
    /// Signed deformation at the boundary cell.
    pub offset: f32,
    /// Mesh hops the effect propagates within the current owning plate.
    pub depth: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryDeformationConfig {
    pub convergent: BoundaryEffect,
    pub divergent: BoundaryEffect,
    pub transform: BoundaryEffect,
    /// Continental side of a mixed-crust convergent boundary.
    pub collision: BoundaryEffect,
    /// Oceanic side of a mixed-crust convergent boundary.
    pub trench: BoundaryEffect,
    /// Motion magnitude at which a boundary effect reaches its full offset.
    pub saturation_speed: f32,
}

impl Default for BoundaryDeformationConfig {
    fn default() -> Self {
        Self {
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
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoundaryDeformationDiagnostics {
    pub summary: FieldSummary,
    pub source_cell_count: usize,
    pub uplifted_cell_count: usize,
    pub subsided_cell_count: usize,
}

impl BoundaryDeformationDiagnostics {
    fn summarize(deformation: &[f32], source_cell_count: usize) -> Self {
        let mut uplifted_cell_count = 0;
        let mut subsided_cell_count = 0;
        let summary = summarize_field(deformation, |value| {
            uplifted_cell_count += usize::from(value > 0.0);
            subsided_cell_count += usize::from(value < 0.0);
        });
        Self {
            summary,
            source_cell_count,
            uplifted_cell_count,
            subsided_cell_count,
        }
    }

    pub const fn affected_cell_count(&self) -> usize {
        self.uplifted_cell_count + self.subsided_cell_count
    }
}

/// Signed per-cell deformation derived only from the final tectonic state.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryDeformation {
    pub cell_deformation: Vec<f32>,
    pub diagnostics: BoundaryDeformationDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryDeformationError {
    InvalidConfig,
    Input(StageInputError),
}

impl fmt::Display for BoundaryDeformationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str(
                "deformation offsets must be finite and saturation speed must be finite and positive",
            ),
            Self::Input(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for BoundaryDeformationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidConfig => None,
            Self::Input(error) => Some(error),
        }
    }
}

impl From<StageInputError> for BoundaryDeformationError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
    }
}

/// Derives signed boundary deformation from final ownership, current-owner
/// crust, and final boundary classes and strengths.
///
/// Each boundary cell retains the strongest local source by absolute
/// magnitude. Sources then propagate for a bounded number of mesh hops without
/// crossing the final owning plate. Later overlaps also use maximum absolute
/// magnitude; stable cell iteration makes equal-magnitude ties deterministic.
pub fn derive_boundary_deformation(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    boundaries: &BoundaryClassification,
    config: BoundaryDeformationConfig,
) -> Result<BoundaryDeformation, BoundaryDeformationError> {
    validate_config(config)?;
    validate_ownership_and_crust(mesh, partition, crust)?;
    validate_boundaries(mesh, boundaries)?;

    let sources = collect_boundary_sources(mesh, partition, crust, boundaries, &config);
    let source_cell_count = sources.iter().flatten().count();
    let cell_deformation = propagate_boundary_effects(mesh, partition, &sources);
    let diagnostics =
        BoundaryDeformationDiagnostics::summarize(&cell_deformation, source_cell_count);

    Ok(BoundaryDeformation {
        cell_deformation,
        diagnostics,
    })
}

fn collect_boundary_sources(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    boundaries: &BoundaryClassification,
    config: &BoundaryDeformationConfig,
) -> Vec<Option<BoundaryEffect>> {
    let mut sources = vec![None; mesh.cell_count()];
    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        let class = boundaries.edge_classes[edge_index];
        let Some(strength) = boundaries.strength(edge_index) else {
            continue;
        };

        let classes = edge.cells.map(|cell| crust.cell_class(partition, cell));
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
    config: &BoundaryDeformationConfig,
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

fn validate_config(config: BoundaryDeformationConfig) -> Result<(), BoundaryDeformationError> {
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
    {
        return Err(BoundaryDeformationError::InvalidConfig);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        empty_boundaries, final_state_fixture, fingerprint, mesh as test_mesh,
        two_plate_boundary_partition,
    };

    #[test]
    fn deformation_is_deterministic_signed_and_stable() {
        let (mesh, partition, crust, boundaries) = final_state_fixture();
        let config = BoundaryDeformationConfig::default();
        let first =
            derive_boundary_deformation(&mesh, &partition, &crust, &boundaries, config).unwrap();

        assert_eq!(
            first,
            derive_boundary_deformation(&mesh, &partition, &crust, &boundaries, config).unwrap()
        );
        assert!(first.diagnostics.summary.minimum < 0.0);
        assert!(first.diagnostics.summary.maximum > 0.0);
        assert_eq!(
            first.diagnostics.affected_cell_count(),
            first.diagnostics.uplifted_cell_count + first.diagnostics.subsided_cell_count
        );
        assert!(first.diagnostics.affected_cell_count() >= first.diagnostics.source_cell_count);

        let fingerprint = fingerprint(
            first
                .cell_deformation
                .iter()
                .map(|value| value.to_bits() as u64),
        );
        assert_eq!(fingerprint, 16_915_549_137_106_129_144);
    }

    #[test]
    fn mixed_convergence_uses_current_owner_crust_for_uplift_and_trench() {
        let (mesh, edge_index, mut partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Continental, CrustClass::Oceanic],
        };
        let mut boundaries = empty_boundaries(&mesh);
        boundaries.edge_classes[edge_index] = BoundaryClass::Convergent;
        boundaries.edge_normal_speeds[edge_index] = [1.0, 1.0];
        let config = BoundaryDeformationConfig {
            collision: BoundaryEffect {
                depth: 0,
                ..BoundaryDeformationConfig::default().collision
            },
            trench: BoundaryEffect {
                depth: 0,
                ..BoundaryDeformationConfig::default().trench
            },
            ..Default::default()
        };

        let original =
            derive_boundary_deformation(&mesh, &partition, &crust, &boundaries, config).unwrap();
        assert!(original.cell_deformation[edge.cells[0]] > 0.0);
        assert!(original.cell_deformation[edge.cells[1]] < 0.0);

        partition.cell_plates.swap(edge.cells[0], edge.cells[1]);
        let changed =
            derive_boundary_deformation(&mesh, &partition, &crust, &boundaries, config).unwrap();
        assert!(changed.cell_deformation[edge.cells[0]] < 0.0);
        assert!(changed.cell_deformation[edge.cells[1]] > 0.0);
    }

    #[test]
    fn divergent_uses_normal_strength_and_transform_uses_shear() {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Continental; 2],
        };
        let config = BoundaryDeformationConfig {
            divergent: BoundaryEffect {
                depth: 0,
                ..BoundaryDeformationConfig::default().divergent
            },
            transform: BoundaryEffect {
                depth: 0,
                ..BoundaryDeformationConfig::default().transform
            },
            saturation_speed: 4.0,
            ..Default::default()
        };
        let mut boundaries = empty_boundaries(&mesh);
        boundaries.edge_classes[edge_index] = BoundaryClass::Divergent;
        boundaries.edge_normal_speeds[edge_index] = [-0.5, -0.5];
        boundaries.edge_shear[edge_index] = 3.0;
        let divergent =
            derive_boundary_deformation(&mesh, &partition, &crust, &boundaries, config).unwrap();
        for cell in edge.cells {
            assert_eq!(
                divergent.cell_deformation[cell],
                config.divergent.offset / 4.0
            );
        }

        boundaries.edge_classes[edge_index] = BoundaryClass::Transform;
        let transform =
            derive_boundary_deformation(&mesh, &partition, &crust, &boundaries, config).unwrap();
        for cell in edge.cells {
            assert_eq!(
                transform.cell_deformation[cell],
                config.transform.offset * 3.0 / 4.0
            );
        }
    }

    #[test]
    fn propagation_is_bounded_to_the_current_plate() {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Continental; 2],
        };
        let mut boundaries = empty_boundaries(&mesh);
        boundaries.edge_classes[edge_index] = BoundaryClass::Convergent;
        boundaries.edge_normal_speeds[edge_index] = [1.0, 1.0];
        let config = BoundaryDeformationConfig {
            convergent: BoundaryEffect {
                offset: 0.4,
                depth: 1,
            },
            ..Default::default()
        };

        let deformation =
            derive_boundary_deformation(&mesh, &partition, &crust, &boundaries, config).unwrap();
        assert_eq!(deformation.cell_deformation[edge.cells[1]], 0.4);
        assert_eq!(
            deformation
                .cell_deformation
                .iter()
                .enumerate()
                .filter(|(cell, value)| partition.cell_plates[*cell] == 1 && **value != 0.0)
                .count(),
            1
        );
        assert!(
            deformation
                .cell_deformation
                .iter()
                .enumerate()
                .any(|(cell, &value)| partition.cell_plates[cell] == 0 && value == 0.2)
        );
    }

    #[test]
    fn maximum_magnitude_wins_and_equal_ties_keep_the_first_source() {
        let mut source = None;
        let first = BoundaryEffect {
            offset: -0.4,
            depth: 1,
        };
        retain_stronger_source(&mut source, first);
        retain_stronger_source(
            &mut source,
            BoundaryEffect {
                offset: 0.4,
                depth: 7,
            },
        );
        assert_eq!(source, Some(first));

        let stronger = BoundaryEffect {
            offset: 0.5,
            depth: 2,
        };
        retain_stronger_source(&mut source, stronger);
        assert_eq!(source, Some(stronger));

        let mesh = test_mesh(32);
        let mut source_cells = mesh.edges[0].cells;
        source_cells.sort();
        let overlap = mesh
            .cell_corners(source_cells[0])
            .iter()
            .map(|corner| corner.neighbor)
            .find(|&cell| {
                cell != source_cells[1]
                    && mesh
                        .cell_corners(source_cells[1])
                        .iter()
                        .any(|corner| corner.neighbor == cell)
            })
            .unwrap();
        let partition = PlatePartition {
            cell_plates: vec![0; mesh.cell_count()],
            plate_count: 1,
        };
        let mut sources = vec![None; mesh.cell_count()];
        sources[source_cells[0]] = Some(BoundaryEffect {
            offset: -0.4,
            depth: 1,
        });
        sources[source_cells[1]] = Some(BoundaryEffect {
            offset: 0.4,
            depth: 1,
        });
        let propagated = propagate_boundary_effects(&mesh, &partition, &sources);
        assert_eq!(propagated[overlap], -0.2);
    }

    #[test]
    fn rejects_invalid_configuration_and_mismatched_inputs() {
        let (mesh, partition, crust, boundaries) = final_state_fixture();
        assert_eq!(
            derive_boundary_deformation(
                &mesh,
                &partition,
                &crust,
                &boundaries,
                BoundaryDeformationConfig {
                    saturation_speed: 0.0,
                    ..Default::default()
                }
            ),
            Err(BoundaryDeformationError::InvalidConfig)
        );

        let mut short_partition = partition.clone();
        short_partition.cell_plates.pop();
        assert_eq!(
            derive_boundary_deformation(
                &mesh,
                &short_partition,
                &crust,
                &boundaries,
                BoundaryDeformationConfig::default()
            ),
            Err(BoundaryDeformationError::Input(StageInputError::Cells))
        );

        let short_crust = CrustClassification {
            plate_classes: crust.plate_classes[..crust.plate_classes.len() - 1].to_vec(),
        };
        assert_eq!(
            derive_boundary_deformation(
                &mesh,
                &partition,
                &short_crust,
                &boundaries,
                BoundaryDeformationConfig::default()
            ),
            Err(BoundaryDeformationError::Input(StageInputError::Plates))
        );

        let mut short_boundaries = boundaries.clone();
        short_boundaries.edge_classes.pop();
        assert_eq!(
            derive_boundary_deformation(
                &mesh,
                &partition,
                &crust,
                &short_boundaries,
                BoundaryDeformationConfig::default()
            ),
            Err(BoundaryDeformationError::Input(StageInputError::Boundaries))
        );
    }
}
