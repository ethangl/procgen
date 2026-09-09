// Fragment shader for the raster viewer's cube-face grids.
// Compose procgen-cubesphere's mapping and raster mirrors, the raster
// tectonics field accessors, and the constant preamble that render.rs emits
// before this source.

#import bevy_pbr::forward_io::VertexOutput

struct RasterFaceDisplay {
    resolution: u32,
    layer: u32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<storage, read> raster_ownership: array<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<storage, read> raster_boundary_classes: array<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<storage, read> raster_plates: array<RasterPlate>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var<uniform> raster_display: RasterFaceDisplay;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var<storage, read> raster_palette: array<vec4<f32>>;

/// Fixed key light, so the sphere reads as a sphere without a lighting rig.
const RASTER_LIGHT_DIRECTION = vec3<f32>(-0.4767, 0.5720, 0.6673);
const RASTER_AMBIENT: f32 = 0.35;

/// The most active class among a cell's four borders, so a boundary reads as a
/// line one texel wide on both of the cells that share it.
fn raster_strongest_border(cell: u32) -> u32 {
    let classes = raster_boundary_classes[cell];
    var seen = 0u;
    for (var link = 0u; link < CUBESPHERE_BORDER_LINKS_PER_CELL; link++) {
        seen |= 1u << raster_boundary_class(classes, link);
    }
    if (seen & (1u << RASTER_BOUNDARY_CONVERGENT)) != 0u {
        return RASTER_BOUNDARY_CONVERGENT;
    }
    if (seen & (1u << RASTER_BOUNDARY_DIVERGENT)) != 0u {
        return RASTER_BOUNDARY_DIVERGENT;
    }
    if (seen & (1u << RASTER_BOUNDARY_TRANSFORM)) != 0u {
        return RASTER_BOUNDARY_TRANSFORM;
    }
    return RASTER_BOUNDARY_INTERIOR;
}

fn raster_layer_color(cell: u32, plate: u32) -> vec3<f32> {
    switch raster_display.layer {
        case RASTER_LAYER_CRUST: {
            return raster_palette[RASTER_CRUST_PALETTE_BASE + raster_plates[plate].crust].rgb;
        }
        case RASTER_LAYER_BOUNDARIES: {
            let border = raster_strongest_border(cell);
            if border == RASTER_BOUNDARY_INTERIOR {
                return raster_palette[plate].rgb * RASTER_INTERIOR_DIMMING;
            }
            return raster_palette[RASTER_BOUNDARY_PALETTE_BASE + border].rgb;
        }
        default: {
            return raster_palette[plate].rgb;
        }
    }
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
    let cell = cubesphere_texel_index(face, u32(texel.x), u32(texel.y), resolution);
    let color = raster_layer_color(cell, raster_current_plate(raster_ownership[cell]));
    let lambert = RASTER_AMBIENT
        + (1.0 - RASTER_AMBIENT)
            * max(dot(normalize(in.world_normal), RASTER_LIGHT_DIRECTION), 0.0);
    return vec4(color * lambert, 1.0);
}
