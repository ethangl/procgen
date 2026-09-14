//! Fixed native orbit/descent/flight route and frame measurements.
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
    last_stats: GpuStats,
}
#[derive(Clone, Copy)]
pub enum RouteAction {
    Descend,
    Ground,
    Rise,
    Fly,
    Orbit,
    Finish,
}
const STAGES: [(f64, RouteAction); 6] = [
    (5., RouteAction::Descend),
    (30., RouteAction::Ground),
    (60., RouteAction::Fly),
    (75., RouteAction::Rise),
    (85., RouteAction::Orbit),
    (90., RouteAction::Finish),
];

impl PhysicalRecord {
    pub fn new(path: PathBuf) -> io::Result<Self> {
        let mut output = BufWriter::new(File::create(&path)?);
        writeln!(
            output,
            "seconds,phase,frame_ms,height_tiles,height_bytes,height_update_ms,selection_ms,scheduler_ms,draw_ms,x_m,y_m,z_m,surface_target_bytes,status,gpu_stats_fresh,height_build_ms,height_wait_ms,terrain_clearance_m,altitude_protection"
        )?;
        Ok(Self {
            start: Instant::now(),
            output,
            path,
            phase: 0,
            capture: 0,
            last_stats: GpuStats::default(),
        })
    }
    pub fn action(&mut self) -> Option<RouteAction> {
        if let Some(&(at, action)) = STAGES.get(self.phase)
            && self.start.elapsed().as_secs_f64() >= at
        {
            self.phase += 1;
            Some(action)
        } else {
            None
        }
    }
    pub fn forward(&self) -> bool {
        matches!(
            self.current_action(),
            Some(RouteAction::Ground | RouteAction::Fly)
        )
    }
    pub fn rising(&self) -> bool {
        matches!(self.current_action(), Some(RouteAction::Rise))
    }
    fn current_action(&self) -> Option<RouteAction> {
        self.phase.checked_sub(1).map(|i| STAGES[i].1)
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
        stats: Option<&GpuStats>,
        eye: MeterPosition,
        clearance_m: f32,
        protected: bool,
    ) -> io::Result<()> {
        let gpu_stats_fresh = stats.is_some();
        if let Some(stats) = stats {
            self.last_stats.clone_from(stats);
        }
        let stats = &self.last_stats;
        let p = eye.anchor();
        writeln!(
            self.output,
            "{:.3},{},{frame_ms:.3},{},{},{:.3},{:.3},{:.3},{:.3},{},{},{},{},{:?},{gpu_stats_fresh},{:.3},{:.3},{clearance_m:.3},{protected}",
            self.start.elapsed().as_secs_f64(),
            self.phase,
            stats.height_tiles,
            stats.height_bytes,
            stats.height_update_ms,
            stats.selection_ms,
            stats.scheduler_ms,
            stats.draw_ms,
            p.x_m,
            p.y_m,
            p.z_m,
            stats.surface_target_bytes,
            stats.status,
            stats.height_build_ms,
            stats.height_wait_ms
        )
    }
    pub fn finish(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_realtime_pilot::VoxelPosition;
    #[test]
    fn recorded_route_flies_near_ground_then_rises_before_returning_to_orbit() {
        let path =
            std::env::temp_dir().join(format!("procgen-flight-input-{}.csv", std::process::id()));
        let mut record = PhysicalRecord::new(path.clone()).unwrap();
        assert!(!record.forward() && !record.rising());
        for (seconds, forward, rising) in [
            (5, false, false),
            (30, true, false),
            (60, true, false),
            (75, false, true),
            (85, false, false),
        ] {
            record.start = Instant::now() - std::time::Duration::from_secs(seconds);
            assert!(record.action().is_some());
            assert_eq!(record.forward(), forward, "at {seconds} seconds");
            assert_eq!(record.rising(), rising, "at {seconds} seconds");
        }
        drop(record);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn fresh_camera_clearance_is_recorded_when_gpu_statistics_are_busy() {
        let path =
            std::env::temp_dir().join(format!("procgen-flight-record-{}.csv", std::process::id()));
        let mut record = PhysicalRecord::new(path.clone()).unwrap();
        let stats = GpuStats {
            height_tiles: 384,
            ..Default::default()
        };
        let eye = MeterPosition::new(
            VoxelPosition {
                x_m: 300000,
                y_m: 0,
                z_m: 0,
            },
            procgen_core::Vec3::ZERO,
        );
        record.write(8.0, Some(&stats), eye, 5.0, true).unwrap();
        record.write(8.0, None, eye, -10.0, false).unwrap();
        record.finish().unwrap();
        drop(record);
        let csv = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        let rows: Vec<_> = csv
            .lines()
            .map(|line| line.split(',').collect::<Vec<_>>())
            .collect();
        assert_eq!(rows.len(), 3);
        let value = |row: usize, name| rows[row][rows[0].iter().position(|&h| h == name).unwrap()];
        assert_eq!(value(2, "height_tiles"), "384");
        assert_eq!(value(2, "gpu_stats_fresh"), "false");
        assert_eq!(value(2, "terrain_clearance_m"), "-10.000");
        assert_eq!(value(2, "altitude_protection"), "false");
    }
}
