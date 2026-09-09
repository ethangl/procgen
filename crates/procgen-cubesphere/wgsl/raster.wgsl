// WGSL mirror of procgen-cubesphere's face-major raster addressing and adjacency.
// Include mapping.wgsl before this source.

const CUBESPHERE_AXIS_LINK_LENGTH: u32 = 5u;
const CUBESPHERE_DIAGONAL_LINK_LENGTH: u32 = 7u;
const CUBESPHERE_BORDER_LINKS_PER_CELL: u32 = 4u;
const CUBESPHERE_NO_RASTER_CELL: u32 = 0xffffffffu;

fn cubesphere_texel_link_offset(link: u32) -> vec2<i32> {
    switch link {
        case 0u: { return vec2(-1, 0); }
        case 1u: { return vec2(1, 0); }
        case 2u: { return vec2(0, -1); }
        case 3u: { return vec2(0, 1); }
        case 4u: { return vec2(-1, -1); }
        case 5u: { return vec2(1, -1); }
        case 6u: { return vec2(-1, 1); }
        default: { return vec2(1, 1); }
    }
}

fn cubesphere_texel_link_length(link: u32) -> u32 {
    return select(CUBESPHERE_DIAGONAL_LINK_LENGTH, CUBESPHERE_AXIS_LINK_LENGTH, link < 4u);
}

fn cubesphere_texel_neighbor(cell_id: u32, resolution: u32, link: u32) -> u32 {
    let face_size = resolution * resolution;
    let face = cell_id / face_size;
    let local = cell_id % face_size;
    let source_coordinates = vec2<i32>(i32(local % resolution), i32(local / resolution));
    let coordinates = source_coordinates + cubesphere_texel_link_offset(link);
    let raster_resolution = i32(resolution);
    let x_outside = coordinates.x < 0 || coordinates.x >= raster_resolution;
    let y_outside = coordinates.y < 0 || coordinates.y >= raster_resolution;

    if !x_outside && !y_outside {
        return cubesphere_texel_index(face, u32(coordinates.x), u32(coordinates.y), resolution);
    }
    if x_outside && y_outside {
        return CUBESPHERE_NO_RASTER_CELL;
    }

    var edge: u32;
    var along: u32;
    if x_outside {
        edge = select(1u, 0u, coordinates.x < 0);
        along = u32(coordinates.y);
    } else {
        edge = select(3u, 2u, coordinates.y < 0);
        along = u32(coordinates.x);
    }
    let neighbor = cubesphere_seam_neighbor(face, edge, along, resolution - 1u);
    return cubesphere_texel_index(
        neighbor.x,
        neighbor.y,
        neighbor.z,
        resolution,
    );
}

fn cubesphere_texel_center(index: u32, resolution: u32) -> f32 {
    return -1.0 + f32(2u * index + 1u) / f32(resolution);
}

fn cubesphere_texel_direction(cell_id: u32, resolution: u32) -> vec3<f32> {
    let face_size = resolution * resolution;
    let face = cell_id / face_size;
    let local = cell_id % face_size;
    return cubesphere_unit_direction(CubesphereFaceCoordinates(
        face,
        cubesphere_texel_center(local % resolution, resolution),
        cubesphere_texel_center(local / resolution, resolution),
    ));
}

/// Canonical id of the border edge `link` crosses, identical from either cell.
///
/// The lower cell id owns the border and contributes its own direction toward
/// the other cell. Seams may rotate which direction that is, so the owner's
/// link is searched for rather than assumed opposite.
fn cubesphere_border_edge(cell_id: u32, resolution: u32, link: u32) -> u32 {
    let neighbor = cubesphere_texel_neighbor(cell_id, resolution, link);
    if cell_id < neighbor {
        return CUBESPHERE_BORDER_LINKS_PER_CELL * cell_id + link;
    }
    var back = 0u;
    for (var candidate = 1u; candidate < CUBESPHERE_BORDER_LINKS_PER_CELL; candidate++) {
        if cubesphere_texel_neighbor(neighbor, resolution, candidate) == cell_id {
            back = candidate;
        }
    }
    return CUBESPHERE_BORDER_LINKS_PER_CELL * neighbor + back;
}

/// Solid angle a texel covers, in steradians.
fn cubesphere_texel_solid_angle(cell_id: u32, resolution: u32) -> f32 {
    let local = cell_id % (resolution * resolution);
    let a = cubesphere_equiangular_tangent(
        cubesphere_texel_center(local % resolution, resolution),
    );
    let b = cubesphere_equiangular_tangent(
        cubesphere_texel_center(local / resolution, resolution),
    );
    let width = 2.0 * CUBESPHERE_PI_OVER_FOUR / f32(resolution);
    let squared = 1.0 + a * a + b * b;
    return width * width * (1.0 + a * a) * (1.0 + b * b) / (squared * sqrt(squared));
}
