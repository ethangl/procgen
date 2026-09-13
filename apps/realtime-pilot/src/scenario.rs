//! Resolved inputs and spherical adaptations of the three local presets.
use crate::{PILOT_PLANET, PRESETS, PlanetConfig, PlanetError, PlanetField, TerrainConfig};
use serde::{Deserialize, Serialize};
pub const TOOLCHAIN: &str = env!("PILOT_RUSTC");
pub const BUILD_ID: &str = env!("PILOT_BUILD_ID");
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RouteKind {
    Flight,
    Walk,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub seed: u64,
    pub planet: PlanetConfig,
    pub route: RouteKind,
}
impl Scenario {
    pub fn validate(&self) -> Result<PlanetField, PlanetError> {
        let field = self.planet.validate(self.seed)?;
        // Fixed route distances, streaming resolution, and budgets target this radius.
        if self.planet.radius != PILOT_PLANET.radius {
            return Err(PlanetError::PilotRadius);
        }
        Ok(field)
    }
}
pub struct PlanetPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub planet: PlanetConfig,
}
pub const PLANET_PRESETS: [PlanetPreset; 3] = [
    PlanetPreset {
        id: "hills",
        name: "Rounded hills",
        planet: PILOT_PLANET,
    },
    PlanetPreset {
        id: "ridges",
        name: "Ridged mountains",
        planet: PlanetConfig {
            terrain: TerrainConfig {
                noise: PRESETS[1].config.noise,
                height_scale: 0.7,
                detail_scale: 0.055,
                cave_density: 2.5,
            },
            ..PILOT_PLANET
        },
    },
    PlanetPreset {
        id: "basins",
        name: "Broad basins",
        planet: PlanetConfig {
            terrain: TerrainConfig {
                noise: PRESETS[2].config.noise,
                height_scale: 0.5,
                detail_scale: 0.025,
                cave_density: 1.5,
            },
            ..PILOT_PLANET
        },
    },
];
