//! Viewer settings stay separate from generation cases and camera paths.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[repr(u32)]
pub enum NormalMode {
    #[default]
    Averaged,
    Triangle,
    Density,
}
impl std::str::FromStr for NormalMode {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "averaged" => Ok(Self::Averaged),
            "triangle" => Ok(Self::Triangle),
            "density" => Ok(Self::Density),
            _ => Err("--normals needs averaged, triangle, or density"),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[repr(u32)]
pub enum SurfaceOverlay {
    #[default]
    Neutral,
    Lod,
    Normals,
    Agreement,
}
impl std::str::FromStr for SurfaceOverlay {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "neutral" => Ok(Self::Neutral),
            "lod" => Ok(Self::Lod),
            "normals" => Ok(Self::Normals),
            "agreement" => Ok(Self::Agreement),
            _ => Err("--surface needs neutral, lod, normals, or agreement"),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[repr(u32)]
pub enum SurfaceDetail {
    Plain,
    #[default]
    Textured,
}
impl std::str::FromStr for SurfaceDetail {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "plain" => Ok(Self::Plain),
            "textured" => Ok(Self::Textured),
            _ => Err("--surface-detail needs plain or textured"),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceViewConfig {
    pub normals: NormalMode,
    pub overlay: SurfaceOverlay,
    pub wireframe: bool,
    pub detail: SurfaceDetail,
}
