//! Saved viewer water settings, separate from terrain generation parameters.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OceanConfig {
    pub enabled: bool,
    /// Meters above the terrain reference sphere.
    pub sea_level_m: f32,
}
impl Default for OceanConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sea_level_m: 0.0,
        }
    }
}
impl OceanConfig {
    pub const SEA_LEVEL_RANGE: std::ops::RangeInclusive<f32> = -20_000.0..=20_000.0;

    pub fn validate(self) -> Result<(), String> {
        if !Self::SEA_LEVEL_RANGE.contains(&self.sea_level_m) {
            return Err(format!(
                "ocean sea level must be finite and between {} and {} meters",
                Self::SEA_LEVEL_RANGE.start(),
                Self::SEA_LEVEL_RANGE.end()
            ));
        }
        Ok(())
    }
}
