//! Optional device timestamps, separate from CPU preparation and queue receipt time.
//! Use pass boundaries: encoder timestamps returned zero intervals and stale output on Metal.
pub(crate) const TIMING_BYTES: u64 = 32;
#[derive(Clone, Copy, Debug)]
pub struct VoxelGpuTimes {
    pub density_ms: f64,
    pub extraction_ms: f64,
}
pub(crate) struct GpuTiming {
    pub queries: wgpu::QuerySet,
    pub resolved: wgpu::Buffer,
}
impl GpuTiming {
    pub fn new(device: &wgpu::Device) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        Some(Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("voxel stage times"),
                ty: wgpu::QueryType::Timestamp,
                count: 4,
            }),
            resolved: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("voxel timestamps"),
                size: TIMING_BYTES,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
        })
    }
    pub fn times(data: &[u8], period_ns: f32) -> VoxelGpuTimes {
        let ticks: &[u64] = bytemuck::cast_slice(data);
        VoxelGpuTimes {
            density_ms: ticks[1].wrapping_sub(ticks[0]) as f64 * period_ns as f64 / 1_000_000.0,
            extraction_ms: ticks[3].wrapping_sub(ticks[2]) as f64 * period_ns as f64 / 1_000_000.0,
        }
    }
}
