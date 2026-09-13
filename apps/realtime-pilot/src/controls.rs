use bevy_egui::egui;
use procgen_realtime_pilot::{NoiseConfig, TerrainConfig};

pub fn terrain(ui: &mut egui::Ui, config: &mut TerrainConfig) {
    // Leave enough room for the damping labels beside their value editors.
    ui.spacing_mut().slider_width = 75.0;
    for (label, value, range) in [
        (
            "Wavelength",
            &mut config.noise.wavelength,
            NoiseConfig::WAVELENGTH_RANGE,
        ),
        (
            "Sharpness",
            &mut config.noise.sharpness,
            NoiseConfig::SHARPNESS_RANGE,
        ),
        (
            "Warp",
            &mut config.noise.perturbation,
            NoiseConfig::SHAPING_RANGE,
        ),
        (
            "Slope damping",
            &mut config.noise.slope_erosion,
            NoiseConfig::SHAPING_RANGE,
        ),
        (
            "Altitude damping",
            &mut config.noise.altitude_erosion,
            NoiseConfig::SHAPING_RANGE,
        ),
        (
            "Ridge damping",
            &mut config.noise.ridge_erosion,
            NoiseConfig::SHAPING_RANGE,
        ),
        ("Gain", &mut config.noise.gain, NoiseConfig::GAIN_RANGE),
        (
            "Height",
            &mut config.height_scale,
            TerrainConfig::HEIGHT_RANGE,
        ),
        (
            "3D detail",
            &mut config.detail_scale,
            TerrainConfig::DETAIL_RANGE,
        ),
        (
            "Caves / area",
            &mut config.cave_density,
            TerrainConfig::CAVE_DENSITY_RANGE,
        ),
    ] {
        ui.add(egui::Slider::new(value, range).text(label));
    }
}
