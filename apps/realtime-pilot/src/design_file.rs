//! Versioned, engine-independent exchange file for editable planet designs.
use procgen_realtime_pilot::PlanetDesignConfig;
use serde::{Deserialize, Serialize};
use std::{error::Error, fs, path::Path};

const FORMAT_VERSION: u32 = 1;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignFile {
    format_version: u32,
    pub design: PlanetDesignConfig,
    #[serde(default)]
    pub ocean: crate::ocean::OceanConfig,
}
impl DesignFile {
    pub fn new(design: PlanetDesignConfig) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            design,
            ocean: Default::default(),
        }
    }
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        self.design.validate()?;
        self.ocean.validate()?;
        Ok(())
    }
}
pub fn encode(file: &DesignFile) -> Result<String, Box<dyn Error>> {
    file.validate()?;
    Ok(serde_json::to_string_pretty(file)? + "\n")
}
pub fn load(path: &Path) -> Result<DesignFile, Box<dyn Error>> {
    let file: DesignFile = serde_json::from_str(&fs::read_to_string(path)?)?;
    if file.format_version != FORMAT_VERSION {
        return Err(format!(
            "unsupported design format {} (expected {FORMAT_VERSION})",
            file.format_version
        )
        .into());
    }
    file.validate()?;
    Ok(file)
}
pub fn save(path: &Path, config: &DesignFile) -> Result<(), Box<dyn Error>> {
    fs::write(path, encode(config)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exchange_preserves_edits_and_rejects_unsupported_or_invalid_files() {
        let path = std::env::temp_dir().join(format!(
            "procgen-design-exchange-{}.json",
            std::process::id()
        ));
        let mut config = PlanetDesignConfig::starter(u64::MAX);
        config.octaves[3].enabled = false;
        config.octaves[7].amplitude_m = 12.5;
        config.octaves[10].perturbation = 0.9;
        let mut config = DesignFile::new(config);
        config.ocean.sea_level_m = 123.25;
        save(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap(), config);
        let mut value: serde_json::Value = serde_json::from_str(&encode(&config).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("ocean");
        fs::write(&path, value.to_string()).unwrap();
        assert_eq!(
            load(&path).unwrap().ocean,
            crate::ocean::OceanConfig::default()
        );
        value["ocean"] = serde_json::json!({"enabled": true, "sea_level_m": 20001});
        fs::write(&path, value.to_string()).unwrap();
        assert!(load(&path).unwrap_err().to_string().contains("sea level"));
        value.as_object_mut().unwrap().remove("ocean");
        value["format_version"] = 2.into();
        fs::write(&path, value.to_string()).unwrap();
        assert!(
            load(&path)
                .unwrap_err()
                .to_string()
                .contains("unsupported design format")
        );
        value["format_version"] = FORMAT_VERSION.into();
        value["design"]["octaves"][1]["wavelength_m"] = 16_000_000.into();
        fs::write(&path, value.to_string()).unwrap();
        assert!(
            load(&path)
                .unwrap_err()
                .to_string()
                .contains("strictly decrease")
        );
        fs::remove_file(path).unwrap();
    }
}
