@group(0) @binding(0) var<storage, read> terrain_control_texels: array<CubesphereFieldTexel>;
@group(0) @binding(1) var<storage, read> terrain_stamps: array<TerrainStamp>;
@group(0) @binding(2) var<storage, read> terrain_world: TerrainParameters;
struct TerrainTileJob {
    address: vec4<u32>,
    destination: vec4<u32>,
}
@group(0) @binding(3) var<storage, read> terrain_jobs: array<TerrainTileJob>;
@group(0) @binding(4) var<storage, read_write> terrain_addresses: array<vec4<u32>>;
@group(0) @binding(5) var<storage, read_write> terrain_samples: array<vec4<f32>>;

fn cubesphere_load_field_texel(index: u32) -> CubesphereFieldTexel {
    return terrain_control_texels[index];
}

fn terrain_load_stamp(index: u32) -> TerrainStamp {
    return terrain_stamps[index];
}

fn terrain_parameters() -> TerrainParameters {
    return terrain_world;
}

@compute @workgroup_size(64)
fn generate_terrain_tiles(@builtin(global_invocation_id) id: vec3<u32>) {
    let sample_count = arrayLength(&terrain_jobs) * CUBESPHERE_TILE_SAMPLE_COUNT;
    if id.x >= sample_count { return; }
    let job_index = id.x / CUBESPHERE_TILE_SAMPLE_COUNT;
    let job = terrain_jobs[job_index];
    let destination_slot = job.destination.x;
    let local_index = id.x % CUBESPHERE_TILE_SAMPLE_COUNT;
    let local = vec2(
        local_index % CUBESPHERE_TILE_VERTICES,
        local_index / CUBESPHERE_TILE_VERTICES,
    );
    let direction = cubesphere_tile_direction(job.address, local);
    let height = terrain_height_gpu(direction, cubesphere_tile_level(job.address));
    if local_index == 0u {
        terrain_addresses[destination_slot] = job.address;
    }
    let output_index = destination_slot * CUBESPHERE_TILE_SAMPLE_COUNT + local_index;
    terrain_samples[output_index] = vec4(height.value, height.derivative);
}
