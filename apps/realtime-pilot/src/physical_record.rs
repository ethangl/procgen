//! Fixed native orbit/descent/walk/flight route and frame measurements.
use crate::physical_gpu_bridge::GpuStats;
use procgen_realtime_pilot::{ContactError, MeterPosition, VoxelCollisionError};
use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::PathBuf,
    time::Instant,
};

#[derive(Clone, Copy, Debug)]
pub enum MotionOutcome {
    Idle,
    Advanced,
    MissingCoverage,
    NoLanding,
    Overlap,
    SweepLimit,
    Invalid,
}
impl MotionOutcome {
    pub fn from_error(error: &VoxelCollisionError) -> Self {
        match error {
            VoxelCollisionError::Contact(ContactError::MissingCoverage) => Self::MissingCoverage,
            VoxelCollisionError::Contact(ContactError::NoLanding) => Self::NoLanding,
            VoxelCollisionError::Contact(ContactError::SweepLimit) => Self::SweepLimit,
            VoxelCollisionError::Overlap => Self::Overlap,
            _ => Self::Invalid,
        }
    }
}
pub struct ContactFrame {
    pub walking: bool,
    pub grounded: bool,
    pub covered: bool,
    pub building: bool,
    pub build_seconds: f32,
    pub motion: MotionOutcome,
}

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
            "seconds,phase,frame_ms,height_tiles,height_bytes,height_update_ms,resident,finest_spacing_m,drawn,target,in_flight,retiring,gpu_bytes,resident_bytes,retiring_bytes,selection_ms,preparation_ms,encoding_ms,completion_ms,submission_latency_ms,publication_ms,scheduler_ms,draw_ms,gpu_density_ms,gpu_extraction_ms,x_m,y_m,z_m,walking,collision_ready,surface_target_bytes,status,collision_building,collision_seconds,grounded,motion,gpu_stats_fresh,height_build_ms,height_wait_ms"
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
        stats: Option<&GpuStats>,
        eye: MeterPosition,
        contact: ContactFrame,
    ) -> io::Result<()> {
        let gpu_stats_fresh = stats.is_some();
        if let Some(stats) = stats {
            self.last_stats.clone_from(stats);
        }
        let stats = &self.last_stats;
        let p = eye.anchor();
        let ContactFrame {
            walking,
            covered: collision_ready,
            building,
            build_seconds,
            grounded,
            motion,
        } = contact;
        let (density, extraction) = stats
            .gpu_times
            .map(|t| (t.density_ms.to_string(), t.extraction_ms.to_string()))
            .unwrap_or_default();
        writeln!(
            self.output,
            "{:.3},{},{frame_ms:.3},{},{},{:.3},{},{},{},{},{},{},{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{density},{extraction},{},{},{},{walking},{collision_ready},{},{:?},{building},{build_seconds:.3},{grounded},{motion:?},{gpu_stats_fresh},{:.3},{:.3}",
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
    fn collision_failures_are_recorded_even_when_gpu_statistics_are_busy() {
        let path =
            std::env::temp_dir().join(format!("procgen-contact-record-{}.csv", std::process::id()));
        let mut record = PhysicalRecord::new(path.clone()).unwrap();
        let stats = GpuStats {
            resident: 125,
            ..Default::default()
        };
        let eye = MeterPosition::new(
            VoxelPosition {
                x_m: 300_000,
                y_m: 0,
                z_m: 0,
            },
            procgen_core::Vec3::ZERO,
        );
        for (stats, motion, covered) in [
            (Some(&stats), MotionOutcome::Advanced, true),
            (None, MotionOutcome::MissingCoverage, false),
        ] {
            record
                .write(
                    8.0,
                    stats,
                    eye,
                    ContactFrame {
                        walking: true,
                        grounded: false,
                        covered,
                        building: true,
                        build_seconds: 1.5,
                        motion,
                    },
                )
                .unwrap();
        }
        record.finish().unwrap();
        drop(record);
        let csv = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        let rows: Vec<_> = csv
            .lines()
            .map(|line| line.split(',').collect::<Vec<_>>())
            .collect();
        assert_eq!(rows.len(), 3, "every movement update has a row");
        let value = |row: usize, name| rows[row][rows[0].iter().position(|&h| h == name).unwrap()];
        assert_eq!(value(1, "gpu_stats_fresh"), "true");
        assert_eq!(value(2, "gpu_stats_fresh"), "false");
        assert_eq!(
            value(2, "resident"),
            "125",
            "retain the last GPU snapshot explicitly"
        );
        assert_eq!(value(2, "motion"), "MissingCoverage");
        assert_eq!(value(2, "collision_ready"), "false");
    }
}
