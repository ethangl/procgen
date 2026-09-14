struct Parameters { capacity: u32, tetrahedra: u32, one: f32, zero: f32 }
struct Node { position: vec4<i32>, samples: array<u32,8>, count: u32, pad0: u32, pad1: u32, pad2: u32 }
struct Vertex { anchor: vec4<i32>, offset: vec4<f32>, normal: vec4<f32> }
struct Draw { indices: u32, instances: u32, first_index: u32, base_vertex: i32, first_instance: u32 }
@group(0) @binding(0) var<uniform> params: Parameters;
@group(0) @binding(1) var<storage,read> nodes: array<Node>;
@group(0) @binding(2) var<storage,read> tetrahedra: array<vec4<u32>>;
@group(0) @binding(3) var<storage,read> samples: array<f32>;
@group(0) @binding(4) var<storage,read_write> scratch: array<vec2<u32>>;
@group(0) @binding(5) var<storage,read_write> status: vec4<u32>;
@group(0) @binding(6) var<storage,read_write> vertices: array<Vertex>;
@group(0) @binding(7) var<storage,read_write> indices: array<u32>;
@group(0) @binding(8) var<storage,read_write> draw: Draw;
const NONE: u32 = 4294967295u;
fn value(id: u32) -> f32 {
    let node = nodes[id];
    var result = 0.0;
    let math = F32Arithmetic(params.one,params.zero);
    for (var i = 0u; i < node.count; i++) { result = f32_add(result, samples[node.samples[i]] / f32(node.count), math); }
    return result;
}
struct Polygon { roots: array<vec2<u32>,4>, count: u32 }
fn polygon(id: u32) -> Polygon {
    let tet = tetrahedra[id];
    var values: array<f32,4>; var mask = 0u;
    for (var i = 0u; i < 4u; i++) { values[i] = value(tet[i]); if values[i] >= 0.0 { mask |= 1u<<i; } }
    var result: Polygon;
    for (var i = 0u; i < 4u; i++) {
        let edge = RINGS[mask][i]; if edge == NONE { break; }
        let a = edge/4u; let b = edge%4u;
        var root = vec2<u32>(min(tet[a],tet[b]),max(tet[a],tet[b]));
        if values[a] == 0.0 { root = vec2<u32>(tet[a]); } else if values[b] == 0.0 { root = vec2<u32>(tet[b]); }
        var duplicate = false;
        for (var j = 0u; j < result.count; j++) { duplicate = duplicate || all(result.roots[j] == root); }
        if !duplicate { result.roots[result.count] = root; result.count++; }
    }
    return result;
}
fn vertex(root: vec2<u32>) -> Vertex {
    var a = nodes[root.x].position.xyz; var b = nodes[root.y].position.xyz;
    if root.x == root.y { return Vertex(vec4<i32>(a,0),vec4<f32>(0.0),vec4<f32>(0.0)); }
    var da = value(root.x); var db = value(root.y);
    let swap = a.x > b.x || (a.x == b.x && (a.y > b.y || (a.y == b.y && a.z > b.z)));
    if swap { let p = a; a = b; b = p; let d = da; da = db; db = d; }
    let use_a = abs(da) <= abs(db);
    let start = select(b,a,use_a); let end = select(a,b,use_a);
    let ratio = abs(select(db,da,use_a) / select(da,db,use_a));
    let fraction = ratio / f32_add(1.0,ratio,F32Arithmetic(params.one,params.zero));
    return Vertex(vec4<i32>(start,0),vec4<f32>(fma(vec3<f32>(end-start),vec3<f32>(fraction),vec3<f32>(params.zero)),0.0),vec4<f32>(0.0));
}
@compute @workgroup_size(GROUP_SIZE)
fn classify(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.tetrahedra { return; }
    let p = polygon(id.x);
    var count = 0u; if p.count >= 3u { count = (p.count-2u)*3u; }
    scratch[id.x] = vec2<u32>(count,0u);
}
var<workgroup> scan: array<u32,GROUP_SIZE>;
@compute @workgroup_size(GROUP_SIZE)
fn scan_blocks(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) lane: u32, @builtin(workgroup_id) group: vec3<u32>) {
    var count = 0u; if id.x < params.tetrahedra { count = scratch[id.x].x; }
    scan[lane] = count; workgroupBarrier();
    for (var step = 1u; step < GROUP_SIZE; step *= 2u) {
        var previous = 0u; if lane >= step { previous = scan[lane-step]; }
        workgroupBarrier(); scan[lane] += previous; workgroupBarrier();
    }
    if id.x < params.tetrahedra { scratch[id.x].y = scan[lane]-count; }
    if lane == GROUP_SIZE-1u { scratch[params.tetrahedra+group.x].x = scan[lane]; }
}
@compute @workgroup_size(1)
fn scan_totals() {
    let blocks = (params.tetrahedra+GROUP_SIZE-1u)/GROUP_SIZE;
    var total = 0u;
    for (var i = 0u; i < blocks; i++) { scratch[params.tetrahedra+i].y = total; total += scratch[params.tetrahedra+i].x; }
    let overflow = total > params.capacity;
    status = vec4<u32>(total,total,u32(overflow),0u);
    draw = Draw(select(total,0u,overflow),1u,0u,0,0u);
}
@compute @workgroup_size(GROUP_SIZE)
fn emit(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.tetrahedra || status.z != 0u { return; }
    let p = polygon(id.x);
    var output = scratch[id.x].y + scratch[params.tetrahedra+id.x/GROUP_SIZE].y;
    for (var i = 1u; i+1u < p.count; i++) {
        let roots = array<vec2<u32>,3>(p.roots[0],p.roots[i],p.roots[i+1u]);
        for (var j = 0u; j < 3u; j++) { vertices[output] = vertex(roots[j]); indices[output] = output; output++; }
    }
}
