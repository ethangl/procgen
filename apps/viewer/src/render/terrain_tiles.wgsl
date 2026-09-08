#import bevy_pbr::{
    mesh_bindings::mesh,
    mesh_functions,
    forward_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage, read> terrain_samples: array<vec4<f32>>;

struct TerrainDisplayParameters {
    relief_exaggeration: f32,
    surface_radius: f32,
    padding: vec2<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<uniform> terrain_display: TerrainDisplayParameters;

struct TerrainElevationPalette {
    stops: array<vec4<f32>, TERRAIN_ELEVATION_PALETTE_STOP_COUNT>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<uniform> terrain_palette: TerrainElevationPalette;

fn terrain_srgb_channel_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        return value / 12.92;
    }
    return pow((value + 0.055) / 1.055, 2.4);
}

fn terrain_srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    return vec3(
        terrain_srgb_channel_to_linear(color.r),
        terrain_srgb_channel_to_linear(color.g),
        terrain_srgb_channel_to_linear(color.b),
    );
}

fn terrain_elevation_color(height: f32) -> vec4<f32> {
    let last = TERRAIN_ELEVATION_PALETTE_STOP_COUNT - 1u;
    let value = clamp(height, terrain_palette.stops[0].w, terrain_palette.stops[last].w);
    for (var index = 0u; index < last; index++) {
        let low = terrain_palette.stops[index];
        let high = terrain_palette.stops[index + 1u];
        if value < high.w {
            let t = (value - low.w) / (high.w - low.w);
            let srgb = low.xyz + (high.xyz - low.xyz) * t;
            return vec4(terrain_srgb_to_linear(srgb), 1.0);
        }
    }
    return vec4(terrain_srgb_to_linear(terrain_palette.stops[last].xyz), 1.0);
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let tag = mesh[vertex.instance_index].tag;
    let slot = tag & 0x1ffu;
    let address = vec4(
        (tag >> 9u) & 0x7u,
        (tag >> 12u) & 0x7u,
        (tag >> 15u) & 0xfu,
        (tag >> 19u) & 0xfu,
    );
    let local = vec2<u32>(vertex.position.xy);
    let sample_index = cubesphere_tile_sample_index(slot, local);
    let sample = terrain_samples[sample_index];
    let direction = cubesphere_tile_direction(address, local);
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
