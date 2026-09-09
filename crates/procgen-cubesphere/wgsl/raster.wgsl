// WGSL mirror of procgen-cubesphere's face-major raster addressing and adjacency.
// Include mapping.wgsl before this source.

const CUBESPHERE_AXIS_LINK_LENGTH: u32 = 5u;
const CUBESPHERE_DIAGONAL_LINK_LENGTH: u32 = 7u;
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
