#import bevy_pbr::{
    mesh_bindings::mesh,
    mesh_functions,
    forward_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage, read> terrain_control_texels: array<TerrainControlTexel>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage, read> terrain_stamps: array<TerrainStamp>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<uniform> terrain_world: TerrainParameters;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var<storage, read> terrain_addresses: array<vec4<u32>>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var<storage, read> terrain_samples: array<vec4<f32>>;

struct TerrainDisplayParameters {
    relief_exaggeration: f32,
    surface_radius: f32,
    padding: vec2<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var<uniform> terrain_display: TerrainDisplayParameters;

fn terrain_load_control_texel(index: u32) -> TerrainControlTexel {
    return terrain_control_texels[index];
}

fn terrain_load_stamp(index: u32) -> TerrainStamp {
    return terrain_stamps[index];
}

fn terrain_parameters() -> TerrainParameters {
    return terrain_world;
}

fn terrain_elevation_color(height: f32) -> vec4<f32> {
    let deep = vec3(0.02, 0.08, 0.3);
    let shore = vec3(0.08, 0.65, 0.85);
    let lowland = vec3(0.16, 0.55, 0.18);
    let highland = vec3(0.55, 0.38, 0.16);
    let summit = vec3(0.96, 0.96, 0.94);
    if height <= 0.5 {
        return vec4(deep + (shore - deep) * clamp(height / 0.5, 0.0, 1.0), 1.0);
    }
    if height <= 0.75 {
        return vec4(lowland + (highland - lowland) * ((height - 0.5) / 0.25), 1.0);
    }
    return vec4(highland + (summit - highland) * clamp((height - 0.75) / 0.25, 0.0, 1.0), 1.0);
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let slot = mesh[vertex.instance_index].tag;
    let local = vec2<u32>(vertex.position.xy);
    let sample_index = slot * 4225u + local.y * 65u + local.x;
    let sample = terrain_samples[sample_index];
    let direction = terrain_tile_direction(terrain_addresses[slot], local);
    let radius = terrain_display.surface_radius
        + (sample.x - 0.5) * terrain_display.relief_exaggeration;
    let local_position = direction * radius;
    let local_normal = normalize(direction - sample.yzw * terrain_display.relief_exaggeration / radius);
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(local_position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(local_normal, vertex.instance_index);
    out.color = terrain_elevation_color(sample.x);
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex.instance_index,
        world_from_local[3],
    );
#endif
    return out;
}
