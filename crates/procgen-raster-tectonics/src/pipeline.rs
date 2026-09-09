//! wgpu dispatch for the raster tectonics pipeline.
//!
//! This module holds no per-cell logic: it allocates the stage buffers, packs
//! the configuration, sequences the kernels the stage modules define, and
//! copies back the diagnostics they counted. Buffers stay resident between runs
//! so a settings change reruns the stages rather than reallocating them.

use crate::device::{
    BINDING_COUNT, PipelineTuning, RasterTectonicsError, validate_device, validate_resolution,
};
use crate::evolution::RasterEvolutionConfig;
use crate::field::{PLATE_ID_COUNT, RasterPlate};
use crate::kernels::tectonics_kernel_source;
use crate::layout::{PackedDiagnostics, PackedTectonicsConfig, PackedTectonicsState};
use crate::partition::{
    RasterPlatePartitionConfig, SEED_REDUCTION_WORKGROUPS, frontier_size, growth_passes,
};
use procgen_tectonics::BoundaryClass;
use std::{
    mem::offset_of,
    sync::mpsc,
    time::{Duration, Instant},
};

/// Everything one tectonics run is configured by.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RasterTectonicsConfig {
    pub partition: RasterPlatePartitionConfig,
    pub evolution: RasterEvolutionConfig,
}

impl RasterTectonicsConfig {
    pub(crate) fn validate(&self, resolution: u32) -> Result<(), RasterTectonicsError> {
        self.partition.validate(resolution)?;
        self.evolution.validate()
    }
}

/// The stages a run reports separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineStage {
    /// Clearing the resident buffers for a new run.
    Initialize,
    /// Farthest-point seed selection for every plate.
    PlateSeeds,
    /// Both frontier relaxations: majors alone, then majors with minors.
    PlateGrowth,
    /// Per-plate area reduction and the crust classes it decides.
    CrustClassification,
    /// Every boundary classification the evolution runs.
    BoundaryClassification,
    /// Every migration step the evolution runs.
    PlateMigration,
}

impl PipelineStage {
    pub const ALL: [Self; 6] = [
        Self::Initialize,
        Self::PlateSeeds,
        Self::PlateGrowth,
        Self::CrustClassification,
        Self::BoundaryClassification,
        Self::PlateMigration,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Initialize => "Initialize",
            Self::PlateSeeds => "Plate seeds",
            Self::PlateGrowth => "Plate growth",
            Self::CrustClassification => "Crust",
            Self::BoundaryClassification => "Boundaries",
            Self::PlateMigration => "Migration",
        }
    }
}

/// Wall-clock time each stage took from submission to device idle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StageTimings {
    durations: [Duration; PipelineStage::ALL.len()],
}

impl StageTimings {
    pub fn duration(&self, stage: PipelineStage) -> Duration {
        self.durations[stage as usize]
    }

    pub fn total(&self) -> Duration {
        self.durations.iter().sum()
    }

    fn record(&mut self, stage: PipelineStage, elapsed: Duration) {
        self.durations[stage as usize] += elapsed;
    }
}

/// What one run produced, counted on the device and copied back once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TectonicsDiagnostics {
    /// Frontier passes the longer of the two growth relaxations consumed. The
    /// frontier's order varies run to run, so this varies by a pass or two
    /// while the labels it settles on do not. Read it as a cost, not a result.
    pub longest_relaxation_passes: u32,
    /// Share of the sphere's area the oceanic plates cover.
    pub ocean_fraction: f32,
    /// Plates that own no cell, which a head start large enough to claim every
    /// cell leaves behind.
    pub empty_plate_count: u32,
    /// Ownership changes across every migration step. A cell can migrate more
    /// than once.
    pub migrated_cell_count: u32,
    boundary_class_counts: [u32; BoundaryClass::ALL.len()],
}

impl TectonicsDiagnostics {
    /// Border edges the final classification put in `class`.
    pub const fn count(&self, class: BoundaryClass) -> u32 {
        self.boundary_class_counts[class as usize]
    }

    /// Share of the raster's border edges the final classification put in
    /// `class`.
    pub fn boundary_fraction(&self, class: BoundaryClass) -> f32 {
        let total: u32 = self.boundary_class_counts.iter().sum();
        if total == 0 {
            return 0.0;
        }
        self.count(class) as f32 / total as f32
    }
}

/// What one run cost and produced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TectonicsRun {
    pub timings: StageTimings,
    pub diagnostics: TectonicsDiagnostics,
    /// Frontier passes each growth relaxation was allowed.
    pub pass_budget: u32,
}

impl TectonicsRun {
    /// Whether both growth relaxations reached their fixed point. A run that
    /// spends its whole budget stopped short of one, and its ownership is not
    /// the partition the configuration describes.
    pub const fn settled(&self) -> bool {
        self.diagnostics.longest_relaxation_passes < self.pass_budget
    }
}

/// The GPU-resident tectonics pipeline.
pub struct TectonicsPipeline {
    resolution: u32,
    cell_count: u32,
    tuning: PipelineTuning,
    kernels: TectonicsKernels,
    bind_group: wgpu::BindGroup,
    config_buffer: wgpu::Buffer,
    state_buffer: wgpu::Buffer,
    growth_labels: wgpu::Buffer,
    ownership: wgpu::Buffer,
    boundary_classes: wgpu::Buffer,
    plates: wgpu::Buffer,
    diagnostics: wgpu::Buffer,
}

struct TectonicsKernels {
    initialize: wgpu::ComputePipeline,
    begin_frontier: wgpu::ComputePipeline,
    seed_reduce: wgpu::ComputePipeline,
    seed_select: wgpu::ComputePipeline,
    seed_place: wgpu::ComputePipeline,
    prepare_relax: wgpu::ComputePipeline,
    relax: wgpu::ComputePipeline,
    derive_ownership: wgpu::ComputePipeline,
    reduce_plate_areas: wgpu::ComputePipeline,
    classify_crust: wgpu::ComputePipeline,
    begin_boundaries: wgpu::ComputePipeline,
    classify_boundaries: wgpu::ComputePipeline,
    propose_migration: wgpu::ComputePipeline,
    apply_migration: wgpu::ComputePipeline,
}

impl TectonicsPipeline {
    /// Compiles the kernels and allocates every stage buffer for one face
    /// resolution. Changing the resolution builds a new pipeline.
    ///
    /// The device must meet `wgpu`'s default limits;
    /// [`crate::device::validate_device`] states which of them the kernels
    /// actually depend on.
    pub fn new(
        device: &wgpu::Device,
        resolution: u32,
        tuning: PipelineTuning,
    ) -> Result<Self, RasterTectonicsError> {
        let cell_count = validate_resolution(resolution)?;
        tuning.validate()?;
        validate_device(device, tuning, frontier_size(cell_count))?;

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raster tectonics"),
            source: wgpu::ShaderSource::Wgsl(tectonics_kernel_source(tuning).into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raster tectonics"),
            entries: &bind_group_layout_entries(),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raster tectonics"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let kernel = |entry_point: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry_point),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        let kernels = TectonicsKernels {
            initialize: kernel("initialize"),
            begin_frontier: kernel("begin_frontier"),
            seed_reduce: kernel("seed_reduce"),
            seed_select: kernel("seed_select"),
            seed_place: kernel("seed_place"),
            prepare_relax: kernel("prepare_relax"),
            relax: kernel("relax"),
            derive_ownership: kernel("derive_ownership"),
            reduce_plate_areas: kernel("reduce_plate_areas"),
            classify_crust: kernel("classify_crust"),
            begin_boundaries: kernel("begin_boundaries"),
            classify_boundaries: kernel("classify_boundaries"),
            propose_migration: kernel("propose_migration"),
            apply_migration: kernel("apply_migration"),
        };

        let cells = u64::from(cell_count) * size_of::<u32>() as u64;
        let storage = wgpu::BufferUsages::STORAGE;
        let config_buffer = buffer(
            device,
            "raster tectonics config",
            size_of::<PackedTectonicsConfig>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let state_buffer = buffer(
            device,
            "raster tectonics state",
            size_of::<PackedTectonicsState>() as u64,
            storage | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC,
        );
        let diagnostics = buffer(
            device,
            "raster tectonics diagnostics",
            size_of::<PackedDiagnostics>() as u64,
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        );
        let growth_labels = buffer(
            device,
            "raster growth labels",
            cells,
            storage | wgpu::BufferUsages::COPY_SRC,
        );
        let queued_pass = buffer(device, "raster frontier queue marks", cells, storage);
        let frontier = buffer(
            device,
            "raster frontier",
            frontier_size(cell_count),
            storage,
        );
        let seed_distance = buffer(device, "raster farthest-point distance", cells, storage);
        let ownership = buffer(
            device,
            "raster cell ownership",
            cells,
            storage | wgpu::BufferUsages::COPY_SRC,
        );
        let boundary_classes = buffer(
            device,
            "raster boundary classes",
            cells,
            storage | wgpu::BufferUsages::COPY_SRC,
        );
        let plates = buffer(
            device,
            "raster plates",
            u64::from(PLATE_ID_COUNT) * size_of::<RasterPlate>() as u64,
            storage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        );
        let resources = [
            config_buffer.as_entire_binding(),
            state_buffer.as_entire_binding(),
            growth_labels.as_entire_binding(),
            queued_pass.as_entire_binding(),
            frontier.as_entire_binding(),
            seed_distance.as_entire_binding(),
            ownership.as_entire_binding(),
            boundary_classes.as_entire_binding(),
            plates.as_entire_binding(),
        ];
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raster tectonics"),
            layout: &layout,
            entries: &std::array::from_fn::<_, BINDING_COUNT, _>(|binding| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: resources[binding].clone(),
            }),
        });

        Ok(Self {
            resolution,
            cell_count,
            tuning,
            kernels,
            bind_group,
            config_buffer,
            state_buffer,
            growth_labels,
            ownership,
            boundary_classes,
            plates,
            diagnostics,
        })
    }

    pub const fn resolution(&self) -> u32 {
        self.resolution
    }

    pub const fn cell_count(&self) -> u32 {
        self.cell_count
    }

    pub const fn tuning(&self) -> PipelineTuning {
        self.tuning
    }

    /// One `u32` per cell holding the packed `(arrival cost, plate)` growth
    /// label the partition settled on, before any migration.
    pub const fn growth_label_buffer(&self) -> &wgpu::Buffer {
        &self.growth_labels
    }

    /// One `u32` per cell holding its current and pending plate, as
    /// [`crate::plate_ownership`] packs them.
    pub const fn ownership_buffer(&self) -> &wgpu::Buffer {
        &self.ownership
    }

    /// One `u32` per cell holding the class of each of its four borders, as
    /// [`crate::boundary_class`] reads them.
    pub const fn boundary_class_buffer(&self) -> &wgpu::Buffer {
        &self.boundary_classes
    }

    /// One [`RasterPlate`] per addressable plate id, including the reserved id
    /// no plate uses.
    pub const fn plate_buffer(&self) -> &wgpu::Buffer {
        &self.plates
    }

    /// Runs the pipeline from a clean state and returns its timings and
    /// diagnostics.
    pub fn run(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: &RasterTectonicsConfig,
    ) -> Result<TectonicsRun, RasterTectonicsError> {
        // Everything that can fail happens before anything is written, so a
        // configuration the pipeline refuses leaves the resident buffers as the
        // last accepted run left them.
        config.validate(self.resolution)?;
        let plate_count = config.partition.plate_count();
        let plates = config.evolution.plate_records(plate_count)?;
        queue.write_buffer(
            &self.config_buffer,
            0,
            bytemuck::bytes_of(&PackedTectonicsConfig::new(
                &config.partition,
                &config.evolution,
                self.resolution,
                self.cell_count,
            )),
        );
        queue.write_buffer(&self.plates, 0, bytemuck::cast_slice(&plates));

        let mut timings = StageTimings::default();
        self.submit(
            device,
            queue,
            &mut timings,
            PipelineStage::Initialize,
            |pass, kernels, workgroups| {
                pass.set_pipeline(&kernels.initialize);
                pass.dispatch_workgroups(workgroups, 1, 1);
            },
        );
        self.seed_round(
            device,
            queue,
            &mut timings,
            0..config.partition.major_plate_count,
        );
        self.grow(device, queue, &mut timings);
        self.seed_round(
            device,
            queue,
            &mut timings,
            config.partition.major_plate_count..plate_count,
        );
        self.grow(device, queue, &mut timings);
        self.classify_crust(device, queue, &mut timings);
        for _ in 0..config.evolution.step_count {
            self.classify_boundaries(device, queue, &mut timings);
            self.migrate(device, queue, &mut timings);
        }
        self.classify_boundaries(device, queue, &mut timings);

        Ok(TectonicsRun {
            timings,
            diagnostics: self.read_diagnostics(device, queue),
            pass_budget: growth_passes(self.resolution),
        })
    }

    /// Places one seed per plate in `plates`, each on the eligible cell
    /// farthest from every seed already placed. Plate zero takes the
    /// configuration's cell, so it needs no reduction.
    fn seed_round(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        timings: &mut StageTimings,
        plates: std::ops::Range<u32>,
    ) {
        self.submit(
            device,
            queue,
            timings,
            PipelineStage::PlateSeeds,
            |pass, kernels, _| {
                pass.set_pipeline(&kernels.begin_frontier);
                pass.dispatch_workgroups(1, 1, 1);
                for plate in plates {
                    if plate > 0 {
                        pass.set_pipeline(&kernels.seed_reduce);
                        pass.dispatch_workgroups(SEED_REDUCTION_WORKGROUPS, 1, 1);
                        pass.set_pipeline(&kernels.seed_select);
                        pass.dispatch_workgroups(1, 1, 1);
                    }
                    pass.set_pipeline(&kernels.seed_place);
                    pass.dispatch_workgroups(1, 1, 1);
                }
            },
        );
    }

    /// Relaxes the frontier until it empties. Passes past convergence read an
    /// empty frontier and dispatch no workgroups, so the fixed budget costs
    /// only its dispatches and needs no readback to stop.
    fn grow(&self, device: &wgpu::Device, queue: &wgpu::Queue, timings: &mut StageTimings) {
        let passes = growth_passes(self.resolution);
        self.submit(
            device,
            queue,
            timings,
            PipelineStage::PlateGrowth,
            |pass, kernels, _| {
                for _ in 0..passes {
                    pass.set_pipeline(&kernels.prepare_relax);
                    pass.dispatch_workgroups(1, 1, 1);
                    pass.set_pipeline(&kernels.relax);
                    pass.dispatch_workgroups_indirect(
                        &self.state_buffer,
                        offset_of!(PackedTectonicsState, relax_dispatch) as u64,
                    );
                }
            },
        );
    }

    /// Lifts the settled labels into ownership and decides the crust classes
    /// the plate areas imply. Deriving ownership belongs to this submission
    /// because the area reduction is its first reader.
    fn classify_crust(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        timings: &mut StageTimings,
    ) {
        self.submit(
            device,
            queue,
            timings,
            PipelineStage::CrustClassification,
            |pass, kernels, workgroups| {
                pass.set_pipeline(&kernels.derive_ownership);
                pass.dispatch_workgroups(workgroups, 1, 1);
                pass.set_pipeline(&kernels.reduce_plate_areas);
                pass.dispatch_workgroups(workgroups, 1, 1);
                pass.set_pipeline(&kernels.classify_crust);
                pass.dispatch_workgroups(1, 1, 1);
            },
        );
    }

    fn classify_boundaries(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        timings: &mut StageTimings,
    ) {
        self.submit(
            device,
            queue,
            timings,
            PipelineStage::BoundaryClassification,
            |pass, kernels, workgroups| {
                pass.set_pipeline(&kernels.begin_boundaries);
                pass.dispatch_workgroups(1, 1, 1);
                pass.set_pipeline(&kernels.classify_boundaries);
                pass.dispatch_workgroups(workgroups, 1, 1);
            },
        );
    }

    fn migrate(&self, device: &wgpu::Device, queue: &wgpu::Queue, timings: &mut StageTimings) {
        self.submit(
            device,
            queue,
            timings,
            PipelineStage::PlateMigration,
            |pass, kernels, workgroups| {
                pass.set_pipeline(&kernels.propose_migration);
                pass.dispatch_workgroups(workgroups, 1, 1);
                pass.set_pipeline(&kernels.apply_migration);
                pass.dispatch_workgroups(workgroups, 1, 1);
            },
        );
    }

    /// Copies the counters the kernels accumulated and derives the fractions
    /// the host reports from them.
    fn read_diagnostics(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> TectonicsDiagnostics {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("raster tectonics diagnostics"),
        });
        encoder.copy_buffer_to_buffer(
            &self.state_buffer,
            offset_of!(PackedTectonicsState, diagnostics) as u64,
            &self.diagnostics,
            0,
            size_of::<PackedDiagnostics>() as u64,
        );
        queue.submit([encoder.finish()]);

        let slice = self.diagnostics.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            sender
                .send(result)
                .expect("the diagnostic receiver outlives the map")
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the tectonics device must accept a blocking poll");
        receiver
            .recv()
            .expect("the diagnostic map must report a result")
            .expect("the diagnostic buffer must map for reading");
        let packed = *bytemuck::from_bytes::<PackedDiagnostics>(&slice.get_mapped_range());
        self.diagnostics.unmap();

        TectonicsDiagnostics {
            longest_relaxation_passes: packed.longest_relaxation,
            ocean_fraction: if packed.total_area == 0 {
                0.0
            } else {
                packed.ocean_area as f32 / packed.total_area as f32
            },
            empty_plate_count: packed.empty_plate_count,
            migrated_cell_count: packed.migrated_cell_count,
            boundary_class_counts: packed.boundary_class_counts,
        }
    }

    fn submit(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        timings: &mut StageTimings,
        stage: PipelineStage,
        record: impl FnOnce(&mut wgpu::ComputePass<'_>, &TectonicsKernels, u32),
    ) {
        let started = Instant::now();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some(stage.label()),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(stage.label()),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            record(
                &mut pass,
                &self.kernels,
                self.tuning.cell_workgroups(self.cell_count),
            );
        }
        queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the tectonics device must accept a blocking poll");
        timings.record(stage, started.elapsed());
    }
}

fn bind_group_layout_entries() -> [wgpu::BindGroupLayoutEntry; BINDING_COUNT] {
    std::array::from_fn(|binding| wgpu::BindGroupLayoutEntry {
        binding: binding as u32,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: if binding == 0 {
                wgpu::BufferBindingType::Uniform
            } else {
                wgpu::BufferBindingType::Storage { read_only: false }
            },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    })
}

fn buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timings_accumulate_per_stage() {
        let mut timings = StageTimings::default();
        timings.record(PipelineStage::PlateGrowth, Duration::from_millis(3));
        timings.record(PipelineStage::PlateGrowth, Duration::from_millis(4));
        assert_eq!(
            timings.duration(PipelineStage::PlateGrowth),
            Duration::from_millis(7)
        );
        assert_eq!(timings.total(), Duration::from_millis(7));
    }

    #[test]
    fn boundary_fractions_share_out_every_classified_border() {
        let diagnostics = TectonicsDiagnostics {
            longest_relaxation_passes: 0,
            ocean_fraction: 0.0,
            empty_plate_count: 0,
            migrated_cell_count: 0,
            boundary_class_counts: [70, 10, 15, 5],
        };
        assert_eq!(diagnostics.count(BoundaryClass::Divergent), 15);
        assert_eq!(diagnostics.boundary_fraction(BoundaryClass::Interior), 0.7);
        let total: f32 = BoundaryClass::ALL
            .into_iter()
            .map(|class| diagnostics.boundary_fraction(class))
            .sum();
        assert!((total - 1.0).abs() < f32::EPSILON);
    }
}
