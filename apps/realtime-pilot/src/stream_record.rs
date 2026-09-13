use procgen_realtime_pilot::{
    DetailLevel, MANAGED_MEMORY_LIMIT, PILOT_PLANET, ROUTE_SECONDS, SOURCE_WORK_RESERVATION,
    StreamView, StreamingWorld, streaming_route,
};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
};

pub struct RecordingConfig {
    pub path: PathBuf,
    pub screenshots: bool,
}
pub struct FrameTiming {
    pub seconds: f64,
    pub install_ms: f64,
    pub upload_bytes: usize,
}
pub struct Recording {
    config: RecordingConfig,
    writer: BufWriter<File>,
    elapsed: f32,
    frames: Vec<f64>,
    last_capture: Option<&'static str>,
    seed: u64,
}
impl Recording {
    pub fn new(config: RecordingConfig, seed: u64) -> std::io::Result<Self> {
        if let Some(parent) = config.path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut writer = BufWriter::new(File::create(&config.path)?);
        writeln!(
            writer,
            "seconds,phase,x,y,z,fx,fy,fz,frame_ms,install_ms,upload_bytes,managed_bytes,active,queued,cancellations,rejected,installations,waiting_frames,lod_px,lod_nx,lod_py,lod_ny,lod_pz,lod_nz,resident_px,resident_nx,resident_py,resident_ny,resident_pz,resident_nz,requested_px,requested_nx,requested_py,requested_ny,requested_pz,requested_nz"
        )?;
        Ok(Self {
            config,
            writer,
            elapsed: 0.0,
            frames: Vec::new(),
            last_capture: None,
            seed,
        })
    }
    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }
    pub fn screenshot(&mut self) -> Option<PathBuf> {
        let sample = streaming_route(self.elapsed);
        if self.config.screenshots
            && sample.phase_seconds >= 1.5
            && self.last_capture != Some(sample.phase)
        {
            self.last_capture = Some(sample.phase);
            Some(
                self.config
                    .path
                    .with_extension(format!("{}.png", sample.phase)),
            )
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
        let stats = world.stats();
        let phase = streaming_route(self.elapsed).phase;
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
        writeln!(self.writer)?;
        self.frames.push(timing.seconds * 1000.0);
        self.elapsed += timing.seconds as f32;
        if self.elapsed < ROUTE_SECONDS {
            return Ok(None);
        }
        self.frames.sort_by(f64::total_cmp);
        let percentile = |p: f64| self.frames[((self.frames.len() - 1) as f64 * p) as usize];
        let summary = format!(
            "seed={}\nplanet={PILOT_PLANET:#?}\nsource_work_reservation_bytes={SOURCE_WORK_RESERVATION}\nmanaged_limit_bytes={MANAGED_MEMORY_LIMIT}\nviewport=1280x900 logical pixels\nscreenshots={}\nroute_seconds={}\nframes={}\nframe_p50_ms={:.3}\nframe_p95_ms={:.3}\nframe_p99_ms={:.3}\nframe_max_ms={:.3}\nstats={stats:?}\n",
            self.seed,
            self.config.screenshots,
            self.elapsed,
            self.frames.len(),
            percentile(0.5),
            percentile(0.95),
            percentile(0.99),
            self.frames.last().expect("recorded frame")
        );
        self.writer.flush()?;
        std::fs::write(self.config.path.with_extension("summary.txt"), &summary)?;
        Ok(Some(summary))
    }
}
