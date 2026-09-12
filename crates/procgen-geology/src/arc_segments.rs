//! What one arc segment is, and how the overriding plate carries it.
//!
//! A segment is the stretch of one convergent boundary that has a polarity,
//! together with the belt of cells the arc sits on a fixed distance inland of
//! it and the peaks that belt carries. [`volcanic_arcs`] finds the boundaries
//! and groups them; this module turns one group into a segment.
//!
//! [`volcanic_arcs`]: crate::volcanic_arcs

use crate::field::position_in_cell;
use procgen_core::{RandomStream, Vec3};
use procgen_sphere_mesh::SphereMesh;
use procgen_tectonics::{CellCrust, CrustClass, PlatePartition};

/// One volcano of an arc. A cell carries as many as its own area asks for,
/// so each holds the position that separates it from the others in its cell
/// rather than being named by its cell alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolcanicArcPeak {
    pub cell: usize,
    /// Seeded surface position guaranteed to remain inside `cell`.
    pub position: Vec3,
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
    /// Peak candidates in ascending cell order. A cell appears once per peak
    /// it carries.
    pub peaks: Vec<VolcanicArcPeak>,
    /// Actual inland depth used, which may be shallower than the requested bound.
    pub inland_depth: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct InlandClaim {
    pub(crate) strength: f32,
    pub(crate) source_edge: usize,
}

/// How an arc's volcanoes are placed: how many the arc's area asks for, and
/// where inside a cell each one sits.
///
/// The three travel together because none of them means anything without the
/// others, and a segment reads all three or none.
#[derive(Clone, Copy)]
pub(crate) struct PeakPlacement {
    pub(crate) density: f32,
    pub(crate) positions: RandomStream,
    pub(crate) maximum_offset: f32,
}

pub(crate) struct BoundaryGroup {
    pub(crate) overriding_plate: usize,
    pub(crate) kind: ArcKind,
    pub(crate) boundary_edges: Vec<usize>,
    pub(crate) boundary_cells: Vec<usize>,
}

pub(crate) const fn arc_kind(class: CrustClass) -> ArcKind {
    match class {
        CrustClass::Continental => ArcKind::Continental,
        CrustClass::Oceanic => ArcKind::Island,
    }
}

pub(crate) fn derive_segment(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: CellCrust<'_>,
    boundary_claims: &[Option<InlandClaim>],
    placement: PeakPlacement,
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
    let peaks = select_peak_candidates(mesh, &arc_cells, arc_area, placement);
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

/// The strongest cells of an arc, one peak per unit of area the density asks
/// for.
///
/// A retained segment always keeps at least one peak, the same guarantee the
/// count-based rule gave: a segment long enough to survive the minimum-length
/// filter is an arc, and an arc has a volcano on it. Only that floor is
/// discrete; everything above it follows the arc's real area, so an arc of the
/// same width and length carries the same volcanoes on any mesh.
///
/// An arc whose cells are larger than the spacing the density asks for wants
/// more peaks than it has cells. Every cell then carries the whole number of
/// them it can, and the strongest carry the remainder: taking the strongest
/// cells alone capped the count at the arc's cell count and lost the density
/// on any mesh coarse enough, which at 16,384 cells was half the volcanoes
/// the default asks for. The count is exact rather than expected, because
/// nothing here is drawn — only where inside a cell a peak sits is.
fn select_peak_candidates(
    mesh: &SphereMesh,
    arc_cells: &[VolcanicArcCell],
    arc_area: f32,
    placement: PeakPlacement,
) -> Vec<VolcanicArcPeak> {
    let wanted = ((arc_area * placement.density).round() as usize).max(1);
    peak_counts_by_cell(arc_cells, wanted)
        .into_iter()
        .flat_map(|(cell, count)| (0..count).map(move |peak| (cell, peak)))
        .map(|(cell, peak)| VolcanicArcPeak {
            cell,
            position: position_in_cell(
                mesh,
                cell,
                placement.positions,
                peak,
                placement.maximum_offset,
            ),
        })
        .collect()
}

/// How many peaks each cell of an arc carries, in ascending cell order.
///
/// Whole peaks go to every cell and the strongest carry the remainder, so the
/// total is exactly `wanted` however few cells the arc has. Below one peak a
/// cell this is the strongest `wanted` cells and nothing else, which is the
/// rule as it stood; above it, taking the strongest cells alone capped the
/// count at the arc's cell count and lost the density.
fn peak_counts_by_cell(arc_cells: &[VolcanicArcCell], wanted: usize) -> Vec<(usize, usize)> {
    let mut ranked: Vec<_> = arc_cells.iter().collect();
    ranked.sort_unstable_by(|left, right| {
        right
            .strength
            .total_cmp(&left.strength)
            .then_with(|| left.cell.cmp(&right.cell))
    });
    let each = wanted / ranked.len();
    let remainder = wanted % ranked.len();
    let mut counts: Vec<_> = ranked
        .iter()
        .enumerate()
        .map(|(rank, arc_cell)| (arc_cell.cell, each + usize::from(rank < remainder)))
        .filter(|&(_, count)| count > 0)
        .collect();
    counts.sort_unstable_by_key(|&(cell, _)| cell);
    counts
}

pub(crate) fn claim_precedes(candidate: InlandClaim, existing: InlandClaim) -> bool {
    candidate.strength > existing.strength
        || (candidate.strength == existing.strength && candidate.source_edge < existing.source_edge)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arc_cells() -> Vec<VolcanicArcCell> {
        [(7, 0.2), (2, 0.9), (5, 0.5), (1, 0.5)]
            .map(|(cell, strength)| VolcanicArcCell { cell, strength })
            .to_vec()
    }

    /// One peak per unit of area the density asks for, taken strongest first,
    /// equal strengths broken by the lower cell id, and returned in cell
    /// order.
    #[test]
    fn peaks_are_the_strongest_cells_of_the_arc_in_cell_order() {
        let cells = arc_cells();

        // Two of the four: cell 2 at 0.9, then the lower of the pair at 0.5.
        assert_eq!(peak_counts_by_cell(&cells, 2), vec![(1, 1), (2, 1)]);
        assert_eq!(
            peak_counts_by_cell(&cells, 4),
            vec![(1, 1), (2, 1), (5, 1), (7, 1)]
        );
    }

    /// A segment long enough to be kept is an arc, and an arc has a volcano on
    /// it, so the count never falls to nothing.
    #[test]
    fn a_segment_keeps_its_strongest_cell_however_little_area_it_covers() {
        assert_eq!(peak_counts_by_cell(&arc_cells(), 1), vec![(2, 1)]);
    }

    /// An arc that wants more peaks than it has cells gives every cell the
    /// whole number it can and the remainder to the strongest. The rule that
    /// took the strongest cells alone stopped at four here however many the
    /// density asked for.
    #[test]
    fn an_arc_that_wants_more_peaks_than_cells_gives_every_cell_its_share() {
        let cells = arc_cells();
        // Two each, and the remaining three to the three strongest: cell 2 at
        // 0.9, then cells 1 and 5 at 0.5 with the lower id first.
        assert_eq!(
            peak_counts_by_cell(&cells, 11),
            vec![(1, 3), (2, 3), (5, 3), (7, 2)]
        );
        for wanted in 1..40 {
            let counts = peak_counts_by_cell(&cells, wanted);
            assert_eq!(
                counts.iter().map(|&(_, count)| count).sum::<usize>(),
                wanted,
                "{wanted} peaks were not all placed"
            );
            assert!(
                counts.windows(2).all(|pair| pair[0].0 < pair[1].0),
                "{wanted} peaks came back out of cell order"
            );
        }
    }
}
