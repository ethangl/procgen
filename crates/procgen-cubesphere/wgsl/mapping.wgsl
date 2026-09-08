// WGSL mirror of procgen-cubesphere's canonical mapping and tile addressing.

const CUBESPHERE_PI_OVER_FOUR: f32 = 0.7853981633974483;
const CUBESPHERE_TILE_QUADS: u32 = 64u;
const CUBESPHERE_TILE_VERTICES: u32 = 65u;
const CUBESPHERE_TILE_SAMPLE_COUNT: u32 = 4225u;

fn cubesphere_tile_level(address: vec4<u32>) -> u32 {
    return address.y;
}

fn cubesphere_vertex_spacing(level: u32) -> f32 {
    return 2.0 * CUBESPHERE_PI_OVER_FOUR / f32(CUBESPHERE_TILE_QUADS << level);
}

struct CubesphereFaceFrame {
    normal: vec3<f32>,
    u_axis: vec3<f32>,
    v_axis: vec3<f32>,
}

struct CubesphereFaceCoordinates {
    face: u32,
    u: f32,
    v: f32,
}

fn cubesphere_texel_index(face: u32, x: u32, y: u32, resolution: u32) -> u32 {
    return (face * resolution + y) * resolution + x;
}

fn cubesphere_face_frame(face: u32) -> CubesphereFaceFrame {
    switch face {
        case 0u: { return CubesphereFaceFrame(vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, -1.0), vec3(0.0, 1.0, 0.0)); }
        case 1u: { return CubesphereFaceFrame(vec3(-1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 0.0)); }
        case 2u: { return CubesphereFaceFrame(vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, -1.0)); }
        case 3u: { return CubesphereFaceFrame(vec3(0.0, -1.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0)); }
        case 4u: { return CubesphereFaceFrame(vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)); }
        default: { return CubesphereFaceFrame(vec3(0.0, 0.0, -1.0), vec3(-1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)); }
    }
}

fn cubesphere_face_for_normal(normal: vec3<f32>) -> u32 {
    if normal.x != 0.0 { return select(1u, 0u, normal.x > 0.0); }
    if normal.y != 0.0 { return select(3u, 2u, normal.y > 0.0); }
    return select(5u, 4u, normal.z > 0.0);
}

fn cubesphere_seam_neighbor(face: u32, edge: u32, along: u32, edge_index: u32) -> vec3<u32> {
    let source = cubesphere_face_frame(face);
    var neighbor_normal: vec3<f32>;
    var along_axis: vec3<f32>;
    switch edge {
        case 0u: {
            neighbor_normal = -source.u_axis;
            along_axis = source.v_axis;
        }
        case 1u: {
            neighbor_normal = source.u_axis;
            along_axis = source.v_axis;
        }
        case 2u: {
            neighbor_normal = -source.v_axis;
            along_axis = source.u_axis;
        }
        default: {
            neighbor_normal = source.v_axis;
            along_axis = source.u_axis;
        }
    }
    let neighbor_face = cubesphere_face_for_normal(neighbor_normal);
    let neighbor = cubesphere_face_frame(neighbor_face);
    let normal_on_u = abs(dot(source.normal, neighbor.u_axis)) > 0.5;
    let fixed_axis = select(neighbor.v_axis, neighbor.u_axis, normal_on_u);
    let running_axis = select(neighbor.u_axis, neighbor.v_axis, normal_on_u);
    let fixed = select(0u, edge_index, dot(source.normal, fixed_axis) > 0.5);
    let running = select(edge_index - along, along, dot(along_axis, running_axis) > 0.5);
    let coordinates = select(vec2(running, fixed), vec2(fixed, running), normal_on_u);
    return vec3(neighbor_face, coordinates);
}

fn cubesphere_unit_direction(coordinates: CubesphereFaceCoordinates) -> vec3<f32> {
    let frame = cubesphere_face_frame(coordinates.face);
    let a = cubesphere_equiangular_tangent(coordinates.u);
    let b = cubesphere_equiangular_tangent(coordinates.v);
    return normalize(frame.normal + frame.u_axis * a + frame.v_axis * b);
}

fn cubesphere_equiangular_tangent(coordinate: f32) -> f32 {
    let squared = coordinate * coordinate;
    var correction = -0.00516628;
    correction = correction * squared - 0.01221719;
    correction = correction * squared - 0.05330517;
    correction = correction * squared - 0.21459327;
    return coordinate * (1.0 + (1.0 - squared) * correction);
}

fn cubesphere_dominant_face(direction: vec3<f32>) -> u32 {
    if abs(direction.x) >= abs(direction.y) && abs(direction.x) >= abs(direction.z) {
        return select(1u, 0u, direction.x >= 0.0);
    }
    if abs(direction.y) >= abs(direction.z) {
        return select(3u, 2u, direction.y >= 0.0);
    }
    return select(5u, 4u, direction.z >= 0.0);
}

fn cubesphere_project_direction(direction: vec3<f32>, face: u32) -> CubesphereFaceCoordinates {
    let frame = cubesphere_face_frame(face);
    let depth = dot(direction, frame.normal);
    return CubesphereFaceCoordinates(
        face,
        atan(dot(direction, frame.u_axis) / depth) / CUBESPHERE_PI_OVER_FOUR,
        atan(dot(direction, frame.v_axis) / depth) / CUBESPHERE_PI_OVER_FOUR,
    );
}

fn cubesphere_tile_direction(address: vec4<u32>, local: vec2<u32>) -> vec3<f32> {
    let quads_per_axis = CUBESPHERE_TILE_QUADS << cubesphere_tile_level(address);
    let grid_x = address.z * CUBESPHERE_TILE_QUADS + local.x;
    let grid_y = address.w * CUBESPHERE_TILE_QUADS + local.y;
    let u = -1.0 + 2.0 * f32(grid_x) / f32(quads_per_axis);
    let v = -1.0 + 2.0 * f32(grid_y) / f32(quads_per_axis);
    return cubesphere_unit_direction(CubesphereFaceCoordinates(address.x, u, v));
}

fn cubesphere_tile_sample_index(slot: u32, local: vec2<u32>) -> u32 {
    return slot * CUBESPHERE_TILE_SAMPLE_COUNT
        + local.y * CUBESPHERE_TILE_VERTICES
        + local.x;
}
