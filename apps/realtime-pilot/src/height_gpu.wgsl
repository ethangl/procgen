// Reuses the physical field and cube-sphere mapping. Mesh payload stays on GPU.
struct HeightTile { address: vec4<u32>, coarse_edges: vec4<u32>, corner_spans: vec4<u32> }
struct HeightVertex { anchor: vec4<i32>, offset: vec4<f32>, normal: vec4<f32> }
@group(1) @binding(0) var<storage, read> height_tiles: array<HeightTile>;
@group(1) @binding(1) var<storage, read_write> height_vertices: array<HeightVertex>;
fn height_footprint(local: vec2<u32>, spans: vec4<u32>) -> f32 {
    let x = local.x; let y = local.y; let q = HEIGHT_QUADS;
    let numerator = spans.x*(q-x)*(q-y) + spans.y*x*(q-y) + spans.z*(q-x)*y + spans.w*x*y;
    let unit = cubesphere_vertex_spacing(HEIGHT_FINEST_LEVEL) * pilot.radius_m
        * f32(CUBESPHERE_TILE_QUADS / HEIGHT_QUADS) / HEIGHT_DETAIL_RATIO;
    return max(f32(numerator)*unit/f32(q*q), HEIGHT_FILTER_MIN_M);
}
fn height_normal_sample(direction: vec3<f32>, footprint: f32) -> f32 {
    return pilot_filtered_height(normalize(direction),footprint);
}
fn height_normal(direction: vec3<f32>, height: f32, footprint: f32) -> vec3<f32> {
    let step = max(footprint,HEIGHT_NORMAL_MIN_STEP_M);
    let delta = step / pilot.radius_m;
    let dx = vec3(delta,0.0,0.0);
    let dy = vec3(0.0,delta,0.0);
    let dz = vec3(0.0,0.0,delta);
    let gradient = vec3(
        height_normal_sample(direction+dx,footprint)-height_normal_sample(direction-dx,footprint),
        height_normal_sample(direction+dy,footprint)-height_normal_sample(direction-dy,footprint),
        height_normal_sample(direction+dz,footprint)-height_normal_sample(direction-dz,footprint)
    ) / (2.0*step);
    let tangent = gradient-direction*dot(gradient,direction);
    return normalize(direction-tangent*(pilot.radius_m/(pilot.radius_m+height)));
}
@compute @workgroup_size(64)
fn height_mesh(@builtin(global_invocation_id) id: vec3<u32>) {
    let tile_index = id.x / HEIGHT_VERTEX_COUNT;
    if tile_index >= arrayLength(&height_tiles) { return; }
    let tile = height_tiles[tile_index].address;
    let edges = height_tiles[tile_index].coarse_edges.x;
    let vertex = id.x % HEIGHT_VERTEX_COUNT;
    var local = vec2(vertex % HEIGHT_SIDE, vertex / HEIGHT_SIDE);
    if (local.x == 0u && (edges & HEIGHT_EDGE_LEFT) != 0u)
        || (local.x == HEIGHT_QUADS && (edges & HEIGHT_EDGE_RIGHT) != 0u) {
        local.y &= ~1u;
    }
    if (local.y == 0u && (edges & HEIGHT_EDGE_BOTTOM) != 0u)
        || (local.y == HEIGHT_QUADS && (edges & HEIGHT_EDGE_TOP) != 0u) {
        local.x &= ~1u;
    }
    let direction = cubesphere_tile_direction(tile, local * (CUBESPHERE_TILE_QUADS / HEIGHT_QUADS));
    let spacing = cubesphere_vertex_spacing(tile.y) * pilot.radius_m * f32(CUBESPHERE_TILE_QUADS / HEIGHT_QUADS);
    let footprint = height_footprint(local,height_tiles[tile_index].corner_spans);
    let height = pilot_filtered_height(direction, footprint);
    let surface_height = height;
    let normal = height_normal(direction,height,footprint);
    // Split the base radius before adding height, preserving local relief even
    // on the large comparison planet. fma retains the radial product residual.
    let anchor = vec3<i32>(floor(direction * pilot.radius_m));
    let residual = fma(direction, vec3(pilot.radius_m), -vec3<f32>(anchor));
    height_vertices[id.x] = HeightVertex(vec4(anchor, i32(tile.y)), vec4(residual + direction * height, spacing), vec4(normal,surface_height));
}
