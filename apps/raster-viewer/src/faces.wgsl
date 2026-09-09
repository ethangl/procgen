// Fragment shader for the raster viewer's cube-face grids.
// Compose procgen-cubesphere's mapping mirror and the constant preamble that
// render.rs emits before this source.

#import bevy_pbr::forward_io::VertexOutput

struct RasterFaceDisplay {
    resolution: u32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<storage, read> raster_growth_labels: array<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<uniform> raster_display: RasterFaceDisplay;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<storage, read> raster_plate_palette: array<vec4<f32>>;

/// Fixed key light, so the sphere reads as a sphere without a lighting rig.
const RASTER_LIGHT_DIRECTION = vec3<f32>(-0.4767, 0.5720, 0.6673);
const RASTER_AMBIENT: f32 = 0.35;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let direction = normalize(in.world_position.xyz);
    let face = cubesphere_dominant_face(direction);
    let coordinates = cubesphere_project_direction(direction, face);
    let resolution = raster_display.resolution;
    let limit = f32(resolution) - 1.0;
    let texel = clamp(
        floor((vec2(coordinates.u, coordinates.v) + 1.0) * 0.5 * f32(resolution)),
        vec2(0.0),
        vec2(limit),
    );
    let label = raster_growth_labels[
        cubesphere_texel_index(face, u32(texel.x), u32(texel.y), resolution)
    ];
    // The reserved plate id is all ones, so it doubles as the field's mask and
    // as the palette entry for a cell no plate reached.
    let color = raster_plate_palette[label & RASTER_PLATE_LABEL_MASK].rgb;
    let lambert = RASTER_AMBIENT
        + (1.0 - RASTER_AMBIENT)
            * max(dot(normalize(in.world_normal), RASTER_LIGHT_DIRECTION), 0.0);
    return vec4(color * lambert, 1.0);
}
