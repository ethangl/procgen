//! What a model length is worth in mesh hops.
//!
//! A stage that reaches a few cells out of a boundary is stating a distance,
//! not a hop count: the same configuration on a finer mesh should reach the
//! same distance over more cells. Configs therefore hold model lengths on the
//! unit sphere and convert once, at the top of the stage that walks the graph,
//! through [`hops`].

use std::f32::consts::PI;

/// The default mesh's cell count, which every length default in the pipeline
/// was set against. [`default_hop_length`] turns it into the length one hop
/// spans there, so a default written as a multiple of that converts back to
/// the hop count it replaced, exactly.
pub const DEFAULT_CELL_COUNT: usize = 65_536;

/// The one representative cell width of a sphere of `radius` covered by
/// `cell_count` cells: the side of a square with the mean cell area.
///
/// A sphere's area is known before its cells are, so this answers for a mesh
/// that does not exist yet: that is what lets the viewer bound a step duration
/// against a cell count the user has only typed.
pub fn mean_cell_width(radius: f32, cell_count: usize) -> f32 {
    (4.0 * PI * radius * radius / cell_count as f32).sqrt()
}

/// One hop on the default mesh, in model units on the unit sphere.
pub fn default_hop_length() -> f32 {
    mean_cell_width(1.0, DEFAULT_CELL_COUNT)
}

/// The hop count a model length spans on a mesh of `cell_count` cells.
///
/// Lengths are on the unit sphere, the same convention particle positions use,
/// so the mesh's radius does not enter and the cell count is the whole of what
/// the answer depends on. A positive length is never less than one hop: a
/// distance a stage was configured to reach must not vanish on a mesh too
/// coarse to resolve it. Zero is the one length that returns zero, because a
/// zero width is how a taper, a ramp, or a rim is switched off.
pub fn hops(cell_count: usize, length: f32) -> usize {
    if length <= 0.0 {
        return 0;
    }
    (length / mean_cell_width(1.0, cell_count)).round().max(1.0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_multiple_of_the_default_hop_length_is_that_many_hops_on_the_default_mesh() {
        for count in 1..=16 {
            assert_eq!(
                hops(DEFAULT_CELL_COUNT, count as f32 * default_hop_length()),
                count
            );
        }
        assert_eq!(hops(DEFAULT_CELL_COUNT, 0.0), 0);
    }

    #[test]
    fn a_finer_mesh_spends_more_hops_on_the_same_length_and_never_fewer_than_one() {
        // Four times the cells halves the cell width, so the same length
        // spans twice the hops.
        let length = 8.0 * default_hop_length();
        assert_eq!(hops(1_024, length), 1);
        assert_eq!(hops(4_096, length), 2);
        assert_eq!(hops(16_384, length), 4);
        // A length far below one cell width still reaches the first neighbor.
        assert_eq!(hops(1_024, 1.0e-4), 1);
    }
}
