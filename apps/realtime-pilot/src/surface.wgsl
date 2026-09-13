#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#ifdef VISIBILITY_RANGE_DITHER
#import bevy_pbr::pbr_functions::visibility_range_dither
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> modes: vec4<u32>;

struct SurfaceBand {
    frequency: f32,
    slope: f32,
    roughness: f32,
    key: u32,
}
const SURFACE_BANDS = array<SurfaceBand, 2>(
    SurfaceBand(48.0, 0.045, 0.055, 0x53555246u),
    SurfaceBand(192.0, 0.025, 0.025, 0x47524149u),
);
struct SurfaceSample {
    slope: vec3<f32>,
    roughness_delta: f32,
}
fn surface_detail(position: vec3<f32>, footprint: f32) -> SurfaceSample {
    var result = SurfaceSample(vec3(0.0), 0.0);
    for (var i = 0u; i < 2u; i++) {
        let band = SURFACE_BANDS[i];
        // Remove a band before a lattice cell spans fewer than 2.5 pixels.
        let fade = 1.0 - smoothstep(0.12, 0.4, footprint * band.frequency);
        if fade > 0.0 {
            let sample = gradient_noise_3d(band.key, position * band.frequency);
            // Slope is an authored bump strength, independent of wavelength.
            result.slope += sample.derivative * (band.slope * fade);
            result.roughness_delta += clamp(sample.value * 2.0, -1.0, 1.0)
                * (band.roughness * fade);
        }
    }
    return result;
}

@fragment
fn fragment(vertex: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    // Streamed meshes use planet coordinates with identity transforms. No UV,
    // face, camera, or LOD input changes the pattern's phase or scale.
    let position = vertex.world_position.xyz;
    let dx = dpdx(position);
    let dy = dpdy(position);
    // Upper bound on the pixel footprint, including grazing/diagonal views.
    // All screen derivatives precede visibility dither and divergent branches.
    let footprint = sqrt(dot(dx, dx) + dot(dy, dy));
    // Screen Y points down: this cross product follows outward front-face winding.
    let triangle = normalize(cross(dy, dx));
    let bary = vec3(vertex.uv, 1.0 - vertex.uv.x - vertex.uv.y);
    let width = fwidth(bary);
    let edges = smoothstep(vec3(0.0), width * 1.2, bary);
    let interior = min(edges.x, min(edges.y, edges.z));
#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(vertex.position, vertex.visibility_range_dither);
#endif
    let averaged = vertex.world_normal;
    let density = vertex.color.xyz;
    var normal = averaged;
    if modes.x == 1u { normal = triangle; }
    if modes.x == 2u { normal = density; }
    let undefined = dot(normal, normal) < 0.000001;
    // Terrain has translation-only transforms. Density vectors are planet-space.
    var input = vertex;
    input.color = vec4(1.0);
    var pbr = pbr_input_from_standard_material(input, is_front);
    if !undefined {
        pbr.N = normalize(normal);
        pbr.world_normal = pbr.N;
        if modes.w == 1u && modes.y == 0u {
            let detail = surface_detail(position, footprint);
            let tangent = detail.slope - pbr.N * dot(detail.slope, pbr.N);
            // Limit the cosmetic normal tilt to atan(0.08), about 4.6 degrees.
            let slope = tangent / max(1.0, length(tangent) / 0.08);
            pbr.N = normalize(pbr.N - slope);
            pbr.material.perceptual_roughness += detail.roughness_delta;
        }
    }
    if modes.y == 1u {
        var color = vec3(0.16, 0.32, 0.6);
        if vertex.color.w > 0.5 { color = vec3(0.2, 0.55, 0.32); }
        if vertex.color.w > 1.5 { color = vec3(0.62, 0.44, 0.15); }
        pbr.material.base_color = vec4(color, 1.0);
    }
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr);
    if modes.y == 2u && !undefined {
        out.color = vec4(normalize(normal) * 0.5 + 0.5, 1.0);
    }
    if modes.y == 3u {
        let invalid = dot(averaged, averaged) < 0.000001 || dot(density, density) < 0.000001;
        if invalid {
            out.color = vec4(1.0, 0.0, 1.0, 1.0);
        } else {
            let agreement = dot(normalize(averaged), normalize(density));
            let aligned = vec3(0.04);
            let perpendicular = vec3(1.0, 0.75, 0.0);
            let opposed = vec3(1.0, 0.0, 0.0);
            var color = mix(perpendicular, aligned, max(agreement, 0.0));
            if agreement < 0.0 { color = mix(perpendicular, opposed, -agreement); }
            out.color = vec4(color, 1.0);
        }
    }
    if undefined { out.color = vec4(1.0, 0.0, 1.0, 1.0); }
    if modes.z != 0u { out.color = vec4(mix(vec3(0.015), out.color.rgb, interior), 1.0); }
    return FragmentOutput(main_pass_post_lighting_processing(pbr, out.color));
}
