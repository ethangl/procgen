fn height_color(relative_height: f32) -> vec3<f32> {
    var color = HEIGHT_COLORS[0].xyz;
    for (var i = 1u; i < HEIGHT_COLOR_COUNT; i++) {
        let a = HEIGHT_COLORS[i-1u];
        let b = HEIGHT_COLORS[i];
        let t = clamp((relative_height-a.w)/(b.w-a.w),0.0,1.0);
        color = mix(color,b.xyz,t);
    }
    return color;
}
