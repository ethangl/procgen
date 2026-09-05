use crate::field::MaxWinsField;
use procgen_sphere_mesh::SphereMesh;
use procgen_tectonics::{
    BoundaryClass, BoundaryClassification, CrustClass, CrustClassification, PlatePartition,
    StageInputError,
};
use std::{collections::VecDeque, fmt};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolcanicArcFieldConfig {
    /// Minimum qualifying boundary edges required to retain a segment.
    pub minimum_boundary_edges: usize,
    /// Desired graph distance from the boundary on the overriding plate.
    pub inland_offset_cells: usize,
    /// Selects approximately one peak candidate per this many inland cells,
    /// retaining the strongest candidates first.
    pub peak_density_divisor: usize,
    /// Convergence at which diagnostic strength reaches one.
    pub strength_saturation: f32,
}

impl Default for VolcanicArcFieldConfig {
    fn default() -> Self {
        Self {
            minimum_boundary_edges: 3,
            inland_offset_cells: 2,
            peak_density_divisor: 2,
            strength_saturation: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolcanicArcCell {
    pub cell: usize,
    /// Normalized strength propagated from the strongest nearest boundary source.
    pub strength: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolcanicPeakCandidate {
    pub cell: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VolcanicArcSegment {
    pub overriding_plate: usize,
    /// Qualifying mesh edge ids in ascending order.
    pub boundary_edges: Vec<usize>,
    /// Continental boundary cells in ascending order.
    pub boundary_cells: Vec<usize>,
    /// Inland cells in ascending order.
    pub arc_cells: Vec<VolcanicArcCell>,
    /// Peak candidates in ascending cell order.
    pub peaks: Vec<VolcanicPeakCandidate>,
    /// Actual inland depth used, which may be shallower than the requested bound.
    pub inland_depth: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VolcanicArcDiagnostics {
    pub qualifying_edge_count: usize,
    pub boundary_cell_count: usize,
    pub discarded_short_segment_count: usize,
    pub discarded_landlocked_segment_count: usize,
    pub arc_cell_count: usize,
    pub affected_cell_count: usize,
    pub overlap_cell_count: usize,
    pub peak_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VolcanicArcField {
    pub segments: Vec<VolcanicArcSegment>,
    /// Max-wins aggregate strength, independent of elevation.
    pub cell_strengths: Vec<f32>,
    /// Winning segment for each affected cell. Equal strengths resolve to the
    /// lower stable segment index.
    pub cell_segments: Vec<Option<usize>>,
    pub diagnostics: VolcanicArcDiagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolcanicArcFieldError {
    Input(StageInputError),
    EmptyMinimumSegment,
    ZeroInlandOffset,
    ZeroPeakDensityDivisor,
    InvalidStrengthSaturation,
}

impl fmt::Display for VolcanicArcFieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => error.fmt(formatter),
            Self::EmptyMinimumSegment => {
                formatter.write_str("minimum boundary edges must be at least one")
            }
            Self::ZeroInlandOffset => formatter.write_str("inland offset must be at least one"),
            Self::ZeroPeakDensityDivisor => {
                formatter.write_str("peak density divisor must be at least one")
            }
            Self::InvalidStrengthSaturation => {
                formatter.write_str("strength saturation must be finite and greater than zero")
            }
        }
    }
}

impl std::error::Error for VolcanicArcFieldError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StageInputError> for VolcanicArcFieldError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
    }
}

#[derive(Clone, Copy)]
struct InlandClaim {
    strength: f32,
    source_edge: usize,
}

struct BoundaryData {
    edges_by_cell: Vec<Vec<usize>>,
    claims: Vec<Option<InlandClaim>>,
    qualifying_edge_count: usize,
}

struct BoundaryGroup {
    overriding_plate: usize,
    boundary_edges: Vec<usize>,
    boundary_cells: Vec<usize>,
}

/// Derives present-day volcanic-arc fields from final mixed-crust convergent
/// boundaries. This operation does not read or modify elevation.
pub fn derive_volcanic_arc_field(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: &CrustClassification,
    boundaries: &BoundaryClassification,
    config: VolcanicArcFieldConfig,
) -> Result<VolcanicArcField, VolcanicArcFieldError> {
    validate_inputs(mesh, plates, crust, boundaries, config)?;

    let boundary = collect_boundary_data(mesh, plates, crust, boundaries, config);
    let boundary_cell_count = boundary.claims.iter().flatten().count();
    let mut groups = group_boundaries(mesh, plates, &boundary);
    let original_group_count = groups.len();
    groups.retain(|group| group.boundary_edges.len() >= config.minimum_boundary_edges);
    let discarded_short_segment_count = original_group_count - groups.len();

    let mut discarded_landlocked_segment_count = 0;
    let mut segments: Vec<_> = groups
        .into_iter()
        .filter_map(|group| {
            let segment = derive_segment(mesh, plates, &boundary.claims, config, group);
            discarded_landlocked_segment_count += usize::from(segment.is_none());
            segment
        })
        .collect();
    segments.sort_unstable_by_key(|segment| {
        (
            segment.overriding_plate,
            segment.boundary_edges[0],
            segment.boundary_cells[0],
        )
    });

    let mut aggregate = MaxWinsField::new(mesh.cell_count());
    for (segment_index, segment) in segments.iter().enumerate() {
        for arc_cell in &segment.arc_cells {
            aggregate.claim(arc_cell.cell, arc_cell.strength, segment_index);
        }
    }

    let arc_cell_count = segments.iter().map(|segment| segment.arc_cells.len()).sum();
    let peak_count = segments.iter().map(|segment| segment.peaks.len()).sum();
    let affected_cell_count = aggregate.affected_cell_count();
    let overlap_cell_count = aggregate.overlap_cell_count();
    let (cell_strengths, cell_segments) = aggregate.into_parts();

    Ok(VolcanicArcField {
        segments,
        cell_strengths,
        cell_segments,
        diagnostics: VolcanicArcDiagnostics {
            qualifying_edge_count: boundary.qualifying_edge_count,
            boundary_cell_count,
            discarded_short_segment_count,
            discarded_landlocked_segment_count,
            arc_cell_count,
            affected_cell_count,
            overlap_cell_count,
            peak_count,
        },
    })
}

fn validate_inputs(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: &CrustClassification,
    boundaries: &BoundaryClassification,
    config: VolcanicArcFieldConfig,
) -> Result<(), VolcanicArcFieldError> {
    if config.minimum_boundary_edges == 0 {
        return Err(VolcanicArcFieldError::EmptyMinimumSegment);
    }
    if config.inland_offset_cells == 0 {
        return Err(VolcanicArcFieldError::ZeroInlandOffset);
    }
    if config.peak_density_divisor == 0 {
        return Err(VolcanicArcFieldError::ZeroPeakDensityDivisor);
    }
    if !config.strength_saturation.is_finite() || config.strength_saturation <= 0.0 {
        return Err(VolcanicArcFieldError::InvalidStrengthSaturation);
    }
    plates.validate(mesh)?;
    crust.validate(plates)?;
    boundaries.validate(mesh)?;
    Ok(())
}

fn collect_boundary_data(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: &CrustClassification,
    boundaries: &BoundaryClassification,
    config: VolcanicArcFieldConfig,
) -> BoundaryData {
    let mut data = BoundaryData {
        edges_by_cell: vec![Vec::new(); mesh.cell_count()],
        claims: vec![None; mesh.cell_count()],
        qualifying_edge_count: 0,
    };
    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        if boundaries.edge_classes[edge_index] != BoundaryClass::Convergent {
            continue;
        }
        let classes = edge.cells.map(|cell| crust.cell_class(plates, cell));
        if classes[0] == classes[1] {
            continue;
        }
        let overriding_cell = if classes[0] == CrustClass::Continental {
            edge.cells[0]
        } else {
            edge.cells[1]
        };
        let claim = InlandClaim {
            strength: (boundaries.convergence(edge_index) / config.strength_saturation)
                .clamp(0.0, 1.0),
            source_edge: edge_index,
        };
        let current = &mut data.claims[overriding_cell];
        if current.is_none_or(|existing| claim_precedes(claim, existing)) {
            *current = Some(claim);
        }
        data.edges_by_cell[overriding_cell].push(edge_index);
        data.qualifying_edge_count += 1;
    }
    data
}

fn group_boundaries(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    boundary: &BoundaryData,
) -> Vec<BoundaryGroup> {
    let mut visited = vec![false; mesh.cell_count()];
    let mut groups = Vec::new();
    for start in 0..mesh.cell_count() {
        if visited[start] || boundary.claims[start].is_none() {
            continue;
        }
        let overriding_plate = plates.cell_plates[start];
        let mut queue = VecDeque::from([start]);
        let mut boundary_cells = Vec::new();
        let mut boundary_edges = Vec::new();
        visited[start] = true;
        while let Some(cell) = queue.pop_front() {
            boundary_cells.push(cell);
            boundary_edges.extend_from_slice(&boundary.edges_by_cell[cell]);
            for corner in mesh.cell_corners(cell) {
                let neighbor = corner.neighbor;
                if !visited[neighbor]
                    && boundary.claims[neighbor].is_some()
                    && plates.cell_plates[neighbor] == overriding_plate
                {
                    visited[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }
        boundary_cells.sort_unstable();
        boundary_edges.sort_unstable();
        boundary_edges.dedup();
        groups.push(BoundaryGroup {
            overriding_plate,
            boundary_edges,
            boundary_cells,
        });
    }
    groups
}

fn derive_segment(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    boundary_claims: &[Option<InlandClaim>],
    config: VolcanicArcFieldConfig,
    group: BoundaryGroup,
) -> Option<VolcanicArcSegment> {
    let (arc_cells, inland_depth) = walk_inland(
        mesh,
        plates,
        boundary_claims,
        group.overriding_plate,
        &group.boundary_cells,
        config.inland_offset_cells,
    )?;
    let peaks = select_peak_candidates(&arc_cells, config.peak_density_divisor);
    Some(VolcanicArcSegment {
        overriding_plate: group.overriding_plate,
        boundary_edges: group.boundary_edges,
        boundary_cells: group.boundary_cells,
        arc_cells,
        peaks,
        inland_depth,
    })
}

fn walk_inland(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    boundary_claims: &[Option<InlandClaim>],
    overriding_plate: usize,
    boundary_cells: &[usize],
    maximum_depth: usize,
) -> Option<(Vec<VolcanicArcCell>, usize)> {
    let mut visited: Vec<_> = boundary_claims.iter().map(Option::is_some).collect();
    let mut claim_buffer = vec![None; mesh.cell_count()];
    let mut touched = Vec::new();
    let mut frontier: Vec<_> = boundary_cells
        .iter()
        .filter_map(|&cell| boundary_claims[cell].map(|claim| (cell, claim)))
        .collect();
    let mut inland_depth = 0;

    for depth in 1..=maximum_depth {
        touched.clear();
        for &(cell, claim) in &frontier {
            for corner in mesh.cell_corners(cell) {
                let neighbor = corner.neighbor;
                if visited[neighbor] || plates.cell_plates[neighbor] != overriding_plate {
                    continue;
                }
                touched.push(neighbor);
                let slot = &mut claim_buffer[neighbor];
                if slot.is_none_or(|existing| claim_precedes(claim, existing)) {
                    *slot = Some(claim);
                }
            }
        }
        touched.sort_unstable();
        touched.dedup();
        let next: Vec<_> = touched
            .iter()
            .filter_map(|&cell| claim_buffer[cell].take().map(|claim| (cell, claim)))
            .collect();
        if next.is_empty() {
            break;
        }
        for &(cell, _) in &next {
            visited[cell] = true;
        }
        frontier = next;
        inland_depth = depth;
    }

    (inland_depth > 0).then(|| {
        let cells = frontier
            .into_iter()
            .map(|(cell, claim)| VolcanicArcCell {
                cell,
                strength: claim.strength,
            })
            .collect();
        (cells, inland_depth)
    })
}

fn select_peak_candidates(
    arc_cells: &[VolcanicArcCell],
    density_divisor: usize,
) -> Vec<VolcanicPeakCandidate> {
    let peak_count = arc_cells.len().div_ceil(density_divisor);
    let mut peak_cells: Vec<_> = arc_cells.iter().collect();
    peak_cells.sort_unstable_by(|left, right| {
        right
            .strength
            .total_cmp(&left.strength)
            .then_with(|| left.cell.cmp(&right.cell))
    });
    peak_cells.truncate(peak_count);
    peak_cells.sort_unstable_by_key(|arc_cell| arc_cell.cell);
    peak_cells
        .into_iter()
        .map(|arc_cell| VolcanicPeakCandidate {
            cell: arc_cell.cell,
        })
        .collect()
}

fn claim_precedes(candidate: InlandClaim, existing: InlandClaim) -> bool {
    candidate.strength > existing.strength
        || (candidate.strength == existing.strength && candidate.source_edge < existing.source_edge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_core::fingerprint;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::build_sphere_mesh;
    use procgen_tectonics::{
        CrustClassificationConfig, PlateKinematicsConfig, PlatePartitionConfig,
        classify_boundaries, classify_crust, generate_plate_kinematics, partition_plates,
    };

    fn fixture(
        cell_count: usize,
    ) -> (
        SphereMesh,
        PlatePartition,
        CrustClassification,
        BoundaryClassification,
    ) {
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
                major_plate_count: 8,
                minor_plate_count: 12,
                major_head_start_rounds: 2,
                seed: 11,
            },
        )
        .unwrap();
        let crust = classify_crust(
            &mesh,
            &plates,
            CrustClassificationConfig {
                target_ocean_fraction: 0.7,
                seed: 17,
            },
        )
        .unwrap();
        let kinematics =
            generate_plate_kinematics(plates.plate_count, PlateKinematicsConfig::new(13)).unwrap();
        let boundaries = classify_boundaries(&mesh, &plates, &kinematics).unwrap();
        (mesh, plates, crust, boundaries)
    }

    #[test]
    fn field_is_deterministic_ordered_and_bounded_inland() {
        let (mesh, plates, crust, boundaries) = fixture(1_024);
        let config = VolcanicArcFieldConfig::default();
        let field = derive_volcanic_arc_field(&mesh, &plates, &crust, &boundaries, config).unwrap();

        assert_eq!(
            field,
            derive_volcanic_arc_field(&mesh, &plates, &crust, &boundaries, config).unwrap()
        );
        assert!(!field.segments.is_empty());
        assert!(field.segments.windows(2).all(|pair| {
            (pair[0].overriding_plate, pair[0].boundary_edges[0])
                < (pair[1].overriding_plate, pair[1].boundary_edges[0])
        }));
        for segment in &field.segments {
            assert!(
                segment
                    .boundary_edges
                    .windows(2)
                    .all(|pair| pair[0] < pair[1])
            );
            assert!(
                segment
                    .boundary_cells
                    .windows(2)
                    .all(|pair| pair[0] < pair[1])
            );
            assert!(
                segment
                    .arc_cells
                    .windows(2)
                    .all(|pair| pair[0].cell < pair[1].cell)
            );
            assert!(
                segment
                    .peaks
                    .windows(2)
                    .all(|pair| pair[0].cell < pair[1].cell)
            );
            assert!((1..=config.inland_offset_cells).contains(&segment.inland_depth));
            assert!(segment.arc_cells.iter().all(|arc_cell| {
                plates.cell_plates[arc_cell.cell] == segment.overriding_plate
                    && crust.cell_class(&plates, arc_cell.cell) == CrustClass::Continental
                    && !segment.boundary_cells.contains(&arc_cell.cell)
            }));
            for &edge_index in &segment.boundary_edges {
                let edge = mesh.edges[edge_index];
                let classes = edge.cells.map(|cell| crust.cell_class(&plates, cell));
                let continental_cell = if classes[0] == CrustClass::Continental {
                    edge.cells[0]
                } else {
                    edge.cells[1]
                };
                assert_eq!(
                    boundaries.edge_classes[edge_index],
                    BoundaryClass::Convergent
                );
                assert_ne!(classes[0], classes[1]);
                assert_eq!(
                    plates.cell_plates[continental_cell],
                    segment.overriding_plate
                );
                assert!(segment.boundary_cells.contains(&continental_cell));
            }
        }
    }

    #[test]
    fn reference_field_has_stable_fingerprint() {
        let (mesh, plates, crust, boundaries) = fixture(1_024);
        let field = derive_volcanic_arc_field(
            &mesh,
            &plates,
            &crust,
            &boundaries,
            VolcanicArcFieldConfig::default(),
        )
        .unwrap();
        let values = field.segments.iter().flat_map(|segment| {
            [
                segment.overriding_plate as u64,
                segment.inland_depth as u64,
                segment.boundary_edges.len() as u64,
                segment.boundary_cells.len() as u64,
                segment.arc_cells.len() as u64,
                segment.peaks.len() as u64,
            ]
            .into_iter()
            .chain(segment.boundary_edges.iter().map(|&value| value as u64))
            .chain(segment.boundary_cells.iter().map(|&value| value as u64))
            .chain(segment.arc_cells.iter().flat_map(|arc_cell| {
                [arc_cell.cell as u64, u64::from(arc_cell.strength.to_bits())]
            }))
            .chain(segment.peaks.iter().map(|peak| peak.cell as u64))
        });

        assert_eq!(fingerprint(values), 5_753_623_188_708_739_124);
    }

    #[test]
    fn peaks_and_overlaps_follow_stable_strength_rules() {
        let (mesh, plates, crust, boundaries) = fixture(1_024);
        let config = VolcanicArcFieldConfig {
            minimum_boundary_edges: 1,
            inland_offset_cells: 3,
            peak_density_divisor: 2,
            strength_saturation: 2.0,
        };
        let field = derive_volcanic_arc_field(&mesh, &plates, &crust, &boundaries, config).unwrap();

        assert!(field.diagnostics.overlap_cell_count > 0);
        for segment in &field.segments {
            let mut expected: Vec<_> = segment.arc_cells.iter().collect();
            expected.sort_unstable_by(|left, right| {
                right
                    .strength
                    .total_cmp(&left.strength)
                    .then_with(|| left.cell.cmp(&right.cell))
            });
            expected.truncate(
                segment
                    .arc_cells
                    .len()
                    .div_ceil(config.peak_density_divisor),
            );
            let mut expected: Vec<_> = expected.into_iter().map(|arc_cell| arc_cell.cell).collect();
            expected.sort_unstable();
            assert_eq!(
                segment
                    .peaks
                    .iter()
                    .map(|peak| peak.cell)
                    .collect::<Vec<_>>(),
                expected
            );
        }
        for cell in 0..mesh.cell_count() {
            let expected = field
                .segments
                .iter()
                .enumerate()
                .flat_map(|(segment, data)| {
                    data.arc_cells
                        .iter()
                        .filter(move |arc_cell| arc_cell.cell == cell)
                        .map(move |arc_cell| (segment, arc_cell.strength))
                })
                .max_by(|(left_segment, left), (right_segment, right)| {
                    left.total_cmp(right)
                        .then_with(|| right_segment.cmp(left_segment))
                });
            assert_eq!(field.cell_segments[cell], expected.map(|value| value.0));
            assert_eq!(
                field.cell_strengths[cell],
                expected.map_or(0.0, |value| value.1)
            );
        }
    }

    #[test]
    fn no_mixed_convergence_produces_an_empty_field() {
        let (mesh, plates, mut crust, mut boundaries) = fixture(512);
        crust.plate_classes.fill(CrustClass::Continental);
        boundaries.edge_classes.fill(BoundaryClass::Convergent);
        let field = derive_volcanic_arc_field(
            &mesh,
            &plates,
            &crust,
            &boundaries,
            VolcanicArcFieldConfig::default(),
        )
        .unwrap();

        assert!(field.segments.is_empty());
        assert!(field.cell_strengths.iter().all(|&strength| strength == 0.0));
        assert!(field.cell_segments.iter().all(Option::is_none));
        assert_eq!(field.diagnostics, VolcanicArcDiagnostics::default());
    }

    #[test]
    fn minimum_edge_filter_reports_discarded_segments() {
        let (mesh, plates, crust, boundaries) = fixture(512);
        let field = derive_volcanic_arc_field(
            &mesh,
            &plates,
            &crust,
            &boundaries,
            VolcanicArcFieldConfig {
                minimum_boundary_edges: usize::MAX,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(field.diagnostics.qualifying_edge_count > 0);
        assert!(field.diagnostics.discarded_short_segment_count > 0);
        assert!(field.segments.is_empty());
        assert_eq!(field.diagnostics.arc_cell_count, 0);
        assert_eq!(field.diagnostics.peak_count, 0);
    }

    #[test]
    fn rejects_invalid_configuration_and_inputs() {
        let (mesh, plates, crust, boundaries) = fixture(512);
        for (config, error) in [
            (
                VolcanicArcFieldConfig {
                    minimum_boundary_edges: 0,
                    ..Default::default()
                },
                VolcanicArcFieldError::EmptyMinimumSegment,
            ),
            (
                VolcanicArcFieldConfig {
                    inland_offset_cells: 0,
                    ..Default::default()
                },
                VolcanicArcFieldError::ZeroInlandOffset,
            ),
            (
                VolcanicArcFieldConfig {
                    peak_density_divisor: 0,
                    ..Default::default()
                },
                VolcanicArcFieldError::ZeroPeakDensityDivisor,
            ),
            (
                VolcanicArcFieldConfig {
                    strength_saturation: f32::NAN,
                    ..Default::default()
                },
                VolcanicArcFieldError::InvalidStrengthSaturation,
            ),
        ] {
            assert_eq!(
                derive_volcanic_arc_field(&mesh, &plates, &crust, &boundaries, config),
                Err(error)
            );
        }

        let mut invalid_crust = crust.clone();
        invalid_crust.plate_classes.pop();
        assert_eq!(
            derive_volcanic_arc_field(
                &mesh,
                &plates,
                &invalid_crust,
                &boundaries,
                VolcanicArcFieldConfig::default(),
            ),
            Err(VolcanicArcFieldError::Input(StageInputError::Plates))
        );

        let mut invalid_boundaries = boundaries.clone();
        invalid_boundaries.edge_classes.pop();
        assert_eq!(
            derive_volcanic_arc_field(
                &mesh,
                &plates,
                &crust,
                &invalid_boundaries,
                VolcanicArcFieldConfig::default(),
            ),
            Err(VolcanicArcFieldError::Input(StageInputError::Boundaries))
        );
    }
}
