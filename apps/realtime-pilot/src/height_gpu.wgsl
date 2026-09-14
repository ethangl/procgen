// Reuses the physical field and cube-sphere mapping. Mesh payload stays on GPU.
struct HeightVertex { anchor: vec4<i32>, offset: vec4<f32>, normal: vec4<f32> }
@group(1) @binding(0) var<storage, read> height_tiles: array<vec4<u32>>;
@group(1) @binding(1) var<storage, read_write> height_vertices: array<HeightVertex>;
@group(1) @binding(2) var<uniform> height_filter: vec4<f32>;
fn height_footprint(direction: vec3<f32>) -> f32 {
    let delta = direction * pilot.radius_m-height_filter.xyz;
    return max(pilot_sqrt(pilot_length_squared(delta)+height_filter.w*height_filter.w)/HEIGHT_FILTER_DISTANCE_RATIO,HEIGHT_FILTER_MIN_M);
}
fn height_normal_sample(direction: vec3<f32>) -> f32 {
    let d = normalize(direction);
    return pilot_filtered_height(d,height_footprint(d));
}
fn height_normal(direction: vec3<f32>, height: f32, footprint: f32) -> vec3<f32> {
    let step = max(footprint,HEIGHT_NORMAL_MIN_STEP_M);
    let delta = step / pilot.radius_m;
    let dx = vec3(delta,0.0,0.0);
    let dy = vec3(0.0,delta,0.0);
    let dz = vec3(0.0,0.0,delta);
    let gradient = vec3(
        height_normal_sample(direction+dx)-height_normal_sample(direction-dx),
        height_normal_sample(direction+dy)-height_normal_sample(direction-dy),
        height_normal_sample(direction+dz)-height_normal_sample(direction-dz)
    ) / (2.0*step);
    let tangent = gradient-direction*dot(gradient,direction);
    return normalize(direction-tangent*(pilot.radius_m/(pilot.radius_m+height)));
}
@compute @workgroup_size(64)
fn height_mesh(@builtin(global_invocation_id) id: vec3<u32>) {
    let tile_index = id.x / HEIGHT_VERTEX_COUNT;
    if tile_index >= arrayLength(&height_tiles) { return; }
    let tile = height_tiles[tile_index];
    let vertex = id.x % HEIGHT_VERTEX_COUNT;
    var local = vec2(vertex % HEIGHT_SIDE, vertex / HEIGHT_SIDE);
    let skirt = vertex >= (HEIGHT_SIDE * HEIGHT_SIDE);
    if skirt {
        let edge_vertex = vertex - (HEIGHT_SIDE * HEIGHT_SIDE);
        let edge = edge_vertex / HEIGHT_SIDE;
        let along = edge_vertex % HEIGHT_SIDE;
        switch edge {
            case 0u: { local = vec2(0u, along); }
            case 1u: { local = vec2(HEIGHT_QUADS, along); }
            case 2u: { local = vec2(along, 0u); }
            default: { local = vec2(along, HEIGHT_QUADS); }
        }
    }
    let direction = cubesphere_tile_direction(tile, local * (CUBESPHERE_TILE_QUADS / HEIGHT_QUADS));
    let spacing = cubesphere_vertex_spacing(tile.y) * pilot.radius_m * f32(CUBESPHERE_TILE_QUADS / HEIGHT_QUADS);
    let footprint = height_footprint(direction);
    var height = pilot_filtered_height(direction, footprint);
    let normal = height_normal(direction,height,footprint);
    if skirt { height = -pilot.height_limit_m - spacing * 2.0; }
    // Split the base radius before adding height, preserving local relief even
    // on the large comparison planet. fma retains the radial product residual.
    let anchor = vec3<i32>(floor(direction * pilot.radius_m));
    let residual = fma(direction, vec3(pilot.radius_m), -vec3<f32>(anchor));
    height_vertices[id.x] = HeightVertex(vec4(anchor, i32(tile.y)), vec4(residual + direction * height, spacing), vec4(normal,0.0));
}
