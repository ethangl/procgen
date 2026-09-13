use crate::replay::{Case, MAX_FRAMES, Pose, Replay};
use procgen_realtime_pilot::{
    BUILD_ID, DetailLevel, MANAGED_MEMORY_LIMIT, ROUTE_SECONDS, SOURCE_WORK_RESERVATION, Scenario,
    StreamView, StreamingWorld, TOOLCHAIN, streaming_route,
};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
};

pub use procgen_realtime_pilot::RouteKind as RecordingRoute;
pub struct RecordingConfig {
    pub path: PathBuf,
    pub screenshots: bool,
    pub surface_view: crate::surface_view::SurfaceViewConfig,
    pub replay_build: Option<String>,
}
pub struct FrameTiming {
    pub seconds: f64,
    pub install_ms: f64,
    pub upload_bytes: usize,
    pub walking_ms: f64,
    pub population_ms: f64,
    pub clearance: Option<f32>,
    pub grounded: bool,
    pub clamped_frames: usize,
    pub collision_failures: usize,
}
pub struct Recording {
    config: RecordingConfig,
    writer: BufWriter<File>,
    elapsed: f32,
    frames: Vec<f64>,
    last_capture: Option<&'static str>,
    scenario: Scenario,
    poses: Vec<Pose>,
}
impl Recording {
    pub fn new(config: RecordingConfig, scenario: Scenario) -> std::io::Result<Self> {
        if let Some(parent) = config.path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        Case::new(scenario)
            .save(&config.path.with_extension("case.json"))
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(
            config.path.with_extension("view.json"),
            serde_json::to_vec_pretty(&config.surface_view).map_err(std::io::Error::other)?,
        )?;
        let mut writer = BufWriter::new(File::create(&config.path)?);
        writeln!(
            writer,
            "seconds,phase,x,y,z,fx,fy,fz,frame_ms,install_ms,upload_bytes,managed_bytes,active,queued,cancellations,rejected,installations,waiting_frames,lod_px,lod_nx,lod_py,lod_ny,lod_pz,lod_nz,resident_px,resident_nx,resident_py,resident_ny,resident_pz,resident_nz,requested_px,requested_nx,requested_py,requested_ny,requested_pz,requested_nz,walking_ms,population_ms,clearance,grounded,clamped_frames,collision_failures"
        )?;
        Ok(Self {
            config,
            writer,
            elapsed: 0.0,
            frames: Vec::new(),
            last_capture: None,
            scenario,
            poses: Vec::new(),
        })
    }
    pub fn route(&self) -> RecordingRoute {
        self.scenario.route
    }
    pub fn phase(&self) -> (&'static str, f32) {
        if self.scenario.route == RecordingRoute::Flight {
            let r = streaming_route(self.elapsed);
            return (r.phase, r.phase_seconds);
        }
        let (phase, start) = if self.elapsed < 12.0 {
            ("walk-out", 0.0)
        } else if self.elapsed < 18.0 {
            ("turn-in-place", 12.0)
        } else if self.elapsed < 30.0 {
            ("walk-back", 18.0)
        } else {
            ("rest", 30.0)
        };
        (phase, self.elapsed - start)
    }
    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }
    pub fn screenshot(&mut self) -> Option<PathBuf> {
        let (phase, phase_seconds) = self.phase();
        if self.config.screenshots && phase_seconds >= 1.5 && self.last_capture != Some(phase) {
            self.last_capture = Some(phase);
            Some(self.config.path.with_extension(format!("{}.png", phase)))
        } else {
            None
        }
    }
    pub fn frame(
        &mut self,
        world: &StreamingWorld,
        view: StreamView,
        timing: FrameTiming,
    ) -> std::io::Result<Option<String>> {
        if self.poses.len() == MAX_FRAMES {
            return Err(std::io::Error::other("recording reached 16384 poses"));
        }
        self.poses.push(Pose::new(self.elapsed, view));
        let stats = world.stats();
        let (phase, _) = self.phase();
        let p = view.position;
        let f = view.forward;
        write!(
            self.writer,
            "{:.6},{phase},{},{},{},{},{},{},{:.4},{:.4},{},{},{},{},{},{},{},{}",
            self.elapsed,
            p.x,
            p.y,
            p.z,
            f.x,
            f.y,
            f.z,
            timing.seconds * 1000.0,
            timing.install_ms,
            timing.upload_bytes,
            stats.managed_bytes,
            stats.active_jobs,
            stats.queued_jobs,
            stats.cancellations,
            stats.rejected_results,
            stats.installations,
            stats.waiting_frames
        )?;
        for level in world.resident_levels() {
            let level = match level {
                Some(DetailLevel::Coarse) => 0,
                Some(DetailLevel::Medium) => 1,
                Some(DetailLevel::Fine) => 2,
                None => -1,
            };
            write!(self.writer, ",{level}")?;
        }
        for ticket in world.resident_tickets() {
            match ticket {
                Some(ticket) => write!(self.writer, ",{}", ticket.serial)?,
                None => write!(self.writer, ",-1")?,
            }
        }
        for ticket in world.requested_tickets() {
            write!(self.writer, ",{}", ticket.serial)?;
        }
        write!(
            self.writer,
            ",{:.4},{:.4},",
            timing.walking_ms, timing.population_ms
        )?;
        if let Some(clearance) = timing.clearance {
            write!(self.writer, "{clearance:.6}")?;
        }
        writeln!(
            self.writer,
            ",{},{},{}",
            timing.grounded, timing.clamped_frames, timing.collision_failures
        )?;
        self.frames.push(timing.seconds * 1000.0);
        self.elapsed += timing.seconds as f32;
        if self.elapsed < ROUTE_SECONDS {
            return Ok(None);
        }
        self.frames.sort_by(f64::total_cmp);
        let percentile = |p: f64| self.frames[((self.frames.len() - 1) as f64 * p) as usize];
        let summary = format!(
            "build={BUILD_ID}\nreplay_source_build={:?}\ntoolchain={TOOLCHAIN}\nplatform={}\nsurface_view={:?}\nroute={:?}\nseed={}\nplanet={:#?}\nsource_work_reservation_bytes={SOURCE_WORK_RESERVATION}\nmanaged_limit_bytes={MANAGED_MEMORY_LIMIT}\nviewport=1280x900 logical pixels\nscreenshots={}\nroute_seconds={}\nframes={}\nframe_p50_ms={:.3}\nframe_p95_ms={:.3}\nframe_p99_ms={:.3}\nframe_max_ms={:.3}\nstats={stats:?}\n",
            self.config.replay_build,
            std::env::consts::OS,
            self.config.surface_view,
            self.scenario.route,
            self.scenario.seed,
            self.scenario.planet,
            self.config.screenshots,
            self.elapsed,
            self.frames.len(),
            percentile(0.5),
            percentile(0.95),
            percentile(0.99),
            self.frames.last().expect("recorded frame")
        );
        self.writer.flush()?;
        Replay {
            case: Case::new(self.scenario),
            poses: std::mem::take(&mut self.poses),
        }
        .save(&self.config.path.with_extension("replay.json"))
        .map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(self.config.path.with_extension("summary.txt"), &summary)?;
        Ok(Some(summary))
    }
}
