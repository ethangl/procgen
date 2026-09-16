//! Native flight and fixed-view visibility routes with frame measurements.
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
    route: RecordRoute,
    /// The displayed revision the scripted edit must replace, once it has been
    /// applied. `edit_pending` is true for every row until that revision changes.
    edited: Option<u64>,
}
#[derive(Clone, Copy)]
enum RecordRoute {
    Flight,
    Visibility,
}
#[derive(Clone, Copy)]
pub enum RecordView {
    Down,
    Horizon,
}
#[derive(Clone, Copy)]
pub enum RouteAction {
    Descend,
    Ground,
    Rise,
    Fly,
    Orbit,
    Finish,
    View { clearance_m: f32, view: RecordView },
}
/// One scripted design edit during the low flight phase, so the route measures
/// edit-to-display latency instead of an impression of it. It is not a stage:
/// the flight continues through it, and the phase column keeps its meaning.
const FLIGHT_EDIT_SECONDS: f64 = 40.;

const STAGES: [(f64, RouteAction); 6] = [
    (5., RouteAction::Descend),
    (30., RouteAction::Ground),
    (60., RouteAction::Fly),
    (75., RouteAction::Rise),
    (85., RouteAction::Orbit),
    (90., RouteAction::Finish),
];

const VISIBILITY_STAGES: [(f64, RouteAction); 5] = [
    (
        3.,
        RouteAction::View {
            clearance_m: 5.,
            view: RecordView::Down,
        },
    ),
    (
        11.,
        RouteAction::View {
            clearance_m: 5.,
            view: RecordView::Horizon,
        },
    ),
    (
        19.,
        RouteAction::View {
            clearance_m: 500.,
            view: RecordView::Down,
        },
    ),
    (
        27.,
        RouteAction::View {
            clearance_m: 500.,
            view: RecordView::Horizon,
        },
    ),
    (35., RouteAction::Finish),
];
impl RecordRoute {
    fn stages(self) -> &'static [(f64, RouteAction)] {
        match self {
            Self::Flight => &STAGES,
            Self::Visibility => &VISIBILITY_STAGES,
        }
    }
    fn captures(self) -> &'static [f64] {
        match self {
            Self::Flight => &[4., 25., 55., 73., 83., 89.],
            Self::Visibility => &[10., 18., 26., 34.],
        }
    }
}
impl PhysicalRecord {
    pub fn new(path: PathBuf) -> io::Result<Self> {
        Self::with_route(path, RecordRoute::Flight)
    }
    pub fn visibility(path: PathBuf) -> io::Result<Self> {
        Self::with_route(path, RecordRoute::Visibility)
    }
    fn with_route(path: PathBuf, route: RecordRoute) -> io::Result<Self> {
        let mut output = BufWriter::new(File::create(&path)?);
        writeln!(
            output,
            "seconds,phase,frame_ms,height_tiles,height_bytes,height_update_ms,selection_ms,scheduler_ms,draw_ms,x_m,y_m,z_m,surface_target_bytes,status,gpu_stats_fresh,height_build_ms,height_wait_ms,terrain_clearance_m,altitude_protection,height_generated_tiles,height_reused_tiles,height_drawn_tiles,height_tested_tiles,local_resident,local_drawn,local_target,local_in_flight,local_retiring,finest_spacing_m,coarsest_spacing_m,gpu_bytes,local_resident_bytes,local_retiring_bytes,preparation_ms,encoding_ms,completion_ms,submission_latency_ms,publication_ms,gpu_density_ms,gpu_extraction_ms,design_revision,edit_pending"
        )?;
        Ok(Self {
            start: Instant::now(),
            output,
            path,
            phase: 0,
            capture: 0,
            last_stats: GpuStats::default(),
            route,
            edited: None,
        })
    }
    pub fn action(&mut self) -> Option<RouteAction> {
        if let Some(&(at, action)) = self.route.stages().get(self.phase)
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
        self.phase.checked_sub(1).map(|i| self.route.stages()[i].1)
    }
    /// True once, on the flight route, when the caller must apply the scripted
    /// design edit. `displayed` is the revision it has to replace, which pins
    /// the `edit_pending` window down to that revision's publication.
    pub fn design_edit(&mut self, displayed: u64) -> bool {
        if self.edited.is_some()
            || !matches!(self.route, RecordRoute::Flight)
            || self.start.elapsed().as_secs_f64() < FLIGHT_EDIT_SECONDS
        {
            return false;
        }
        self.edited = Some(displayed);
        true
    }
    pub fn screenshot(&mut self) -> Option<PathBuf> {
        let times = self.route.captures();
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
        design_revision: u64,
    ) -> io::Result<()> {
        let edit_pending = self.edited == Some(design_revision);
        let gpu_stats_fresh = stats.is_some();
        if let Some(stats) = stats {
            self.last_stats.clone_from(stats);
        }
        let stats = &self.last_stats;
        let p = eye.anchor();
        let (density, extraction) = stats
            .gpu_times
            .map(|t| (t.density_ms.to_string(), t.extraction_ms.to_string()))
            .unwrap_or_default();
        writeln!(
            self.output,
            "{:.3},{},{frame_ms:.3},{},{},{:.3},{:.3},{:.3},{:.3},{},{},{},{},{:?},{gpu_stats_fresh},{:.3},{:.3},{clearance_m:.3},{protected},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{density},{extraction},{design_revision},{edit_pending}",
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
            stats.height_wait_ms,
            stats.height_generated_tiles,
            stats.height_reused_tiles,
            stats.height_drawn_tiles,
            stats.height_tested_tiles,
            stats.resident,
            stats.drawn,
            stats.target,
            stats.in_flight,
            stats.retiring,
            stats
                .finest_spacing_m
                .map(|m| m.to_string())
                .unwrap_or_default(),
            stats
                .coarsest_spacing_m
                .map(|m| m.to_string())
                .unwrap_or_default(),
            stats.bytes,
            stats.resident_bytes,
            stats.retiring_bytes,
            stats.preparation_ms,
            stats.encoding_ms,
            stats.completion_ms,
            stats.submission_latency_ms,
            stats.publication_ms
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
    fn visibility_route_changes_view_without_flight_input() {
        let path = std::env::temp_dir().join(format!(
            "procgen-visibility-record-{}.csv",
            std::process::id()
        ));
        let mut record = PhysicalRecord::visibility(path.clone()).unwrap();
        let mut clearances = Vec::new();
        for seconds in [3, 11, 19, 27] {
            record.start = Instant::now() - std::time::Duration::from_secs(seconds);
            let Some(RouteAction::View { clearance_m, .. }) = record.action() else {
                panic!("expected a fixed view");
            };
            clearances.push(clearance_m);
            assert!(
                !record.forward() && !record.rising(),
                "fixed views must not move"
            );
        }
        assert_eq!(
            clearances[0], clearances[1],
            "first comparison shares one altitude"
        );
        assert_eq!(
            clearances[2], clearances[3],
            "second comparison shares one altitude"
        );
        assert!(clearances[2] > clearances[0]);
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
            resident: 125,
            finest_spacing_m: Some(1),
            coarsest_spacing_m: Some(16),
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
        record.write(8.0, Some(&stats), eye, 5.0, true, 0).unwrap();
        record.write(8.0, None, eye, -10.0, false, 0).unwrap();
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
        assert_eq!(
            value(2, "local_resident"),
            "125",
            "retain the last GPU snapshot explicitly"
        );
        assert_eq!(value(2, "finest_spacing_m"), "1");
        assert_eq!(value(2, "coarsest_spacing_m"), "16");
        assert_eq!(value(2, "terrain_clearance_m"), "-10.000");
        assert_eq!(value(2, "altitude_protection"), "false");
        assert_eq!(value(2, "design_revision"), "0");
        assert_eq!(value(2, "edit_pending"), "false");
    }
    #[test]
    fn the_scripted_edit_fires_once_in_flight_and_stays_pending_until_the_revision_changes() {
        let path =
            std::env::temp_dir().join(format!("procgen-flight-edit-{}.csv", std::process::id()));
        let mut record = PhysicalRecord::new(path.clone()).unwrap();
        let eye = MeterPosition::new(
            VoxelPosition {
                x_m: 300000,
                y_m: 0,
                z_m: 0,
            },
            procgen_core::Vec3::ZERO,
        );
        assert!(
            !record.design_edit(0),
            "no edit before the low flight phase"
        );
        record.start = Instant::now() - std::time::Duration::from_secs(40);
        assert!(record.design_edit(0));
        assert!(!record.design_edit(0), "the route scripts one edit");
        record.write(8.0, None, eye, 5.0, true, 0).unwrap();
        record.write(8.0, None, eye, 5.0, true, 1).unwrap();
        record.finish().unwrap();
        drop(record);
        let csv = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        let rows: Vec<_> = csv
            .lines()
            .map(|line| line.split(',').collect::<Vec<_>>())
            .collect();
        let value = |row: usize, name| rows[row][rows[0].iter().position(|&h| h == name).unwrap()];
        assert_eq!(
            (value(1, "design_revision"), value(1, "edit_pending")),
            ("0", "true")
        );
        assert_eq!(
            (value(2, "design_revision"), value(2, "edit_pending")),
            ("1", "false")
        );
    }
    #[test]
    fn the_visibility_route_scripts_no_design_edit() {
        let path = std::env::temp_dir().join(format!(
            "procgen-visibility-edit-{}.csv",
            std::process::id()
        ));
        let mut record = PhysicalRecord::visibility(path.clone()).unwrap();
        record.start = Instant::now() - std::time::Duration::from_secs(40);
        assert!(!record.design_edit(0));
        drop(record);
        std::fs::remove_file(path).unwrap();
    }
}
