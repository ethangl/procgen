use bevy::prelude::Color;

/// Hue step, in degrees, between consecutive identity colours. The golden angle
/// keeps neighbouring ids distinguishable however many there are.
pub const ID_HUE_STEP_DEGREES: f32 = 137.508;
pub const ID_SATURATION: f32 = 0.62;
pub const ID_LIGHTNESS: f32 = 0.62;
const ID_ALPHA: f32 = 0.95;

/// A stable colour for an integer identity such as a plate or a basin.
pub fn id_color(id: usize) -> Color {
    Color::hsla(
        (id as f32 * ID_HUE_STEP_DEGREES) % 360.0,
        ID_SATURATION,
        ID_LIGHTNESS,
        ID_ALPHA,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_ids_are_far_apart_in_hue() {
        let hue = |id: usize| (id as f32 * ID_HUE_STEP_DEGREES) % 360.0;
        for id in 0..64 {
            let separation = (hue(id) - hue(id + 1))
                .abs()
                .min(360.0 - (hue(id) - hue(id + 1)).abs());
            assert!(separation > 80.0, "ids {id} and {} share a hue", id + 1);
        }
    }
}
