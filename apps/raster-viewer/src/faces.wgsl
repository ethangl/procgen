// Fragment shader for the raster viewer's cube-face grids.
// Compose procgen-cubesphere's mapping mirror and the constant preamble that
// render.rs emits before this source.

#import bevy_pbr::forward_io::VertexOutput

struct RasterFaceDisplay {
    resolution: u32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<storage, read> raster_growth_labels: array<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<uniform> raster_display: RasterFaceDisplay;

/// Fixed key light, so the sphere reads as a sphere without a lighting rig.
const RASTER_LIGHT_DIRECTION = vec3<f32>(-0.4767, 0.5720, 0.6673);
const RASTER_AMBIENT: f32 = 0.35;
const RASTER_UNCLAIMED_COLOR = vec3<f32>(0.08, 0.08, 0.1);

fn raster_srgb_channel_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        return value / 12.92;
    }
    return pow((value + 0.055) / 1.055, 2.4);
}

fn raster_hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> vec3<f32> {
    let chroma = (1.0 - abs(2.0 * lightness - 1.0)) * saturation;
    let sector = hue * 6.0;
    let secondary = chroma * (1.0 - abs(sector % 2.0 - 1.0));
    var rgb = vec3(chroma, 0.0, secondary);
    if sector < 1.0 {
        rgb = vec3(chroma, secondary, 0.0);
    } else if sector < 2.0 {
        rgb = vec3(secondary, chroma, 0.0);
    } else if sector < 3.0 {
        rgb = vec3(0.0, chroma, secondary);
    } else if sector < 4.0 {
        rgb = vec3(0.0, secondary, chroma);
    } else if sector < 5.0 {
        rgb = vec3(secondary, 0.0, chroma);
    }
    return rgb + (lightness - chroma * 0.5);
}

/// The shared identity ramp, evaluated per fragment rather than uploaded.
fn raster_plate_color(label: u32) -> vec3<f32> {
    // The reserved plate id is all ones, so it doubles as the field's mask.
    let plate = label & RASTER_UNCLAIMED_PLATE;
    if plate == RASTER_UNCLAIMED_PLATE {
        return RASTER_UNCLAIMED_COLOR;
    }
    let srgb = raster_hsl_to_rgb(
        fract(f32(plate) * RASTER_PLATE_HUE_TURNS),
        RASTER_PLATE_SATURATION,
        RASTER_PLATE_LIGHTNESS,
    );
    return vec3(
        raster_srgb_channel_to_linear(srgb.r),
        raster_srgb_channel_to_linear(srgb.g),
        raster_srgb_channel_to_linear(srgb.b),
    );
}

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
    let lambert = RASTER_AMBIENT
        + (1.0 - RASTER_AMBIENT)
            * max(dot(normalize(in.world_normal), RASTER_LIGHT_DIRECTION), 0.0);
    return vec4(raster_plate_color(label) * lambert, 1.0);
}
