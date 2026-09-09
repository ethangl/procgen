//! How the raster's buffers pack their fields, and the record one plate
//! occupies.
//!
//! This is the Rust side of `wgsl/field.wgsl`: every constant and accessor here
//! has a mirror there, and the shader preamble
//! [`crate::field_wgsl_source`](crate::field_wgsl_source) emits is built from
//! exactly these definitions. Nothing here knows a stage or a device.

use procgen_cubesphere::TexelLink;
use procgen_tectonics::{BoundaryClass, CrustClass};

/// Radius of the sphere the raster's cells lie on.
///
/// Cell centers are unit directions, so plate motion is evaluated on the unit
/// sphere and every speed the pipeline reports is in model units per unit time
/// at that radius. Consumers that need a speed's scale read this rather than
/// assuming one.
pub const RASTER_SPHERE_RADIUS: f32 = 1.0;

/// Bits of a packed growth label reserved for the owning plate id.
///
/// A label is `(arrival cost << PLATE_LABEL_BITS) | plate`, so the numeric
/// order of the word is the lexicographic order of `(cost, plate)` and one
/// `atomicMin` settles growth ties on cost and then on the lower plate id.
pub const PLATE_LABEL_BITS: u32 = 9;
/// Plate ids the label field can hold, one of which is reserved.
pub const PLATE_ID_COUNT: u32 = 1 << PLATE_LABEL_BITS;
/// Plate id reserved to mean "no plate has reached this cell".
pub const UNCLAIMED_PLATE: u32 = PLATE_ID_COUNT - 1;
/// Largest plate count a partition can address, since one id is reserved.
pub const MAX_PLATE_COUNT: u32 = UNCLAIMED_PLATE;
/// The label of a cell no plate has reached.
pub const UNCLAIMED_LABEL: u32 = u32::MAX;
/// Largest arrival cost the packed label can carry.
pub const MAX_GROWTH_COST: u32 = UNCLAIMED_LABEL >> PLATE_LABEL_BITS;

/// Bits of a cell's ownership word reserved for its current plate. The pending
/// plate a migration step proposes occupies the bits above them.
pub const CELL_PLATE_BITS: u32 = 16;
/// Mask of one plate field inside a cell's ownership word.
pub const CELL_PLATE_MASK: u32 = (1 << CELL_PLATE_BITS) - 1;

const _: () = assert!(PLATE_ID_COUNT <= CELL_PLATE_MASK + 1);

/// Bits one border's class occupies in a cell's packed boundary word.
pub const BOUNDARY_CLASS_BITS: u32 = 2;
/// Mask of one border's class inside a cell's packed boundary word.
pub const BOUNDARY_CLASS_MASK: u32 = (1 << BOUNDARY_CLASS_BITS) - 1;

const _: () = assert!(
    BoundaryClass::ALL.len() as u32 == BOUNDARY_CLASS_MASK + 1,
    "the class field must hold exactly the boundary classes, so every code it \
     can carry decodes"
);

/// Area units one steradian is worth in the crust reduction.
///
/// Cell areas are summed with integer atomics, so the reduction is
/// order-independent by construction. The scale is the largest that keeps the
/// whole sphere inside a `u32` with room to spare, which leaves the coarsest
/// cell about sixty units at the pilot's default resolution.
pub const AREA_UNITS_PER_STERADIAN: u32 = 32_000_000;

const _: () = assert!(
    13 * AREA_UNITS_PER_STERADIAN as u64 <= u32::MAX as u64,
    "the whole sphere's area, under 13 steradians, must fit one u32 counter"
);

/// Packs an arrival cost and a plate into one growth label.
pub const fn growth_label(cost: u32, plate: u32) -> u32 {
    (cost << PLATE_LABEL_BITS) | plate
}

/// Returns the arrival cost a packed growth label carries.
pub const fn growth_label_cost(label: u32) -> u32 {
    label >> PLATE_LABEL_BITS
}

/// Returns the plate a packed growth label carries, or [`UNCLAIMED_PLATE`].
///
/// The reserved plate id is all ones, so it doubles as the field's mask.
pub const fn growth_label_plate(label: u32) -> u32 {
    label & UNCLAIMED_PLATE
}

/// Packs a cell's current and pending plate into one ownership word.
///
/// Migration is a simultaneous update, so a step reads every cell's current
/// plate while writing its own pending plate. Both live in one word because
/// the kernels are already at `wgpu`'s default storage-binding limit.
pub const fn plate_ownership(current: u32, pending: u32) -> u32 {
    (pending << CELL_PLATE_BITS) | current
}

/// Returns the plate a cell currently belongs to.
pub const fn current_plate(ownership: u32) -> u32 {
    ownership & CELL_PLATE_MASK
}

/// Returns the plate the last migration step proposed for a cell, which equals
/// its current plate when no proposal won.
pub const fn pending_plate(ownership: u32) -> u32 {
    ownership >> CELL_PLATE_BITS
}

/// Returns the code the boundary buffer stores for a class.
pub const fn boundary_class_code(class: BoundaryClass) -> u32 {
    class as u32
}

/// Returns the code the plate buffer stores for a crust class.
pub const fn crust_class_code(class: CrustClass) -> u32 {
    class as u32
}

/// Returns the class of one of a cell's borders from its packed boundary word.
///
/// Both cells of a border classify it independently from the same inputs and
/// reach the same class, so a cell's word answers for every border it touches
/// without resolving which cell owns the border edge.
///
/// # Panics
///
/// Panics unless `link` is a border link.
pub fn boundary_class(classes: u32, link: TexelLink) -> BoundaryClass {
    assert!(link.is_border(), "only border links carry a boundary class");
    let code = (classes >> (BOUNDARY_CLASS_BITS * link.index())) & BOUNDARY_CLASS_MASK;
    BoundaryClass::ALL[code as usize]
}

/// One plate's identity, area, motion, and crust class, as the plate buffer
/// stores it.
///
/// The host fills `order` and `angular_velocity` before a run; the kernels fill
/// `seed_cell`, `area`, and `crust`. `padding` completes the sixteen-byte
/// stride the trailing vector requires.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RasterPlate {
    /// Cell the plate's seed was placed on, or
    /// [`procgen_cubesphere::NO_RASTER_CELL`] when the head start left no
    /// eligible cell and the plate stayed empty.
    pub seed_cell: u32,
    /// Quantized surface area of the cells the plate owns, in the units
    /// [`AREA_UNITS_PER_STERADIAN`] defines.
    pub area: u32,
    /// [`crust_class_code`] of the plate's immutable crust class.
    pub crust: u32,
    /// Plate the crust classification considers at this position of its seeded
    /// order.
    pub order: u32,
    /// Euler rotation vector: direction is the rotation axis and magnitude is
    /// angular speed. Quantized by the host before upload.
    pub angular_velocity: [f32; 3],
    pub padding: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_labels_order_by_cost_then_plate() {
        let mut labels: Vec<u32> = [(8, 4), (7, 500), (8, 3), (7, 0)]
            .into_iter()
            .map(|(cost, plate)| growth_label(cost, plate))
            .collect();
        labels.sort_unstable();
        assert_eq!(
            labels
                .iter()
                .map(|&label| (growth_label_cost(label), growth_label_plate(label)))
                .collect::<Vec<_>>(),
            [(7, 0), (7, 500), (8, 3), (8, 4)]
        );
        assert_eq!(
            UNCLAIMED_LABEL,
            growth_label(MAX_GROWTH_COST, UNCLAIMED_PLATE),
            "the unclaimed label must be the largest label, so atomicMin settles it"
        );
    }

    #[test]
    fn ownership_carries_the_current_and_pending_plate_independently() {
        let ownership = plate_ownership(7, 300);
        assert_eq!(current_plate(ownership), 7);
        assert_eq!(pending_plate(ownership), 300);
        assert_eq!(
            pending_plate(plate_ownership(UNCLAIMED_PLATE, UNCLAIMED_PLATE)),
            UNCLAIMED_PLATE
        );
    }

    #[test]
    fn each_border_link_reads_back_its_own_class() {
        let mut classes = 0;
        for (index, link) in TexelLink::BORDERS.into_iter().enumerate() {
            classes |= boundary_class_code(BoundaryClass::ALL[index])
                << (BOUNDARY_CLASS_BITS * link.index());
        }
        for (index, link) in TexelLink::BORDERS.into_iter().enumerate() {
            assert_eq!(boundary_class(classes, link), BoundaryClass::ALL[index]);
        }
    }
}
