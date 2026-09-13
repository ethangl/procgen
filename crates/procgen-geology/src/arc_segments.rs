//! What one arc segment is, and how the overriding plate carries it.
//!
//! A segment is the stretch of one convergent boundary that has a polarity,
//! together with the belt of cells the arc sits on a fixed distance inland of
//! it and the peaks that belt carries. [`volcanic_arcs`] finds the boundaries
//! and groups them; this module turns one group into a segment.
//!
//! [`volcanic_arcs`]: crate::volcanic_arcs

use procgen_sphere_mesh::SphereMesh;
use procgen_tectonics::{CellCrust, CrustClass, PlatePartition};

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

#[derive(Clone, Copy)]
pub(crate) struct InlandClaim {
    pub(crate) strength: f32,
    pub(crate) source_edge: usize,
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
    peak_density: f32,
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
    let peaks = select_peak_candidates(&arc_cells, arc_area, peak_density);
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

pub(crate) fn claim_precedes(candidate: InlandClaim, existing: InlandClaim) -> bool {
    candidate.strength > existing.strength
        || (candidate.strength == existing.strength && candidate.source_edge < existing.source_edge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_sphere_mesh::default_cell_area;

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
        let four_cells = 4.0 * default_cell_area();
        let per_two_cells = 1.0 / (2.0 * default_cell_area());

        // Two of the four: cell 2 at 0.9, then the lower of the pair at 0.5.
        assert_eq!(
            select_peak_candidates(&cells, four_cells, per_two_cells),
            vec![1, 2]
        );
        assert_eq!(
            select_peak_candidates(&cells, four_cells, per_two_cells * 2.0),
            vec![1, 2, 5, 7]
        );
    }

    /// A segment long enough to be kept is an arc, and an arc has a volcano on
    /// it, so the count rounds to nearest but never to nothing.
    #[test]
    fn a_segment_keeps_its_strongest_cell_however_little_area_it_covers() {
        let cells = arc_cells();
        let density = 1.0 / (2.0 * default_cell_area());
        assert_eq!(
            select_peak_candidates(&cells, 0.5 * default_cell_area(), density),
            vec![2]
        );
    }
}
