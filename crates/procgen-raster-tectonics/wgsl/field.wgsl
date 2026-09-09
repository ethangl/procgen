// Packed-field accessors for the raster tectonics buffers.
//
// Compose procgen-cubesphere's mapping and raster mirrors and the constant
// preamble that `field_wgsl_source` emits before this source. The kernels and
// the applications that display their buffers both read the packing through
// these functions rather than restating it, and nothing here touches a
// binding.

/// One plate's identity, area, motion, and crust class.
struct RasterPlate {
    /// Cell the plate's seed was placed on, or `CUBESPHERE_NO_RASTER_CELL`.
    seed_cell: u32,
    /// Quantized surface area of the cells the plate owns.
    area: u32,
    /// `RASTER_CRUST_OCEANIC` or `RASTER_CRUST_CONTINENTAL`.
    crust: u32,
    /// Plate the crust classification considers at this position of its order.
    order: u32,
    /// Euler rotation vector, quantized by the host before upload.
    angular_velocity: vec3<f32>,
}

/// Relative motion across one border, from the first cell's point of view.
struct RasterBorderMotion {
    /// Each side's signed speed toward the other; their sum is the convergence.
    normal_speeds: vec2<f32>,
    /// Absolute relative speed parallel to the border.
    shear: f32,
}

fn raster_pack_label(cost: u32, plate: u32) -> u32 {
    return (cost << RASTER_PLATE_LABEL_BITS) | plate;
}

fn raster_label_cost(label: u32) -> u32 {
    return label >> RASTER_PLATE_LABEL_BITS;
}

fn raster_label_plate(label: u32) -> u32 {
    return label & RASTER_UNCLAIMED_PLATE;
}

fn raster_pack_ownership(current: u32, pending: u32) -> u32 {
    return (pending << RASTER_CELL_PLATE_BITS) | current;
}

fn raster_current_plate(ownership_word: u32) -> u32 {
    return ownership_word & RASTER_CELL_PLATE_MASK;
}

fn raster_pending_plate(ownership_word: u32) -> u32 {
    return ownership_word >> RASTER_CELL_PLATE_BITS;
}

fn raster_boundary_class(classes: u32, link: u32) -> u32 {
    return (classes >> (RASTER_BOUNDARY_CLASS_BITS * link)) & RASTER_BOUNDARY_CLASS_MASK;
}

fn raster_pack_boundary_class(classes: u32, link: u32, border: u32) -> u32 {
    return classes | (border << (RASTER_BOUNDARY_CLASS_BITS * link));
}

/// Quantized area of one cell, in the units the crust reduction sums. Rounding
/// to nearest keeps the sum unbiased and never fuses with a multiply.
fn raster_cell_area(cell: u32, resolution: u32) -> u32 {
    return u32(round(
        cubesphere_texel_solid_angle(cell, resolution) * RASTER_AREA_UNITS_PER_STERADIAN,
    ));
}

/// Instantaneous velocity of the point of a rigidly rotating plate that is at
/// `position` on the unit sphere.
fn raster_plate_velocity(plate: RasterPlate, position: vec3<f32>) -> vec3<f32> {
    return cross(plate.angular_velocity, position);
}

/// Relative motion across the border between two cells, evaluated at the
/// border's midpoint.
///
/// Both cells reach the same result: reversing the two sides negates the normal
/// and the tangent exactly, which swaps the normal speeds and leaves their sum
/// and the shear unchanged. That is what lets each cell classify all four of
/// its borders without resolving which cell owns the border edge.
fn raster_border_motion(
    direction: vec3<f32>,
    plate: RasterPlate,
    other_direction: vec3<f32>,
    other: RasterPlate,
) -> RasterBorderMotion {
    let midpoint = normalize(direction + other_direction);
    let normal = normalize(other_direction - direction);
    let tangent = normalize(cross(midpoint, normal));
    let velocity = raster_plate_velocity(plate, midpoint);
    let other_velocity = raster_plate_velocity(other, midpoint);
    return RasterBorderMotion(
        vec2(dot(velocity, normal), -dot(other_velocity, normal)),
        abs(dot(velocity - other_velocity, tangent)),
    );
}

/// Signed normal closing speed. Positive values converge; negative diverge.
fn raster_convergence(motion: RasterBorderMotion) -> f32 {
    return motion.normal_speeds.x + motion.normal_speeds.y;
}

fn raster_motion_class(motion: RasterBorderMotion) -> u32 {
    let convergence = raster_convergence(motion);
    if abs(convergence) > motion.shear * RASTER_CONVERGENCE_TO_SHEAR_THRESHOLD {
        return select(RASTER_BOUNDARY_DIVERGENT, RASTER_BOUNDARY_CONVERGENT, convergence > 0.0);
    }
    return RASTER_BOUNDARY_TRANSFORM;
}

/// Whether the first side of a convergent border advances into the second:
/// continental crust dominates, then the faster approach, then the lower plate
/// id. Mirrors the mesh pipeline's side-key comparison.
fn raster_side_advances(
    plate: u32,
    crust: u32,
    speed: f32,
    other_plate: u32,
    other_crust: u32,
    other_speed: f32,
) -> bool {
    if crust != other_crust {
        return crust > other_crust;
    }
    if speed != other_speed {
        return speed > other_speed;
    }
    return plate <= other_plate;
}

/// Whether a migration proposal beats the standing one: strongest convergence,
/// then the lower advancing plate id, then the lower border-edge id.
fn raster_proposal_precedes(
    convergence: f32,
    plate: u32,
    edge: u32,
    best_convergence: f32,
    best_plate: u32,
    best_edge: u32,
) -> bool {
    if convergence != best_convergence {
        return convergence > best_convergence;
    }
    if plate != best_plate {
        return plate < best_plate;
    }
    return edge < best_edge;
}
