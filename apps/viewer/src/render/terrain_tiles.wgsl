#import bevy_pbr::{
    mesh_bindings::mesh,
    mesh_functions,
    forward_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage, read> terrain_addresses: array<vec4<u32>>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage, read> terrain_samples: array<vec4<f32>>;

struct TerrainDisplayParameters {
    relief_exaggeration: f32,
    surface_radius: f32,
    padding: vec2<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<uniform> terrain_display: TerrainDisplayParameters;

struct TerrainElevationPalette {
    stops: array<vec4<f32>, 5>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var<uniform> terrain_palette: TerrainElevationPalette;

fn terrain_elevation_color(height: f32) -> vec4<f32> {
    let value = clamp(height, terrain_palette.stops[0].w, terrain_palette.stops[4].w);
    for (var index = 0u; index < 4u; index++) {
        let low = terrain_palette.stops[index];
        let high = terrain_palette.stops[index + 1u];
        if value < high.w {
            let t = (value - low.w) / (high.w - low.w);
            return vec4(low.xyz + (high.xyz - low.xyz) * t, 1.0);
        }
    }
    return vec4(terrain_palette.stops[4].xyz, 1.0);
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let slot = mesh[vertex.instance_index].tag;
    let local = vec2<u32>(vertex.position.xy);
    let sample_index = cubesphere_tile_sample_index(slot, local);
    let sample = terrain_samples[sample_index];
    let direction = cubesphere_tile_direction(terrain_addresses[slot], local);
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
