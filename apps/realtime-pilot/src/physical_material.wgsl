// Surface material from slope and altitude, mirroring `material_color` in
// physical_color.rs. The constants above are generated from the same records.
// `slope_cos` is dot(normal, up): 1 is flat ground, 0 a vertical face.
fn material_color(slope_cos: f32, altitude_m: f32, height_limit_m: f32, sea_level_m: f32, ocean: bool) -> vec3<f32> {
    let rock = 1.0 - smoothstep(MATERIAL_ROCK_COS,MATERIAL_SOIL_COS,slope_cos);
    var color = mix(MATERIAL_SOIL,MATERIAL_ROCK,rock);
    // Snow needs both the altitude and a face gentle enough to hold it.
    let snow = smoothstep(MATERIAL_SNOW_START*height_limit_m,MATERIAL_SNOW_FULL*height_limit_m,altitude_m)
        * smoothstep(MATERIAL_SNOW_BARE_COS,MATERIAL_SNOW_SLOPE_COS,slope_cos);
    color = mix(color,MATERIAL_SNOW,snow);
    if ocean {
        let shore = 1.0 - smoothstep(MATERIAL_SHORE_M,MATERIAL_SHORE_FADE_M,altitude_m-sea_level_m);
        color = mix(color,MATERIAL_SAND,shore*(1.0-rock));
    }
    return color;
}
