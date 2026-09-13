//! Versioned camera capture. Replay reproduces views, not asynchronous timing or physics.
use procgen_core::Vec3;
use procgen_realtime_pilot::{BUILD_ID, ROUTE_SECONDS, Scenario, StreamView};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
pub const MAX_FRAMES: usize = 16384;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
pub type ArtifactResult<T> = Result<T, Box<dyn std::error::Error>>;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub version: u32,
    pub build: String,
    pub scenario: Scenario,
}
impl Case {
    pub fn new(scenario: Scenario) -> Self {
        Self {
            version: 1,
            build: BUILD_ID.into(),
            scenario,
        }
    }
    pub fn load(path: &Path) -> ArtifactResult<Self> {
        let case: Self = read(path)?;
        case.validate()?;
        Ok(case)
    }
    pub fn validate(&self) -> ArtifactResult<()> {
        if self.version != 1 {
            return Err("unsupported case version".into());
        }
        Ok(())
    }
    pub fn save(&self, path: &Path) -> ArtifactResult<()> {
        write(path, self)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pose {
    pub seconds: f32,
    pub position: [f32; 3],
    pub forward: [f32; 3],
}
impl Pose {
    pub fn new(seconds: f32, view: StreamView) -> Self {
        let p = view.position;
        let f = view.forward;
        Self {
            seconds,
            position: [p.x, p.y, p.z],
            forward: [f.x, f.y, f.z],
        }
    }
    fn view(&self) -> StreamView {
        StreamView {
            position: vector(self.position),
            forward: vector(self.forward),
        }
    }
}
fn vector(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], p[2])
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replay {
    pub case: Case,
    pub poses: Vec<Pose>,
}
impl Replay {
    pub fn load(path: &Path) -> ArtifactResult<Self> {
        let r: Self = read(path)?;
        r.validate()?;
        Ok(r)
    }
    fn validate(&self) -> ArtifactResult<()> {
        self.case.validate()?;
        self.case.scenario.validate()?;
        if self.poses.len() < 2 || self.poses.len() > MAX_FRAMES || self.poses[0].seconds != 0.0 {
            return Err("replay must start at zero and contain 2..=16384 poses".into());
        }
        for (i, p) in self.poses.iter().enumerate() {
            let view = p.view();
            if !p.seconds.is_finite()
                || !(0.0..=ROUTE_SECONDS).contains(&p.seconds)
                || (i > 0 && p.seconds <= self.poses[i - 1].seconds)
                || !view.position.is_finite()
                || view.position.length() > 100.0
                || !view.forward.is_finite()
                || (view.forward.length() - 1.0).abs() > 0.001
            {
                return Err("invalid or unordered replay pose".into());
            }
        }
        Ok(())
    }
    pub fn save(&self, path: &Path) -> ArtifactResult<()> {
        self.validate()?;
        write(path, self)
    }
    /// Step playback uses the exact captured pose preceding this elapsed time.
    /// It avoids inventing intermediate rotations during rapid turns.
    #[cfg(any(feature = "inspector", test))]
    pub fn sample(&self, seconds: f32) -> StreamView {
        let i = self
            .poses
            .partition_point(|p| p.seconds <= seconds)
            .saturating_sub(1);
        self.poses[i].view()
    }
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> ArtifactResult<T> {
    if fs::metadata(path)?.len() > MAX_FILE_BYTES {
        return Err("artifact exceeds 8 MiB".into());
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn write<T: Serialize>(path: &Path, value: &T) -> ArtifactResult<()> {
    if let Some(p) = path.parent()
        && !p.as_os_str().is_empty()
    {
        fs::create_dir_all(p)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use procgen_realtime_pilot::{PILOT_PLANET, RouteKind};
    #[test]
    fn round_trip_preserves_large_seed_and_rejects_bad_timelines() {
        let case = Case::new(Scenario {
            seed: u64::MAX,
            planet: PILOT_PLANET,
            route: RouteKind::Flight,
        });
        let view = StreamView {
            position: Vec3::Z * 13.0,
            forward: -Vec3::Z,
        };
        let mut r = Replay {
            case,
            poses: vec![
                Pose::new(0.0, view),
                Pose::new(
                    1.0,
                    StreamView {
                        position: Vec3::X,
                        forward: -Vec3::X,
                    },
                ),
            ],
        };
        let decoded: Replay = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded.case.scenario.seed, u64::MAX);
        assert_eq!(decoded.case.scenario.planet, PILOT_PLANET);
        assert_eq!(decoded.sample(0.9).position, Vec3::Z * 13.0);
        assert_eq!(decoded.sample(1.0).position, Vec3::X);
        r.poses[1].seconds = 0.0;
        assert!(r.validate().is_err());
        r.poses[1].seconds = 1.0;
        r.poses[1].forward = [0.0; 3];
        assert!(r.validate().is_err());
    }
}
