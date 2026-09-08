// WGSL mirror of procgen-cubesphere's canonical mapping and tile addressing.

const CUBESPHERE_PI_OVER_FOUR: f32 = 0.7853981633974483;
const CUBESPHERE_TILE_QUADS: u32 = 64u;
const CUBESPHERE_TILE_VERTICES: u32 = 65u;
const CUBESPHERE_TILE_SAMPLE_COUNT: u32 = 4225u;

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

fn cubesphere_unit_direction(coordinates: CubesphereFaceCoordinates) -> vec3<f32> {
    let frame = cubesphere_face_frame(coordinates.face);
    let a = tan(coordinates.u * CUBESPHERE_PI_OVER_FOUR);
    let b = tan(coordinates.v * CUBESPHERE_PI_OVER_FOUR);
    return normalize(frame.normal + frame.u_axis * a + frame.v_axis * b);
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
    let quads_per_axis = CUBESPHERE_TILE_QUADS << address.y;
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
