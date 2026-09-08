//! wgpu dispatch for the raster plate partition.
//!
//! This module holds no per-cell logic: it allocates the stage buffers, packs
//! the configuration, and sequences the kernels [`crate::partition`] defines.
//! Buffers stay resident between runs so a settings change reruns the stages
//! rather than reallocating them.

use crate::partition::{
    BINDING_COUNT, MAX_DISPATCH_WORKGROUPS, MAX_PLATE_COUNT, PackedPartitionConfig,
    PackedPartitionState, PipelineTuning, RasterPartitionError, RasterPlatePartitionConfig,
    SEED_CANDIDATE_SIZE, SEED_REDUCTION_WORKGROUPS, STORAGE_BINDING_COUNT, frontier_size,
    growth_passes, partition_kernel_source, validate_resolution,
};
use std::{
    mem::offset_of,
    sync::mpsc,
    time::{Duration, Instant},
};

/// The stages a partition run reports separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineStage {
    /// Clearing the resident buffers for a new run.
    Initialize,
    /// Farthest-point seed selection for every plate.
    PlateSeeds,
    /// Both frontier relaxations: majors alone, then majors with minors.
    PlateGrowth,
}

impl PipelineStage {
    pub const ALL: [Self; 3] = [Self::Initialize, Self::PlateSeeds, Self::PlateGrowth];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Initialize => "Initialize",
            Self::PlateSeeds => "Plate seeds",
            Self::PlateGrowth => "Plate growth",
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

/// What one partition run cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartitionRun {
    pub timings: StageTimings,
    /// Frontier passes the longer of the two relaxations consumed. The
    /// frontier's order varies run to run, so this varies by a pass or two
    /// while the labels it settles on do not. Read it as a cost, not a result.
    pub longest_relaxation_passes: u32,
    /// Frontier passes each relaxation was allowed.
    pub pass_budget: u32,
}

impl PartitionRun {
    /// Whether both relaxations reached their fixed point. A run that spends
    /// its whole budget stopped short of one, and its ownership is not the
    /// partition the configuration describes.
    pub const fn settled(&self) -> bool {
        self.longest_relaxation_passes < self.pass_budget
    }
}

/// The GPU-resident plate partition.
pub struct PlatePartitionPipeline {
    resolution: u32,
    cell_count: u32,
    tuning: PipelineTuning,
    kernels: PartitionKernels,
    bind_group: wgpu::BindGroup,
    config_buffer: wgpu::Buffer,
    state_buffer: wgpu::Buffer,
    growth_labels: wgpu::Buffer,
    plate_seed_cells: wgpu::Buffer,
    diagnostics: wgpu::Buffer,
}

struct PartitionKernels {
    initialize: wgpu::ComputePipeline,
    begin_frontier: wgpu::ComputePipeline,
    seed_reduce: wgpu::ComputePipeline,
    seed_select: wgpu::ComputePipeline,
    seed_place: wgpu::ComputePipeline,
    prepare_relax: wgpu::ComputePipeline,
    relax: wgpu::ComputePipeline,
}

impl PlatePartitionPipeline {
    /// Compiles the kernels and allocates every stage buffer for one face
    /// resolution. Changing the resolution builds a new pipeline.
    ///
    /// The device must meet `wgpu`'s default limits; [`validate_device`] states
    /// which of them the kernels actually depend on.
    pub fn new(
        device: &wgpu::Device,
        resolution: u32,
        tuning: PipelineTuning,
    ) -> Result<Self, RasterPartitionError> {
        let cell_count = validate_resolution(resolution)?;
        tuning.validate()?;
        validate_device(device, cell_count, tuning)?;

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raster plate partition"),
            source: wgpu::ShaderSource::Wgsl(partition_kernel_source(tuning).into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raster plate partition"),
            entries: &bind_group_layout_entries(),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raster plate partition"),
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
        let kernels = PartitionKernels {
            initialize: kernel("initialize"),
            begin_frontier: kernel("begin_frontier"),
            seed_reduce: kernel("seed_reduce"),
            seed_select: kernel("seed_select"),
            seed_place: kernel("seed_place"),
            prepare_relax: kernel("prepare_relax"),
            relax: kernel("relax"),
        };

        let cells = u64::from(cell_count) * size_of::<u32>() as u64;
        let storage = wgpu::BufferUsages::STORAGE;
        let config_buffer = buffer(
            device,
            "raster partition config",
            size_of::<PackedPartitionConfig>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let state_buffer = buffer(
            device,
            "raster partition state",
            size_of::<PackedPartitionState>() as u64,
            storage | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC,
        );
        let diagnostics = buffer(
            device,
            "raster partition diagnostics",
            size_of::<u32>() as u64,
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
        let seed_partials = buffer(
            device,
            "raster seed candidates",
            u64::from(SEED_REDUCTION_WORKGROUPS) * SEED_CANDIDATE_SIZE,
            storage,
        );
        let plate_seed_cells = buffer(
            device,
            "raster plate seed cells",
            u64::from(MAX_PLATE_COUNT) * size_of::<u32>() as u64,
            storage | wgpu::BufferUsages::COPY_SRC,
        );
        let resources = [
            config_buffer.as_entire_binding(),
            state_buffer.as_entire_binding(),
            growth_labels.as_entire_binding(),
            queued_pass.as_entire_binding(),
            frontier.as_entire_binding(),
            seed_distance.as_entire_binding(),
            seed_partials.as_entire_binding(),
            plate_seed_cells.as_entire_binding(),
        ];
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raster plate partition"),
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
            plate_seed_cells,
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
    /// label. Cells no plate reached carry
    /// [`crate::UNCLAIMED_LABEL`](crate::UNCLAIMED_LABEL).
    pub const fn growth_label_buffer(&self) -> &wgpu::Buffer {
        &self.growth_labels
    }

    /// One `u32` per plate identity holding the cell its seed was placed on, or
    /// [`procgen_cubesphere::NO_RASTER_CELL`] when the head start left no
    /// eligible cell and the plate stayed empty.
    pub const fn plate_seed_buffer(&self) -> &wgpu::Buffer {
        &self.plate_seed_cells
    }

    /// Runs the partition from a clean state and returns per-stage timings.
    pub fn run(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: &RasterPlatePartitionConfig,
    ) -> Result<PartitionRun, RasterPartitionError> {
        config.validate(self.resolution)?;
        queue.write_buffer(
            &self.config_buffer,
            0,
            bytemuck::bytes_of(&PackedPartitionConfig::new(
                config,
                self.resolution,
                self.cell_count,
            )),
        );

        let cell_workgroups = self.tuning.cell_workgroups(self.cell_count);
        let mut timings = StageTimings::default();
        self.submit(
            device,
            queue,
            &mut timings,
            PipelineStage::Initialize,
            |pass| {
                pass.set_pipeline(&self.kernels.initialize);
                pass.dispatch_workgroups(cell_workgroups, 1, 1);
            },
        );
        self.seed_round(device, queue, &mut timings, 0..config.major_plate_count);
        self.grow(device, queue, &mut timings);
        self.seed_round(
            device,
            queue,
            &mut timings,
            config.major_plate_count..config.plate_count(),
        );
        self.grow(device, queue, &mut timings);
        Ok(PartitionRun {
            timings,
            longest_relaxation_passes: self.read_longest_relaxation(device, queue),
            pass_budget: growth_passes(self.resolution),
        })
    }

    /// Copies the one diagnostic the host reads: how close the longer
    /// relaxation came to exhausting its pass budget.
    fn read_longest_relaxation(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> u32 {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("raster partition diagnostics"),
        });
        encoder.copy_buffer_to_buffer(
            &self.state_buffer,
            offset_of!(PackedPartitionState, longest_relaxation) as u64,
            &self.diagnostics,
            0,
            size_of::<u32>() as u64,
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
            .expect("the partition device must accept a blocking poll");
        receiver
            .recv()
            .expect("the diagnostic map must report a result")
            .expect("the diagnostic buffer must map for reading");
        let passes = u32::from_ne_bytes(
            slice.get_mapped_range()[..size_of::<u32>()]
                .try_into()
                .expect("the diagnostic buffer holds one u32"),
        );
        self.diagnostics.unmap();
        passes
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
        self.submit(device, queue, timings, PipelineStage::PlateSeeds, |pass| {
            pass.set_pipeline(&self.kernels.begin_frontier);
            pass.dispatch_workgroups(1, 1, 1);
            for plate in plates {
                if plate > 0 {
                    pass.set_pipeline(&self.kernels.seed_reduce);
                    pass.dispatch_workgroups(SEED_REDUCTION_WORKGROUPS, 1, 1);
                    pass.set_pipeline(&self.kernels.seed_select);
                    pass.dispatch_workgroups(1, 1, 1);
                }
                pass.set_pipeline(&self.kernels.seed_place);
                pass.dispatch_workgroups(1, 1, 1);
            }
        });
    }

    /// Relaxes the frontier until it empties. Passes past convergence read an
    /// empty frontier and dispatch no workgroups, so the fixed budget costs
    /// only its dispatches and needs no readback to stop.
    fn grow(&self, device: &wgpu::Device, queue: &wgpu::Queue, timings: &mut StageTimings) {
        let passes = growth_passes(self.resolution);
        self.submit(device, queue, timings, PipelineStage::PlateGrowth, |pass| {
            for _ in 0..passes {
                pass.set_pipeline(&self.kernels.prepare_relax);
                pass.dispatch_workgroups(1, 1, 1);
                pass.set_pipeline(&self.kernels.relax);
                pass.dispatch_workgroups_indirect(
                    &self.state_buffer,
                    offset_of!(PackedPartitionState, relax_dispatch) as u64,
                );
            }
        });
    }

    fn submit(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        timings: &mut StageTimings,
        stage: PipelineStage,
        record: impl FnOnce(&mut wgpu::ComputePass<'_>),
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
            record(&mut pass);
        }
        queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the partition device must accept a blocking poll");
        timings.record(stage, started.elapsed());
    }
}

/// The limits the kernels depend on, checked so a device that cannot run them
/// is refused rather than failing inside `wgpu`.
fn validate_device(
    device: &wgpu::Device,
    cell_count: u32,
    tuning: PipelineTuning,
) -> Result<(), RasterPartitionError> {
    let limits = device.limits();
    let frontier = frontier_size(cell_count);
    if limits.max_storage_buffers_per_shader_stage < STORAGE_BINDING_COUNT
        || limits.max_compute_invocations_per_workgroup < tuning.workgroup_size
        || limits.max_compute_workgroup_size_x < tuning.workgroup_size
        || limits.max_compute_workgroups_per_dimension < MAX_DISPATCH_WORKGROUPS
        || u64::from(limits.max_storage_buffer_binding_size) < frontier
        || limits.max_buffer_size < frontier
    {
        return Err(RasterPartitionError::UnsupportedDevice);
    }
    Ok(())
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
}
