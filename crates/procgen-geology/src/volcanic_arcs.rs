use crate::field::{GeologyInputError, MaxWinsField};
use procgen_sphere_mesh::{
    SphereMesh, connected_components, default_cell_area, default_hop_length, hops,
};
use procgen_tectonics::{
    BoundaryClass, BoundaryClassification, CellCrust, CrustClass, PlatePartition, StageInputError,
};
use std::{cmp::Ordering, fmt};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolcanicArcFieldConfig {
    /// Minimum length of qualifying boundary required to retain a segment, as
    /// a model length on the unit sphere. A boundary's edges are its length in
    /// hops, to within the mesh's irregularity, so the stage converts this to
    /// an edge count once against the mesh. The default of three default hops
    /// is about 265 km at Earth radius.
    pub minimum_boundary_length: f32,
    /// Desired distance from the boundary on the overriding plate, as a model
    /// length on the unit sphere. The default of two default hops is about
    /// 180 km at Earth radius, which is where a volcanic front sits behind a
    /// trench.
    pub inland_offset: f32,
    /// Peak candidates per unit area of arc on the unit sphere, retaining the
    /// strongest candidates first. The default is one peak per two cells of
    /// the default mesh, which is a volcano every 120 km or so of arc at Earth
    /// radius.
    pub peak_density: f32,
    /// Convergence at which diagnostic strength reaches one.
    pub strength_saturation: f32,
}

impl Default for VolcanicArcFieldConfig {
    fn default() -> Self {
        Self {
            minimum_boundary_length: 3.0 * default_hop_length(),
            inland_offset: 2.0 * default_hop_length(),
            peak_density: 1.0 / (2.0 * default_cell_area()),
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

/// What the overriding plate carries the arc on. A continental arc is the
/// Andes and an island arc is the Marianas: the same construction, over a
/// continent or over ocean floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArcKind {
    Continental,
    Island,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VolcanicArcSegment {
    pub overriding_plate: usize,
    /// Crust the overriding cells carry, which every cell of a segment
    /// shares because grouping and the inland walk both stay on one class.
    pub kind: ArcKind,
    /// Qualifying mesh edge ids in ascending order.
    pub boundary_edges: Vec<usize>,
    /// Overriding boundary cells in ascending order.
    pub boundary_cells: Vec<usize>,
    /// Inland cells in ascending order.
    pub arc_cells: Vec<VolcanicArcCell>,
    /// Peak candidate cells in ascending order.
    pub peaks: Vec<usize>,
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
    /// Segments of [`ArcKind::Island`], which the rest of the counts include.
    pub island_segment_count: usize,
    /// Arc cells of those segments, which `arc_cell_count` includes.
    pub island_arc_cell_count: usize,
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

impl VolcanicArcField {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), GeologyInputError> {
        if self.cell_strengths.len() != mesh.cell_count()
            || self.cell_segments.len() != mesh.cell_count()
            || self
                .cell_segments
                .iter()
                .flatten()
                .any(|&segment| segment >= self.segments.len())
            || self.segments.iter().any(|segment| {
                segment
                    .boundary_edges
                    .iter()
                    .any(|&edge| edge >= mesh.edge_count())
                    || segment
                        .boundary_cells
                        .iter()
                        .any(|&cell| cell >= mesh.cell_count())
                    || segment
                        .arc_cells
                        .iter()
                        .any(|arc| arc.cell >= mesh.cell_count())
                    || segment.peaks.iter().any(|&cell| cell >= mesh.cell_count())
            })
        {
            return Err(GeologyInputError::VolcanicArcs);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolcanicArcFieldError {
    Input(StageInputError),
    EmptyMinimumSegment,
    ZeroInlandOffset,
    InvalidPeakDensity,
    InvalidStrengthSaturation,
}

impl fmt::Display for VolcanicArcFieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => error.fmt(formatter),
            Self::EmptyMinimumSegment => {
                formatter.write_str("minimum boundary length must be a finite positive length")
            }
            Self::ZeroInlandOffset => {
                formatter.write_str("inland offset must be a finite positive length")
            }
            Self::InvalidPeakDensity => {
                formatter.write_str("peak density must be a finite positive density per unit area")
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
    kind: ArcKind,
    boundary_edges: Vec<usize>,
    boundary_cells: Vec<usize>,
}

/// Derives present-day volcanic-arc fields from the final convergent
/// boundaries that have a polarity. This operation does not read or modify
/// elevation.
pub fn derive_volcanic_arc_field(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: CellCrust<'_>,
    boundaries: &BoundaryClassification,
    config: VolcanicArcFieldConfig,
) -> Result<VolcanicArcField, VolcanicArcFieldError> {
    validate_inputs(mesh, plates, crust, boundaries, config)?;

    // The one conversion: the configured offset is a model length, and the
    // inland walk counts cells.
    let inland_offset = hops(mesh.cell_count(), config.inland_offset);
    let boundary = collect_boundary_data(mesh, crust, boundaries, config);
    let boundary_cell_count = boundary.claims.iter().flatten().count();
    let mut groups = group_boundaries(mesh, plates, crust, &boundary);
    let original_group_count = groups.len();
    let minimum_boundary_edges = hops(mesh.cell_count(), config.minimum_boundary_length);
    groups.retain(|group| group.boundary_edges.len() >= minimum_boundary_edges);
    let discarded_short_segment_count = original_group_count - groups.len();

    let mut discarded_landlocked_segment_count = 0;
    let mut segments: Vec<_> = groups
        .into_iter()
        .filter_map(|group| {
            let segment = derive_segment(
                mesh,
                plates,
                crust,
                &boundary.claims,
                config,
                inland_offset,
                group,
            );
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
    let islands = segments
        .iter()
        .filter(|segment| segment.kind == ArcKind::Island);
    let island_segment_count = islands.clone().count();
    let island_arc_cell_count = islands.map(|segment| segment.arc_cells.len()).sum();
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
            island_segment_count,
            island_arc_cell_count,
            affected_cell_count,
            overlap_cell_count,
            peak_count,
        },
    })
}

fn validate_inputs(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: CellCrust<'_>,
    boundaries: &BoundaryClassification,
    config: VolcanicArcFieldConfig,
) -> Result<(), VolcanicArcFieldError> {
    if !config.minimum_boundary_length.is_finite() || config.minimum_boundary_length <= 0.0 {
        return Err(VolcanicArcFieldError::EmptyMinimumSegment);
    }
    if !config.inland_offset.is_finite() || config.inland_offset <= 0.0 {
        return Err(VolcanicArcFieldError::ZeroInlandOffset);
    }
    if !config.peak_density.is_finite() || config.peak_density <= 0.0 {
        return Err(VolcanicArcFieldError::InvalidPeakDensity);
    }
    if !config.strength_saturation.is_finite() || config.strength_saturation <= 0.0 {
        return Err(VolcanicArcFieldError::InvalidStrengthSaturation);
    }
    plates.validate(mesh)?;
    crust.validate(mesh)?;
    boundaries.validate(mesh)?;
    Ok(())
}

/// Claims every convergent boundary edge that has a polarity for its
/// overriding cell, which is the one [`material_order`] ranks greater: the
/// plate above the slab is where an arc is built. `Equal` has no polarity and
/// is skipped, which leaves continental collisions arc-free as they are on
/// Earth and skips two floors of one age.
///
/// [`material_order`]: procgen_tectonics::material_order
fn collect_boundary_data(
    mesh: &SphereMesh,
    crust: CellCrust<'_>,
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
        let overriding_cell = match crust.order(edge.cells[0], edge.cells[1]) {
            Ordering::Equal => continue,
            Ordering::Greater => edge.cells[0],
            Ordering::Less => edge.cells[1],
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

/// Groups the claimed boundary cells into segments of one overriding plate and
/// one crust class. A segment has one [`ArcKind`] because the class is what
/// the inland walk stays on, so an arc that started over ocean floor never
/// continues onto a continent.
fn group_boundaries(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: CellCrust<'_>,
    boundary: &BoundaryData,
) -> Vec<BoundaryGroup> {
    connected_components(
        mesh,
        |cell| boundary.claims[cell].is_some(),
        |cell, neighbor| {
            plates.cell_plates[cell] == plates.cell_plates[neighbor]
                && crust.class(cell) == crust.class(neighbor)
        },
    )
    .into_iter()
    .map(|mut boundary_cells| {
        let overriding_plate = plates.cell_plates[boundary_cells[0]];
        let kind = arc_kind(crust.class(boundary_cells[0]));
        let mut boundary_edges: Vec<_> = boundary_cells
            .iter()
            .flat_map(|&cell| &boundary.edges_by_cell[cell])
            .copied()
            .collect();
        boundary_cells.sort_unstable();
        boundary_edges.sort_unstable();
        boundary_edges.dedup();
        BoundaryGroup {
            overriding_plate,
            kind,
            boundary_edges,
            boundary_cells,
        }
    })
    .collect()
}

const fn arc_kind(class: CrustClass) -> ArcKind {
    match class {
        CrustClass::Continental => ArcKind::Continental,
        CrustClass::Oceanic => ArcKind::Island,
    }
}

fn derive_segment(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: CellCrust<'_>,
    boundary_claims: &[Option<InlandClaim>],
    config: VolcanicArcFieldConfig,
    inland_offset: usize,
    group: BoundaryGroup,
) -> Option<VolcanicArcSegment> {
    let (arc_cells, inland_depth) =
        walk_inland(mesh, plates, crust, boundary_claims, &group, inland_offset)?;
    // A peak count is a density over the arc's real area, so an arc of the
    // same width and length carries the same volcanoes on any mesh.
    let arc_area = arc_cells
        .iter()
        .map(|arc_cell| mesh.unit_cell_area(arc_cell.cell))
        .sum::<f32>();
    let peaks = select_peak_candidates(&arc_cells, arc_area, config.peak_density);
    Some(VolcanicArcSegment {
        overriding_plate: group.overriding_plate,
        kind: group.kind,
        boundary_edges: group.boundary_edges,
        boundary_cells: group.boundary_cells,
        arc_cells,
        peaks,
        inland_depth,
    })
}

/// Steps the strongest claim inland cell by cell, over the overriding plate's
/// own crust of the segment's class: an arc sits on the plate the trench is
/// eating under, so the walk stops at that plate's boundary, and it sits on
/// one kind of crust, so a continental arc stops at the coast and an island
/// arc stops where the plate's floor meets its own continent.
fn walk_inland(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: CellCrust<'_>,
    boundary_claims: &[Option<InlandClaim>],
    group: &BoundaryGroup,
    maximum_depth: usize,
) -> Option<(Vec<VolcanicArcCell>, usize)> {
    let BoundaryGroup {
        overriding_plate,
        kind,
        boundary_cells,
        ..
    } = group;
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
                if visited[neighbor]
                    || plates.cell_plates[neighbor] != *overriding_plate
                    || arc_kind(crust.class(neighbor)) != *kind
                {
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

/// The strongest cells of an arc, one per unit of area the density asks for.
///
/// A retained segment always keeps at least one peak, the same guarantee the
/// count-based rule gave: a segment long enough to survive the minimum-length
/// filter is an arc, and an arc has a volcano on it. Only that floor is
/// discrete; everything above it follows the arc's real area, so an arc of the
/// same width and length carries the same volcanoes on any mesh.
fn select_peak_candidates(
    arc_cells: &[VolcanicArcCell],
    arc_area: f32,
    peak_density: f32,
) -> Vec<usize> {
    let peak_count = ((arc_area * peak_density).round() as usize).max(1);
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
        .map(|arc_cell| arc_cell.cell)
        .collect()
}

fn claim_precedes(candidate: InlandClaim, existing: InlandClaim) -> bool {
    candidate.strength > existing.strength
        || (candidate.strength == existing.strength && candidate.source_edge < existing.source_edge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{PlateFixture, crust, hemisphere_fixture, plate_birth};
    use procgen_core::fingerprint;
    use procgen_sphere_mesh::{DEFAULT_CELL_COUNT, hop_length, mean_cell_area};
    use std::collections::BTreeSet;

    /// The arc fixtures split into more plates than the hotspot ones, so that
    /// a mesh this size carries several boundaries with a polarity.
    fn fixture(cell_count: usize) -> PlateFixture {
        PlateFixture::new(cell_count, 8)
    }

    /// The default config with its lengths and its peak density carried to a
    /// mesh of `cell_count` cells, so the walk reaches the two cells and the
    /// filter the three edges that the defaults mean, rather than the one a
    /// mesh this coarse would round each to.
    fn reference_config(cell_count: usize) -> VolcanicArcFieldConfig {
        VolcanicArcFieldConfig {
            minimum_boundary_length: hop_length(cell_count, 3.0),
            inland_offset: hop_length(cell_count, 2.0),
            peak_density: 1.0 / (2.0 * mean_cell_area(cell_count)),
            ..VolcanicArcFieldConfig::default()
        }
    }

    /// The default is a model length now, and this is the assertion that it
    /// still means the two cells it was written as.
    #[test]
    fn the_default_inland_offset_is_two_cells_of_the_default_mesh() {
        assert_eq!(
            hops(
                DEFAULT_CELL_COUNT,
                VolcanicArcFieldConfig::default().inland_offset
            ),
            2
        );
    }

    #[test]
    fn field_is_deterministic_ordered_and_bounded_inland() {
        let world = fixture(1_024);
        let boundaries = world.boundaries();
        let PlateFixture {
            mesh,
            plates,
            cell_birth,
            ..
        } = world;
        let config = reference_config(mesh.cell_count());
        let field =
            derive_volcanic_arc_field(&mesh, &plates, crust(&cell_birth), &boundaries, config)
                .unwrap();

        assert_eq!(
            field,
            derive_volcanic_arc_field(&mesh, &plates, crust(&cell_birth), &boundaries, config)
                .unwrap()
        );
        assert!(!field.segments.is_empty());
        // Every oceanic cell of this fixture is one age, so no ocean-ocean
        // edge has a polarity and every segment stands on a continent.
        assert!(
            field
                .segments
                .iter()
                .all(|segment| segment.kind == ArcKind::Continental)
        );
        assert_eq!(field.diagnostics.island_segment_count, 0);
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
            assert!(segment.peaks.windows(2).all(|pair| pair[0] < pair[1]));
            assert!(
                (1..=hops(mesh.cell_count(), config.inland_offset)).contains(&segment.inland_depth)
            );
            assert!(segment.arc_cells.iter().all(|arc_cell| {
                plates.cell_plates[arc_cell.cell] == segment.overriding_plate
                    && arc_kind(crust(&cell_birth).class(arc_cell.cell)) == segment.kind
                    && !segment.boundary_cells.contains(&arc_cell.cell)
            }));
            for &edge_index in &segment.boundary_edges {
                let edge = mesh.edges[edge_index];
                let order = crust(&cell_birth).order(edge.cells[0], edge.cells[1]);
                let overriding_cell = match order {
                    Ordering::Equal => panic!("an edge with no polarity claims no cell"),
                    Ordering::Greater => edge.cells[0],
                    Ordering::Less => edge.cells[1],
                };
                assert_eq!(
                    boundaries.edge_classes[edge_index],
                    BoundaryClass::Convergent
                );
                assert_eq!(
                    plates.cell_plates[overriding_cell],
                    segment.overriding_plate
                );
                assert!(segment.boundary_cells.contains(&overriding_cell));
            }
        }
    }

    /// Grouping follows the cell, not the plate: with crust that ignores the
    /// partition, one plate overrides along part of a boundary and is
    /// overridden along another part of it.
    #[test]
    fn mixed_crust_plates_group_by_their_continental_side() {
        let world = fixture(1_024);
        let boundaries = world.boundaries();
        let PlateFixture { mesh, plates, .. } = world;
        // Every segment kept, so the roles below are the grouping's own answer
        // rather than what survived the length filter.
        let config = VolcanicArcFieldConfig {
            minimum_boundary_length: hop_length(mesh.cell_count(), 1.0),
            ..reference_config(mesh.cell_count())
        };
        let cell_birth: Vec<_> = mesh
            .cell_centers
            .iter()
            .map(|center| (center.z < 0.0).then_some(0.0))
            .collect();

        let field =
            derive_volcanic_arc_field(&mesh, &plates, crust(&cell_birth), &boundaries, config)
                .unwrap();

        assert!(!field.segments.is_empty());
        let mut overriding = BTreeSet::new();
        for segment in &field.segments {
            assert_eq!(segment.kind, ArcKind::Continental);
            overriding.insert(segment.overriding_plate);
            for &cell in &segment.boundary_cells {
                assert_eq!(crust(&cell_birth).class(cell), CrustClass::Continental);
                assert_eq!(plates.cell_plates[cell], segment.overriding_plate);
            }
            for arc_cell in &segment.arc_cells {
                assert_eq!(
                    crust(&cell_birth).class(arc_cell.cell),
                    CrustClass::Continental,
                    "an arc left the continent it belongs to"
                );
            }
        }

        let mut overridden = BTreeSet::new();
        for (edge_index, edge) in mesh.edges.iter().enumerate() {
            if boundaries.edge_classes[edge_index] != BoundaryClass::Convergent {
                continue;
            }
            let classes = edge.cells.map(|cell| crust(&cell_birth).class(cell));
            if classes[0] == classes[1] {
                continue;
            }
            let oceanic = usize::from(classes[0] == CrustClass::Continental);
            overridden.insert(plates.cell_plates[edge.cells[oceanic]]);
        }
        assert!(
            overriding.intersection(&overridden).next().is_some(),
            "no plate both overrides and is overridden"
        );
    }

    /// Ocean-ocean convergence with a polarity is the Marianas: the younger
    /// floor overrides, so the arc is a chain of islands on it and the older
    /// plate carries only the trench.
    #[test]
    fn ocean_ocean_convergence_builds_an_island_arc_on_the_younger_plate() {
        let (mesh, plates, boundaries) = hemisphere_fixture(1_024);
        let cell_birth = plate_birth(&plates, [Some(0.5), Some(0.1)]);
        let config = reference_config(mesh.cell_count());
        let field =
            derive_volcanic_arc_field(&mesh, &plates, crust(&cell_birth), &boundaries, config)
                .unwrap();

        let [segment] = &field.segments[..] else {
            panic!("one boundary with one polarity is one segment");
        };
        assert_eq!(segment.kind, ArcKind::Island);
        assert_eq!(segment.overriding_plate, 0);
        assert!(
            (1..=hops(mesh.cell_count(), config.inland_offset)).contains(&segment.inland_depth)
        );
        assert_eq!(field.diagnostics.island_segment_count, 1);
        assert_eq!(
            field.diagnostics.island_arc_cell_count,
            field.diagnostics.arc_cell_count
        );
        for &cell in &segment.boundary_cells {
            assert_eq!(plates.cell_plates[cell], 0);
        }
        for arc_cell in &segment.arc_cells {
            assert_eq!(plates.cell_plates[arc_cell.cell], 0);
            assert_eq!(
                crust(&cell_birth).class(arc_cell.cell),
                CrustClass::Oceanic,
                "an island arc stands on ocean floor"
            );
        }
        assert!(
            (0..mesh.cell_count())
                .filter(|&cell| plates.cell_plates[cell] == 1)
                .all(
                    |cell| field.cell_strengths[cell] == 0.0 && field.cell_segments[cell].is_none()
                ),
            "the subducting plate carries no arc"
        );
    }

    /// The same fixture, with a continent above the same subducting floor: the
    /// Andean pair, resolved exactly as it was before island arcs existed.
    #[test]
    fn a_continent_over_the_same_floor_builds_a_continental_arc() {
        let (mesh, plates, boundaries) = hemisphere_fixture(1_024);
        let cell_birth = plate_birth(&plates, [None, Some(0.1)]);
        let field = derive_volcanic_arc_field(
            &mesh,
            &plates,
            crust(&cell_birth),
            &boundaries,
            reference_config(mesh.cell_count()),
        )
        .unwrap();

        let [segment] = &field.segments[..] else {
            panic!("one boundary with one polarity is one segment");
        };
        assert_eq!(segment.kind, ArcKind::Continental);
        assert_eq!(segment.overriding_plate, 0);
        assert_eq!(field.diagnostics.island_segment_count, 0);
        assert_eq!(field.diagnostics.island_arc_cell_count, 0);
        assert!(field.diagnostics.arc_cell_count > 0);
    }

    /// Convergence without a polarity builds nothing: two continents collide
    /// into a belt with no arc over it, as Earth's do, and two floors of one
    /// age have no younger side to put an arc on.
    #[test]
    fn convergence_without_a_polarity_produces_an_empty_field() {
        let (mesh, plates, boundaries) = hemisphere_fixture(1_024);
        for births in [[None, None], [Some(0.1), Some(0.1)]] {
            let cell_birth = plate_birth(&plates, births);
            let field = derive_volcanic_arc_field(
                &mesh,
                &plates,
                crust(&cell_birth),
                &boundaries,
                reference_config(mesh.cell_count()),
            )
            .unwrap();

            assert!(field.segments.is_empty());
            assert!(field.cell_strengths.iter().all(|&strength| strength == 0.0));
            assert!(field.cell_segments.iter().all(Option::is_none));
            assert_eq!(field.diagnostics, VolcanicArcDiagnostics::default());
        }
    }

    #[test]
    fn reference_field_has_stable_fingerprint() {
        let world = fixture(1_024);
        let boundaries = world.boundaries();
        let PlateFixture {
            mesh,
            plates,
            cell_birth,
            ..
        } = world;
        let field = derive_volcanic_arc_field(
            &mesh,
            &plates,
            crust(&cell_birth),
            &boundaries,
            reference_config(mesh.cell_count()),
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
            .chain(
                segment
                    .arc_cells
                    .iter()
                    .map(|arc_cell| arc_cell.cell as u64),
            )
            .chain(segment.peaks.iter().map(|&peak| peak as u64))
        });

        assert_eq!(fingerprint(values), 14_285_577_073_894_833_531);
    }

    #[test]
    fn peaks_and_overlaps_follow_stable_strength_rules() {
        let world = fixture(1_024);
        let boundaries = world.boundaries();
        let PlateFixture {
            mesh,
            plates,
            cell_birth,
            ..
        } = world;
        let config = VolcanicArcFieldConfig {
            minimum_boundary_length: hop_length(1_024, 1.0),
            inland_offset: hop_length(1_024, 3.0),
            peak_density: 1.0 / (2.0 * mean_cell_area(1_024)),
            strength_saturation: 2.0,
        };
        let field =
            derive_volcanic_arc_field(&mesh, &plates, crust(&cell_birth), &boundaries, config)
                .unwrap();

        assert!(field.diagnostics.overlap_cell_count > 0);
        for segment in &field.segments {
            let mut expected: Vec<_> = segment.arc_cells.iter().collect();
            expected.sort_unstable_by(|left, right| {
                right
                    .strength
                    .total_cmp(&left.strength)
                    .then_with(|| left.cell.cmp(&right.cell))
            });
            let arc_area = segment
                .arc_cells
                .iter()
                .map(|arc_cell| mesh.unit_cell_area(arc_cell.cell))
                .sum::<f32>();
            expected.truncate(((arc_area * config.peak_density).round() as usize).max(1));
            let mut expected: Vec<_> = expected.into_iter().map(|arc_cell| arc_cell.cell).collect();
            expected.sort_unstable();
            assert_eq!(segment.peaks, expected);
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
    fn minimum_edge_filter_reports_discarded_segments() {
        let world = fixture(512);
        let boundaries = world.boundaries();
        let PlateFixture {
            mesh,
            plates,
            cell_birth,
            ..
        } = world;
        let field = derive_volcanic_arc_field(
            &mesh,
            &plates,
            crust(&cell_birth),
            &boundaries,
            VolcanicArcFieldConfig {
                // Longer than any boundary a sphere can carry.
                minimum_boundary_length: 1.0e3,
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
        let world = fixture(512);
        let boundaries = world.boundaries();
        let PlateFixture {
            mesh,
            plates,
            cell_birth,
            ..
        } = world;
        for (config, error) in [
            (
                VolcanicArcFieldConfig {
                    minimum_boundary_length: 0.0,
                    ..Default::default()
                },
                VolcanicArcFieldError::EmptyMinimumSegment,
            ),
            (
                VolcanicArcFieldConfig {
                    inland_offset: 0.0,
                    ..Default::default()
                },
                VolcanicArcFieldError::ZeroInlandOffset,
            ),
            (
                VolcanicArcFieldConfig {
                    peak_density: 0.0,
                    ..Default::default()
                },
                VolcanicArcFieldError::InvalidPeakDensity,
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
                derive_volcanic_arc_field(&mesh, &plates, crust(&cell_birth), &boundaries, config),
                Err(error)
            );
        }

        assert_eq!(
            derive_volcanic_arc_field(
                &mesh,
                &plates,
                crust(&cell_birth[1..]),
                &boundaries,
                VolcanicArcFieldConfig::default(),
            ),
            Err(VolcanicArcFieldError::Input(StageInputError::CrustBirth))
        );

        let mut invalid_boundaries = boundaries.clone();
        invalid_boundaries.edge_classes.pop();
        assert_eq!(
            derive_volcanic_arc_field(
                &mesh,
                &plates,
                crust(&cell_birth),
                &invalid_boundaries,
                VolcanicArcFieldConfig::default(),
            ),
            Err(VolcanicArcFieldError::Input(StageInputError::Boundaries))
        );
    }
}
