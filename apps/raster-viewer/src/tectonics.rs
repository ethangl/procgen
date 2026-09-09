//! Runs the raster tectonics pipeline on Bevy's own device.
//!
//! The pipeline is not yet interactive, so a run is submitted stage by stage
//! and waited on, which is what makes per-stage timings measurable without
//! timestamp queries. Slice 6 of the pilot moves dispatch into a render-graph
//! node with an asynchronous readback.

use bevy::{
    prelude::*,
    render::{
        RenderApp,
        renderer::{RenderDevice, RenderQueue},
    },
};
use procgen_raster_tectonics::{
    MAX_PLATE_COUNT, MAX_TECTONIC_RESOLUTION, PipelineTuning, RASTER_SPHERE_RADIUS,
    RasterTectonicsConfig, TectonicsPipeline, TectonicsRun,
};
use procgen_tectonics::MAX_GROWTH_ROUGHNESS;

/// Face resolutions the viewer offers. The pilot's target is the largest;
/// until the interactivity slice the default is a resolution that reruns fast
/// enough to explore.
pub const FACE_RESOLUTIONS: [u32; 4] = [128, 256, 512, MAX_TECTONIC_RESOLUTION];
pub const DEFAULT_FACE_RESOLUTION: u32 = 256;

/// Inclusive slider bounds the UI reads rather than restating.
pub const MAJOR_PLATE_RANGE: std::ops::RangeInclusive<u32> = 1..=64;
pub const MINOR_PLATE_RANGE: std::ops::RangeInclusive<u32> = 0..=256;
pub const HEAD_START_ARC_RANGE: std::ops::RangeInclusive<f32> = 0.0..=0.5;
pub const GROWTH_ROUGHNESS_RANGE: std::ops::RangeInclusive<u32> = 0..=MAX_GROWTH_ROUGHNESS;
pub const OCEAN_FRACTION_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.0;
pub const EVOLUTION_STEP_RANGE: std::ops::RangeInclusive<u32> = 0..=32;

const _: () = assert!(*MAJOR_PLATE_RANGE.end() + *MINOR_PLATE_RANGE.end() <= MAX_PLATE_COUNT);

/// Everything the viewer can change about a run.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct TectonicsSettings {
    pub resolution: u32,
    pub config: RasterTectonicsConfig,
}

impl Default for TectonicsSettings {
    fn default() -> Self {
        Self {
            resolution: DEFAULT_FACE_RESOLUTION,
            config: RasterTectonicsConfig::default(),
        }
    }
}

impl TectonicsSettings {
    /// Upper bound on the convergence a migration step can see, which is what
    /// makes the migration threshold a meaningful fraction rather than a
    /// number without a scale.
    pub fn maximum_convergence(&self) -> f32 {
        self.config
            .evolution
            .kinematics
            .maximum_convergence(RASTER_SPHERE_RADIUS)
    }
}

/// Bevy's device, lifted into the main world so the pipeline can be submitted
/// and waited on outside the render graph.
#[derive(Resource)]
struct DeviceAccess {
    device: RenderDevice,
    queue: RenderQueue,
}

/// The resident pipeline and what its last run cost and produced.
#[derive(Resource)]
pub struct ResidentTectonics {
    pipeline: TectonicsPipeline,
    run: TectonicsRun,
}

impl ResidentTectonics {
    pub const fn pipeline(&self) -> &TectonicsPipeline {
        &self.pipeline
    }

    pub const fn run(&self) -> &TectonicsRun {
        &self.run
    }
}

pub struct TectonicsPlugin;

impl Plugin for TectonicsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TectonicsSettings>()
            .add_systems(Update, regenerate);
    }

    fn finish(&self, app: &mut App) {
        let render_world = app.sub_app(RenderApp).world();
        let access = DeviceAccess {
            device: render_world.resource::<RenderDevice>().clone(),
            queue: render_world.resource::<RenderQueue>().clone(),
        };
        app.insert_resource(access);
    }
}

/// Reruns the pipeline once the settings stop changing, so dragging a slider
/// does not queue one blocking run per frame.
fn regenerate(
    mut commands: Commands,
    settings: Res<TectonicsSettings>,
    access: Res<DeviceAccess>,
    resident: Option<ResMut<ResidentTectonics>>,
    mut pending: Local<bool>,
) {
    if settings.is_changed() {
        *pending = true;
        return;
    }
    if !*pending {
        return;
    }
    *pending = false;

    // The exported ranges keep every reachable setting inside the pipeline's
    // contract, and Bevy has already secured a device meeting wgpu's default
    // limits, so a rejection here means those guarantees and the contract
    // disagree.
    const VALID: &str = "the viewer's settings ranges and Bevy's device meet the pipeline contract";
    let device = access.device.wgpu_device();
    match resident.filter(|resident| resident.pipeline.resolution() == settings.resolution) {
        Some(mut resident) => {
            let run = resident
                .pipeline
                .run(device, &access.queue, &settings.config)
                .expect(VALID);
            resident.run = run;
        }
        None => {
            let pipeline =
                TectonicsPipeline::new(device, settings.resolution, PipelineTuning::default())
                    .expect(VALID);
            let run = pipeline
                .run(device, &access.queue, &settings.config)
                .expect(VALID);
            commands.insert_resource(ResidentTectonics { pipeline, run });
        }
    }
}
