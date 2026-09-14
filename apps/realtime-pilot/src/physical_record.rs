//! Fixed native orbit/descent/walk/flight route and frame measurements.
use crate::physical_gpu_bridge::GpuStats;
use procgen_realtime_pilot::MeterPosition;
use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::PathBuf,
    time::Instant,
};

pub struct PhysicalRecord {
    start: Instant,
    output: BufWriter<File>,
    path: PathBuf,
    phase: usize,
    capture: usize,
}
#[derive(Clone, Copy)]
pub enum RouteAction {
    Descend,
    Ground,
    Walk,
    Fly,
    Orbit,
    Finish,
}
impl PhysicalRecord {
    pub fn new(path: PathBuf) -> io::Result<Self> {
        let mut output = BufWriter::new(File::create(&path)?);
        writeln!(
            output,
            "seconds,phase,frame_ms,height_tiles,height_bytes,height_update_ms,resident,finest_spacing_m,drawn,target,in_flight,retiring,gpu_bytes,resident_bytes,retiring_bytes,selection_ms,preparation_ms,encoding_ms,completion_ms,submission_latency_ms,publication_ms,scheduler_ms,draw_ms,gpu_density_ms,gpu_extraction_ms,x_m,y_m,z_m,walking,collision_ready,surface_target_bytes,status"
        )?;
        Ok(Self {
            start: Instant::now(),
            output,
            path,
            phase: 0,
            capture: 0,
        })
    }
    pub fn action(&mut self) -> Option<RouteAction> {
        let stages = [
            (5., RouteAction::Descend),
            (30., RouteAction::Ground),
            (60., RouteAction::Walk),
            (75., RouteAction::Fly),
            (85., RouteAction::Orbit),
            (90., RouteAction::Finish),
        ];
        if let Some(&(at, action)) = stages.get(self.phase)
            && self.start.elapsed().as_secs_f64() >= at
        {
            self.phase += 1;
            Some(action)
        } else {
            None
        }
    }
    pub fn moving(&self) -> bool {
        matches!(self.phase, 3 | 4)
    }
    pub fn screenshot(&mut self) -> Option<PathBuf> {
        let times = [4., 25., 55., 73., 83., 89.];
        if let Some(&at) = times.get(self.capture)
            && self.start.elapsed().as_secs_f64() >= at
        {
            self.capture += 1;
            Some(self.path.with_extension(format!("{}.png", self.capture)))
        } else {
            None
        }
    }
    pub fn write(
        &mut self,
        frame_ms: f32,
        stats: &GpuStats,
        eye: MeterPosition,
        walking: bool,
        collision_ready: bool,
    ) -> io::Result<()> {
        let p = eye.anchor();
        let (density, extraction) = stats
            .gpu_times
            .map(|t| (t.density_ms.to_string(), t.extraction_ms.to_string()))
            .unwrap_or_default();
        writeln!(
            self.output,
            "{:.3},{},{frame_ms:.3},{},{},{:.3},{},{},{},{},{},{},{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{density},{extraction},{},{},{},{walking},{collision_ready},{},{:?}",
            self.start.elapsed().as_secs_f64(),
            self.phase,
            stats.height_tiles,
            stats.height_bytes,
            stats.height_update_ms,
            stats.resident,
            stats
                .finest_spacing_m
                .map(|s| s.to_string())
                .unwrap_or_default(),
            stats.drawn,
            stats.target,
            stats.in_flight,
            stats.retiring,
            stats.bytes,
            stats.resident_bytes,
            stats.retiring_bytes,
            stats.selection_ms,
            stats.preparation_ms,
            stats.encoding_ms,
            stats.completion_ms,
            stats.submission_latency_ms,
            stats.publication_ms,
            stats.scheduler_ms,
            stats.draw_ms,
            p.x_m,
            p.y_m,
            p.z_m,
            stats.surface_target_bytes,
            stats.status
        )
    }
    pub fn finish(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}
