//! Viewer-owned elevation colors, shared by the WGSL shaders and the legend.
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
/// Generate the palette declaration from the same records the legend draws.
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

/// Surface material thresholds. Slope is the cosine between the surface normal
/// and the local up direction, so 1 is flat ground and 0 a vertical face, and a
/// steeper face always has the smaller cosine. Angles are stored in degrees
/// because that is how they are reasoned about; both consumers compare cosines.
/// Every colour and threshold here is a starting value, not a measured one.
pub struct MaterialRule;
impl MaterialRule {
    pub const SOIL: [f32; 3] = [0.22, 0.30, 0.15];
    pub const ROCK: [f32; 3] = [0.42, 0.39, 0.35];
    pub const SNOW: [f32; 3] = [0.92, 0.93, 0.95];
    pub const SAND: [f32; 3] = [0.66, 0.60, 0.44];
    /// Gentler than this is soil; steeper than `ROCK_DEGREES` is rock.
    pub const SOIL_DEGREES: f32 = 28.0;
    pub const ROCK_DEGREES: f32 = 42.0;
    /// Snow holds below `SNOW_SLOPE_DEGREES` and is gone above `SNOW_BARE_DEGREES`,
    /// so cliffs stay rock through the snow line.
    pub const SNOW_SLOPE_DEGREES: f32 = 35.0;
    pub const SNOW_BARE_DEGREES: f32 = 50.0;
    /// Snow band as a fraction of the configured height limit.
    pub const SNOW_START: f32 = 0.55;
    pub const SNOW_FULL: f32 = 0.65;
    /// Sand is full to `SHORE_M` above sea level and gone by `SHORE_FADE_M`.
    pub const SHORE_M: f32 = 4.0;
    pub const SHORE_FADE_M: f32 = 8.0;
}
/// The f32 cosine both consumers compare against, from one expression.
fn cosine(degrees: f32) -> f32 {
    degrees.to_radians().cos()
}
fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
/// Exact at both ends, unlike `a + (b - a) * t`, so a fully resolved material
/// is its own constant and the unit tests can compare for equality.
fn blend(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] * (1.0 - t) + b[i] * t)
}
/// Material colour for one surface point, mirrored by `material_color` in WGSL.
///
/// `slope_cos` is `dot(normal, up)`, `altitude_m` is measured above the
/// reference radius, and `sea_level_m` is `None` when the ocean is disabled.
/// The shore band has no lower bound: ground just under the waterline reads as
/// sand too, which is what a beach looks like through shallow water.
pub fn material_color(
    slope_cos: f32,
    altitude_m: f32,
    height_limit_m: f32,
    sea_level_m: Option<f32>,
) -> [f32; 3] {
    let rock = 1.0
        - smoothstep(
            cosine(MaterialRule::ROCK_DEGREES),
            cosine(MaterialRule::SOIL_DEGREES),
            slope_cos,
        );
    let mut color = blend(MaterialRule::SOIL, MaterialRule::ROCK, rock);
    let snow = smoothstep(
        MaterialRule::SNOW_START * height_limit_m,
        MaterialRule::SNOW_FULL * height_limit_m,
        altitude_m,
    ) * smoothstep(
        cosine(MaterialRule::SNOW_BARE_DEGREES),
        cosine(MaterialRule::SNOW_SLOPE_DEGREES),
        slope_cos,
    );
    color = blend(color, MaterialRule::SNOW, snow);
    if let Some(sea_level_m) = sea_level_m {
        let shore = 1.0
            - smoothstep(
                MaterialRule::SHORE_M,
                MaterialRule::SHORE_FADE_M,
                altitude_m - sea_level_m,
            );
        color = blend(color, MaterialRule::SAND, shore * (1.0 - rock));
    }
    color
}
/// Generate the material declaration from the same constants the legend reads.
pub fn material_color_shader() -> String {
    let vector = |rgb: [f32; 3]| format!("vec3<f32>({:?},{:?},{:?})", rgb[0], rgb[1], rgb[2]);
    format!(
        "const MATERIAL_SOIL = {};\n\
         const MATERIAL_ROCK = {};\n\
         const MATERIAL_SNOW = {};\n\
         const MATERIAL_SAND = {};\n\
         const MATERIAL_SOIL_COS = {:?};\n\
         const MATERIAL_ROCK_COS = {:?};\n\
         const MATERIAL_SNOW_SLOPE_COS = {:?};\n\
         const MATERIAL_SNOW_BARE_COS = {:?};\n\
         const MATERIAL_SNOW_START = {:?};\n\
         const MATERIAL_SNOW_FULL = {:?};\n\
         const MATERIAL_SHORE_M = {:?};\n\
         const MATERIAL_SHORE_FADE_M = {:?};\n{}",
        vector(MaterialRule::SOIL),
        vector(MaterialRule::ROCK),
        vector(MaterialRule::SNOW),
        vector(MaterialRule::SAND),
        cosine(MaterialRule::SOIL_DEGREES),
        cosine(MaterialRule::ROCK_DEGREES),
        cosine(MaterialRule::SNOW_SLOPE_DEGREES),
        cosine(MaterialRule::SNOW_BARE_DEGREES),
        MaterialRule::SNOW_START,
        MaterialRule::SNOW_FULL,
        MaterialRule::SHORE_M,
        MaterialRule::SHORE_FADE_M,
        include_str!("physical_material.wgsl")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    const LIMIT: f32 = 3000.0;
    fn cos_degrees(degrees: f32) -> f32 {
        cosine(degrees)
    }
    #[test]
    fn resolved_materials_are_their_own_constants() {
        assert_eq!(material_color(1.0, 0.0, LIMIT, None), MaterialRule::SOIL);
        assert_eq!(material_color(0.0, 0.0, LIMIT, None), MaterialRule::ROCK);
        assert_eq!(
            material_color(1.0, 0.9 * LIMIT, LIMIT, None),
            MaterialRule::SNOW
        );
        // A steep face stays rock through the snow line.
        assert_eq!(
            material_color(cos_degrees(70.0), 0.9 * LIMIT, LIMIT, None),
            MaterialRule::ROCK
        );
    }
    #[test]
    fn the_shore_band_needs_the_ocean() {
        assert_eq!(
            material_color(1.0, 2.0, LIMIT, Some(0.0)),
            MaterialRule::SAND
        );
        assert_eq!(material_color(1.0, 2.0, LIMIT, None), MaterialRule::SOIL);
    }
    #[test]
    fn every_component_stays_inside_the_unit_range() {
        for slope in 0..=64 {
            let slope_cos = slope as f32 / 64.0;
            for altitude in -64..=64 {
                let altitude_m = altitude as f32 / 64.0 * LIMIT;
                for sea_level_m in [None, Some(0.0), Some(-250.0)] {
                    let color = material_color(slope_cos, altitude_m, LIMIT, sea_level_m);
                    assert!(
                        color.iter().all(|c| (0.0..=1.0).contains(c)),
                        "{color:?} at {slope_cos} {altitude_m} {sea_level_m:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn rock_never_recedes_as_the_face_steepens() {
        // Soil and rock differ in red, and nothing else is mixed in here, so
        // the red channel recovers the rock weight directly.
        let weight = |slope_cos: f32| {
            let color = material_color(slope_cos, 0.0, LIMIT, None);
            (color[0] - MaterialRule::SOIL[0]) / (MaterialRule::ROCK[0] - MaterialRule::SOIL[0])
        };
        let mut previous = weight(1.0);
        for step in 0..=256 {
            let degrees = MaterialRule::SOIL_DEGREES
                + (MaterialRule::ROCK_DEGREES - MaterialRule::SOIL_DEGREES) * step as f32 / 256.0;
            let current = weight(cos_degrees(degrees));
            assert!(current >= previous, "{degrees}: {current} < {previous}");
            previous = current;
        }
        assert!(previous >= 1.0 - f32::EPSILON);
    }
}
