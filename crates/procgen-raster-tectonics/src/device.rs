//! What the pipeline needs of a device, and the crate's one failure
//! convention.
//!
//! The dispatch shape, the resolution ceiling, the bind-group budget, and every
//! way a configuration can be refused live here, so no stage module carries
//! part of the contract with `wgpu`.

use crate::field::MAX_PLATE_COUNT;
use procgen_cubesphere::{FaceTexel, RasterError};
use std::fmt;

/// Largest face resolution the pilot runs, and the cap the cost budget assumes.
pub const MAX_TECTONIC_RESOLUTION: u32 = 1_024;

/// Workgroups one dispatch may cover, from `wgpu`'s default device limits.
pub(crate) const MAX_DISPATCH_WORKGROUPS: u32 = 65_535;

/// Bind-group entries every kernel shares: one uniform block of configuration
/// and the stage buffers behind it.
pub(crate) const BINDING_COUNT: usize = 9;
/// Storage bindings among those, which the device must supply to one stage.
///
/// This is `wgpu`'s default limit exactly. A stage that needs another buffer
/// has to consolidate two of these rather than add a tenth binding.
pub(crate) const STORAGE_BINDING_COUNT: u32 = BINDING_COUNT as u32 - 1;

/// Refuses a device that cannot run the kernels, rather than letting it fail
/// inside `wgpu`.
///
/// `frontier_bytes` is the largest binding the pipeline makes and therefore the
/// one a device is most likely to refuse; the caller sizes it, so this module
/// stays independent of the stages.
pub(crate) fn validate_device(
    device: &wgpu::Device,
    tuning: PipelineTuning,
    frontier_bytes: u64,
) -> Result<(), RasterTectonicsError> {
    let limits = device.limits();
    if limits.max_storage_buffers_per_shader_stage < STORAGE_BINDING_COUNT
        || limits.max_compute_invocations_per_workgroup < tuning.workgroup_size
        || limits.max_compute_workgroup_size_x < tuning.workgroup_size
        || limits.max_compute_workgroups_per_dimension < MAX_DISPATCH_WORKGROUPS
        || u64::from(limits.max_storage_buffer_binding_size) < frontier_bytes
        || limits.max_buffer_size < frontier_bytes
    {
        return Err(RasterTectonicsError::UnsupportedDevice);
    }
    Ok(())
}

/// Returns the cell count of a face resolution the pilot can run.
pub(crate) fn validate_resolution(resolution: u32) -> Result<u32, RasterTectonicsError> {
    let cell_count = FaceTexel::cell_count(resolution)?;
    if resolution > MAX_TECTONIC_RESOLUTION {
        return Err(RasterTectonicsError::UnsupportedResolution);
    }
    Ok(cell_count)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterTectonicsError {
    Resolution(RasterError),
    UnsupportedResolution,
    UnsupportedDevice,
    NoMajorPlates,
    TooManyPlates,
    InvalidGrowthRoughness,
    InvalidHeadStartArc,
    InvalidOceanFraction,
    InvalidAngularSpeedRange,
    InvalidMinimumConvergence,
    InvalidWorkgroupSize,
    InvalidFrontierChunk,
}

impl From<RasterError> for RasterTectonicsError {
    fn from(error: RasterError) -> Self {
        Self::Resolution(error)
    }
}

impl fmt::Display for RasterTectonicsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolution(error) => error.fmt(formatter),
            Self::UnsupportedResolution => write!(
                formatter,
                "raster tectonics runs at face resolutions up to {MAX_TECTONIC_RESOLUTION}"
            ),
            Self::UnsupportedDevice => write!(
                formatter,
                "the device must meet wgpu's default limits: {STORAGE_BINDING_COUNT} storage \
                 bindings per stage, {} invocations per workgroup, {MAX_DISPATCH_WORKGROUPS} \
                 workgroups per dispatch, and a storage binding holding the frontier at the \
                 requested face resolution",
                PipelineTuning::MAX_WORKGROUP_SIZE
            ),
            Self::NoMajorPlates => formatter.write_str("at least one major plate is required"),
            Self::TooManyPlates => write!(
                formatter,
                "plate count cannot exceed the cell count or {MAX_PLATE_COUNT}"
            ),
            Self::InvalidGrowthRoughness => write!(
                formatter,
                "plate growth roughness cannot exceed {}%",
                procgen_tectonics::MAX_GROWTH_ROUGHNESS
            ),
            Self::InvalidHeadStartArc => {
                formatter.write_str("the major head start must be an arc between 0 and PI radians")
            }
            Self::InvalidOceanFraction => {
                formatter.write_str("target ocean fraction must be finite and between 0 and 1")
            }
            Self::InvalidAngularSpeedRange => formatter.write_str(
                "angular speeds must be finite, non-negative, and ordered minimum to maximum",
            ),
            Self::InvalidMinimumConvergence => {
                formatter.write_str("minimum convergence must be finite and non-negative")
            }
            Self::InvalidWorkgroupSize => write!(
                formatter,
                "the workgroup size must be a power of two of at most {} invocations",
                PipelineTuning::MAX_WORKGROUP_SIZE
            ),
            Self::InvalidFrontierChunk => {
                formatter.write_str("the frontier chunk must relax at least one entry")
            }
        }
    }
}

impl std::error::Error for RasterTectonicsError {}

/// Dispatch shape the kernels are compiled for.
///
/// Neither field changes any result: the frontier is order-independent, the
/// farthest-point reduction is an exact minimum, and every other kernel is a
/// per-cell map or gather. They exist so the pipeline tests can assert that by
/// running the same seed at different shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipelineTuning {
    /// Invocations per workgroup. Must be a power of two of at most 256.
    pub workgroup_size: u32,
    /// Frontier entries one invocation relaxes per pass, which sizes the
    /// indirect dispatch and the stride of every per-cell kernel.
    pub frontier_chunk: u32,
}

impl Default for PipelineTuning {
    fn default() -> Self {
        Self {
            workgroup_size: 64,
            frontier_chunk: 4,
        }
    }
}

impl PipelineTuning {
    /// Largest workgroup size `wgpu`'s default device limits allow.
    pub const MAX_WORKGROUP_SIZE: u32 = 256;

    pub(crate) fn validate(&self) -> Result<(), RasterTectonicsError> {
        if self.workgroup_size == 0
            || self.workgroup_size > Self::MAX_WORKGROUP_SIZE
            || !self.workgroup_size.is_power_of_two()
        {
            return Err(RasterTectonicsError::InvalidWorkgroupSize);
        }
        if self.frontier_chunk == 0 {
            return Err(RasterTectonicsError::InvalidFrontierChunk);
        }
        Ok(())
    }

    /// Workgroups a kernel that strides over every cell dispatches.
    pub(crate) fn cell_workgroups(&self, cell_count: u32) -> u32 {
        cell_count
            .div_ceil(self.workgroup_size * self.frontier_chunk)
            .clamp(1, MAX_DISPATCH_WORKGROUPS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dispatch_shapes_the_kernels_cannot_compile() {
        assert_eq!(
            PipelineTuning {
                workgroup_size: 48,
                frontier_chunk: 1,
            }
            .validate(),
            Err(RasterTectonicsError::InvalidWorkgroupSize)
        );
        assert_eq!(
            PipelineTuning {
                workgroup_size: 512,
                frontier_chunk: 1,
            }
            .validate(),
            Err(RasterTectonicsError::InvalidWorkgroupSize)
        );
        assert_eq!(
            PipelineTuning {
                workgroup_size: 64,
                frontier_chunk: 0,
            }
            .validate(),
            Err(RasterTectonicsError::InvalidFrontierChunk)
        );
    }

    #[test]
    fn cell_dispatches_stay_inside_the_device_workgroup_limit() {
        let tuning = PipelineTuning {
            workgroup_size: 64,
            frontier_chunk: 1,
        };
        let cells = FaceTexel::cell_count(MAX_TECTONIC_RESOLUTION).unwrap();
        assert_eq!(tuning.cell_workgroups(cells), MAX_DISPATCH_WORKGROUPS);
        assert_eq!(tuning.cell_workgroups(1), 1);
    }

    #[test]
    fn rejects_resolutions_the_kernels_cannot_address() {
        assert_eq!(
            validate_resolution(3),
            Err(RasterTectonicsError::Resolution(
                RasterError::InvalidResolution
            ))
        );
        assert_eq!(
            validate_resolution(2_048),
            Err(RasterTectonicsError::UnsupportedResolution)
        );
        assert_eq!(validate_resolution(64), Ok(6 * 64 * 64));
    }
}
