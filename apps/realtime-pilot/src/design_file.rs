//! Versioned, engine-independent exchange file for editable planet designs.
use procgen_realtime_pilot::PlanetDesignConfig;
use serde::{Deserialize, Serialize};
use std::{error::Error, fs, path::Path};

const FORMAT_VERSION: u32 = 1;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DesignFile {
    format_version: u32,
    design: PlanetDesignConfig,
}
pub fn encode(config: &PlanetDesignConfig) -> Result<String, Box<dyn Error>> {
    config.validate()?;
    Ok(serde_json::to_string_pretty(&DesignFile {
        format_version: FORMAT_VERSION,
        design: config.clone(),
    })? + "\n")
}
pub fn load(path: &Path) -> Result<PlanetDesignConfig, Box<dyn Error>> {
    let file: DesignFile = serde_json::from_str(&fs::read_to_string(path)?)?;
    if file.format_version != FORMAT_VERSION {
        return Err(format!(
            "unsupported design format {} (expected {FORMAT_VERSION})",
            file.format_version
        )
        .into());
    }
    file.design.validate()?;
    Ok(file.design)
}
pub fn save(path: &Path, config: &PlanetDesignConfig) -> Result<(), Box<dyn Error>> {
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
        save(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap(), config);
        let mut value: serde_json::Value = serde_json::from_str(&encode(&config).unwrap()).unwrap();
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
