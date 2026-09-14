struct Parameters { vertex_capacity: u32, index_capacity: u32, one: f32, zero: f32 }
struct Chunk { x: i32, y: i32, z: i32, spacing: i32, blocked: array<u32, MASK_WORDS> }
struct Vertex { anchor: vec4<i32>, offset: vec4<f32>, normal: vec4<f32> }
struct Block { total: vec2<u32>, offset: vec2<u32> }
struct Status { vertices: u32, indices: u32, overflow: u32, reserved: u32 }
struct Draw { indices: u32, instances: u32, first_index: u32, base_vertex: i32, first_instance: u32 }
@group(0) @binding(0) var<uniform> params: Parameters;
@group(0) @binding(1) var<storage, read> chunk: Chunk;
@group(0) @binding(2) var<storage, read> density: array<f32>;
@group(0) @binding(3) var<storage, read_write> scratch: array<vec2<u32>>;
@group(0) @binding(4) var<storage, read_write> blocks: array<Block>;
@group(0) @binding(5) var<storage, read_write> status: Status;
@group(0) @binding(6) var<storage, read_write> vertices: array<Vertex>;
@group(0) @binding(7) var<storage, read_write> indices: array<u32>;
@group(0) @binding(8) var<storage, read_write> draw: Draw;
const NONE: u32 = 4294967295u;

fn grid_point(id: u32, side: u32) -> vec3<i32> {
    return vec3<i32>(i32(id % side), i32((id / side) % side), i32(id / (side * side)));
}
fn node_id(p: vec3<i32>) -> u32 { return u32(p.x) + NODES * (u32(p.y) + NODES * u32(p.z)); }
fn corner(mask: u32) -> vec3<i32> { return vec3<i32>(i32(mask & 1u), i32((mask >> 1u) & 1u), i32((mask >> 2u) & 1u)); }
fn potential(p: vec3<i32>) -> f32 {
    let h = vec3<u32>(p + vec3<i32>(i32(HALO)));
    return density[h.x + SAMPLE_SIDE * (h.y + SAMPLE_SIDE * h.z)];
}
fn density_normal(p: vec3<i32>) -> vec3<f32> {
    return vec3<f32>(
        potential(p-vec3(1,0,0))-potential(p+vec3(1,0,0)),
        potential(p-vec3(0,1,0))-potential(p+vec3(0,1,0)),
        potential(p-vec3(0,0,1))-potential(p+vec3(0,0,1))
    ) / (2.0*f32(chunk.spacing));
}
fn in_chunk(p: vec3<i32>) -> bool { return all(p >= vec3<i32>(0)) && all(p <= vec3<i32>(i32(CELLS))); }
fn is_active(slot: u32) -> bool {
    let p = grid_point(slot / 8u, NODES);
    let a = potential(p);
    if slot % 8u == 7u {
        if a != 0.0 { return false; }
        for (var mask = 1u; mask < 8u; mask++) {
            let delta = corner(mask);
            if in_chunk(p + delta) { if potential(p + delta) < 0.0 { return true; } }
            if in_chunk(p - delta) { if potential(p - delta) < 0.0 { return true; } }
        }
        return false;
    }
    let q = p + corner(slot % 8u + 1u);
    if !in_chunk(q) { return false; }
    let b = potential(q);
    return (a < 0.0 && b > 0.0) || (a > 0.0 && b < 0.0);
}
fn root_slot(a: vec3<i32>, b: vec3<i32>, da: f32, db: f32) -> u32 {
    if da == 0.0 { return node_id(a) * 8u + 7u; }
    if db == 0.0 { return node_id(b) * 8u + 7u; }
    let delta = abs(a - b);
    return node_id(min(a, b)) * 8u + u32(delta.x + 2 * delta.y + 4 * delta.z) - 1u;
}
struct CellTriangles { roots: array<vec3<u32>, 12>, count: u32 }
fn cell_triangles(cell: u32) -> CellTriangles {
    if (chunk.blocked[cell / 32u] & (1u << (cell % 32u))) != 0u { return CellTriangles(); }
    let base = grid_point(cell, CELLS);
    var d: array<f32, 8>;
    for (var i = 0u; i < 8u; i++) { d[i] = potential(base + corner(i)); }
    var result: CellTriangles;
    for (var t = 0u; t < 6u; t++) {
        let tet = TETS[t];
        var mask = 0u;
        for (var i = 0u; i < 4u; i++) { if d[tet[i]] >= 0.0 { mask |= 1u << i; } }
        let ring = RINGS[t * 16u + mask];
        var roots: array<u32, 4>;
        var count = 0u;
        for (var i = 0u; i < 4u; i++) {
            let edge = ring[i];
            if edge == NONE { break; }
            let a = edge / 8u;
            let b = edge % 8u;
            let root = root_slot(base + corner(a), base + corner(b), d[a], d[b]);
            var duplicate = false;
            for (var j = 0u; j < count; j++) { duplicate = duplicate || roots[j] == root; }
            if !duplicate { roots[count] = root; count++; }
        }
        for (var i = 1u; i + 1u < count; i++) {
            result.roots[result.count] = vec3<u32>(roots[0], roots[i], roots[i + 1u]);
            result.count++;
        }
    }
    return result;
}
fn offset(slot: u32) -> vec2<u32> { return scratch[slot] + blocks[slot / GROUP_SIZE].offset; }
fn vertex(slot: u32) -> Vertex {
    let p = grid_point(slot / 8u, NODES);
    if slot % 8u == 7u { return Vertex(vec4<i32>(p * chunk.spacing, 0), vec4<f32>(0.0), vec4(density_normal(p),0.0)); }
    let q = p + corner(slot % 8u + 1u);
    let a = potential(p);
    let b = potential(q);
    let use_a = abs(a) <= abs(b);
    let start = select(q, p, use_a);
    let end = select(p, q, use_a);
    let near = select(b, a, use_a);
    let far = select(a, b, use_a);
    let ratio = abs(near / far);
    let math = F32Arithmetic(params.one, params.zero);
    let fraction = ratio / f32_add(1.0, ratio, math);
    let delta = fma(vec3<f32>((end - start) * chunk.spacing), vec3<f32>(fraction), vec3<f32>(params.zero));
    let a_normal = density_normal(start);
    let normal = f32_scale_add(density_normal(end)-a_normal, fraction, a_normal, math);
    return Vertex(vec4<i32>(start * chunk.spacing, 0), vec4<f32>(delta, 0.0), vec4(normal,0.0));
}

@compute @workgroup_size(GROUP_SIZE)
fn classify(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= SLOTS { return; }
    var count = 0u;
    if id.x < CELL_COUNT { count = cell_triangles(id.x).count * 3u; }
    scratch[id.x] = vec2<u32>(u32(is_active(id.x)), count);
}

var<workgroup> scan: array<vec2<u32>, GROUP_SIZE>;
@compute @workgroup_size(GROUP_SIZE)
fn scan_blocks(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) lane: u32, @builtin(workgroup_id) group: vec3<u32>) {
    var value = vec2<u32>(0u);
    if id.x < SLOTS { value = scratch[id.x]; }
    scan[lane] = value;
    workgroupBarrier();
    // Inclusive Hillis-Steele scan, with barriers around each read/write phase.
    for (var step = 1u; step < GROUP_SIZE; step *= 2u) {
        var previous = vec2<u32>(0u);
        if lane >= step { previous = scan[lane - step]; }
        workgroupBarrier();
        scan[lane] += previous;
        workgroupBarrier();
    }
    if id.x < SLOTS { scratch[id.x] = scan[lane] - value; }
    if lane == GROUP_SIZE - 1u { blocks[group.x].total = scan[lane]; }
}

@compute @workgroup_size(1)
fn scan_totals() {
    var total = vec2<u32>(0u);
    for (var i = 0u; i < BLOCKS; i++) {
        blocks[i].offset = total;
        total += blocks[i].total;
    }
    let overflow = total.x > params.vertex_capacity || total.y > params.index_capacity;
    status = Status(total.x, total.y, u32(overflow), 0u);
    draw = Draw(select(total.y, 0u, overflow), 1u, 0u, 0, 0u);
}

@compute @workgroup_size(GROUP_SIZE)
fn emit(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= SLOTS || status.overflow != 0u { return; }
    let output = offset(id.x);
    if is_active(id.x) { vertices[output.x] = vertex(id.x); }
    if id.x < CELL_COUNT {
        let cell = cell_triangles(id.x);
        for (var i = 0u; i < cell.count; i++) {
            let roots = cell.roots[i];
            indices[output.y + 3u * i] = offset(roots.x).x;
            indices[output.y + 3u * i + 1u] = offset(roots.y).x;
            indices[output.y + 3u * i + 2u] = offset(roots.z).x;
        }
    }
}
