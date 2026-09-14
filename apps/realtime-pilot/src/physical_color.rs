//! Viewer-owned elevation colors, shared by CPU packing, WGSL, and the legend.
/// Linear RGB and altitude as a fraction of the configured height limit.
#[derive(Clone, Copy)]
pub struct HeightColorStop {
    pub rgb: [f32; 3],
    pub relative_height: f32,
}
/// These colors describe elevation only, not water, vegetation, or snow.
pub const HEIGHT_COLORS: [HeightColorStop; 6] = [
    HeightColorStop {
        rgb: [0.055, 0.075, 0.20],
        relative_height: -1.0,
    },
    HeightColorStop {
        rgb: [0.10, 0.25, 0.32],
        relative_height: -0.25,
    },
    HeightColorStop {
        rgb: [0.20, 0.36, 0.22],
        relative_height: 0.0,
    },
    HeightColorStop {
        rgb: [0.48, 0.40, 0.22],
        relative_height: 0.25,
    },
    HeightColorStop {
        rgb: [0.58, 0.49, 0.40],
        relative_height: 0.60,
    },
    HeightColorStop {
        rgb: [0.90, 0.90, 0.85],
        relative_height: 1.0,
    },
];
pub fn height_color(relative_height: f32) -> [f32; 4] {
    let mut color = HEIGHT_COLORS[0].rgb;
    for pair in HEIGHT_COLORS.windows(2) {
        let [a, b] = [pair[0], pair[1]];
        let t = ((relative_height - a.relative_height) / (b.relative_height - a.relative_height))
            .clamp(0.0, 1.0);
        for (value, target) in color.iter_mut().zip(b.rgb) {
            *value += (target - *value) * t;
        }
    }
    [color[0], color[1], color[2], 1.0]
}
/// Generate the palette declaration from the same records used by CPU and UI.
pub fn height_color_shader() -> String {
    let colors = HEIGHT_COLORS
        .iter()
        .map(|c| {
            format!(
                "vec4<f32>({:?},{:?},{:?},{:?})",
                c.rgb[0], c.rgb[1], c.rgb[2], c.relative_height
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!(
        "const HEIGHT_COLORS = array<vec4<f32>, {}>({colors});\n{}",
        HEIGHT_COLORS.len(),
        include_str!("physical_color.wgsl")
            .replace("HEIGHT_COLOR_COUNT", &HEIGHT_COLORS.len().to_string())
    )
}
