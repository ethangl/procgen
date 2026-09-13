use crate::{noise::NoiseConfig, terrain::TerrainConfig};

pub struct Preset {
    pub name: &'static str,
    pub config: TerrainConfig,
}

/// Fixed local test scenes. Lengths are relative to the two-unit inspection box.
pub const PRESETS: [Preset; 3] = [
    Preset {
        name: "Rounded hills",
        config: TerrainConfig {
            noise: NoiseConfig {
                wavelength: 0.8,
                sharpness: 0.55,
                perturbation: 0.2,
                slope_erosion: 0.15,
                altitude_erosion: 0.0,
                ridge_erosion: 0.0,
                gain: 0.55,
            },
            height_scale: 2.5,
            detail_scale: 0.12,
            cave_density: 2.0,
        },
    },
    Preset {
        name: "Ridged mountains",
        config: TerrainConfig {
            noise: NoiseConfig {
                wavelength: 1.2,
                sharpness: -0.85,
                perturbation: 0.65,
                slope_erosion: 0.15,
                altitude_erosion: 0.15,
                ridge_erosion: 0.2,
                gain: 0.65,
            },
            height_scale: 1.1,
            detail_scale: 0.2,
            cave_density: 3.0,
        },
    },
    Preset {
        name: "Broad basins",
        config: TerrainConfig {
            noise: NoiseConfig {
                wavelength: 1.6,
                sharpness: 0.0,
                perturbation: 0.35,
                slope_erosion: 0.8,
                altitude_erosion: 0.8,
                ridge_erosion: 0.5,
                gain: 0.6,
            },
            height_scale: 2.0,
            detail_scale: 0.08,
            cave_density: 1.5,
        },
    },
];
