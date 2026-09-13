//! Canonical full-band density samples at integer meter positions.
use crate::{
    PlanetDesignField, VOXEL_SAMPLE_COUNT, VoxelChunkAddress, VoxelPosition, VoxelSampleIndex,
};
use procgen_core::Vec3;
use rayon::prelude::*;
use std::{
    error::Error,
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

/// Positive solid, negative air. This is a clamped density, not a signed distance.
pub const VOXEL_DENSITY_LIMIT_M: f32 = 4.0;
pub const VOXEL_DENSITY_BYTES: usize = VOXEL_SAMPLE_COUNT * std::mem::size_of::<f32>();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoxelVolumeError;
impl fmt::Display for VoxelVolumeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "voxel volume must contain {VOXEL_SAMPLE_COUNT} finite potentials (density access clamps to +/-{VOXEL_DENSITY_LIMIT_M} meters)"
        )
    }
}
impl Error for VoxelVolumeError {}

pub struct VoxelVolume {
    address: VoxelChunkAddress,
    potentials: Vec<f32>,
}
impl VoxelVolume {
    #[cfg(test)]
    pub(crate) fn fixture(
        address: VoxelChunkAddress,
        potential: impl Fn(VoxelPosition) -> f32,
    ) -> Self {
        Self {
            address,
            potentials: (0..VOXEL_SAMPLE_COUNT)
                .map(|i| potential(address.sample_position(VoxelSampleIndex::from_linear(i))))
                .collect(),
        }
    }

    pub fn address(&self) -> VoxelChunkAddress {
        self.address
    }
    pub fn allocated_bytes(&self) -> usize {
        self.potentials.capacity() * std::mem::size_of::<f32>()
    }
    pub fn validate(&self) -> Result<(), VoxelVolumeError> {
        if self.potentials.len() != VOXEL_SAMPLE_COUNT
            || self.potentials.iter().any(|d| !d.is_finite())
        {
            return Err(VoxelVolumeError);
        }
        Ok(())
    }
    pub fn density(&self, index: VoxelSampleIndex) -> f32 {
        self.potential(index)
            .clamp(-VOXEL_DENSITY_LIMIT_M, VOXEL_DENSITY_LIMIT_M)
    }
    /// Unsaturated potential preserves edge interpolation at coarse spacing.
    pub(crate) fn potential(&self, index: VoxelSampleIndex) -> f32 {
        self.potentials[index.linear()]
    }
    pub fn samples(&self) -> impl Iterator<Item = (VoxelPosition, f32)> + '_ {
        self.potentials.iter().enumerate().map(|(i, &density)| {
            (
                self.address
                    .sample_position(VoxelSampleIndex::from_linear(i)),
                density.clamp(-VOXEL_DENSITY_LIMIT_M, VOXEL_DENSITY_LIMIT_M),
            )
        })
    }
}

pub fn sample_voxel_chunk(
    field: &PlanetDesignField,
    address: VoxelChunkAddress,
) -> Result<VoxelVolume, VoxelVolumeError> {
    Ok(
        sample_voxel_chunk_cancellable(field, address, &AtomicBool::new(false))?
            .expect("uncancelled sample"),
    )
}

/// Each parallel row checks cancellation before evaluating more noise. Allocate
/// the final buffer once so the worker reservation includes all density storage.
pub(crate) fn sample_voxel_chunk_cancellable(
    field: &PlanetDesignField,
    address: VoxelChunkAddress,
    cancel: &AtomicBool,
) -> Result<Option<VoxelVolume>, VoxelVolumeError> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let mut potentials = vec![0.0; VOXEL_SAMPLE_COUNT];
    potentials
        .par_chunks_mut(crate::VOXEL_SAMPLE_SIDE)
        .enumerate()
        .for_each(|(row, samples)| {
            if !cancel.load(Ordering::Relaxed) {
                for (column, sample) in samples.iter_mut().enumerate() {
                    let i = row * crate::VOXEL_SAMPLE_SIDE + column;
                    *sample = potential_at(
                        field,
                        address.sample_position(VoxelSampleIndex::from_linear(i)),
                    );
                }
            }
        });
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let result = VoxelVolume {
        address,
        potentials,
    };
    result.validate()?;
    Ok(Some(result))
}

fn potential_at(field: &PlanetDesignField, position: VoxelPosition) -> f32 {
    // All root and halo coordinates fit exactly in f32 at their LOD spacing.
    let p = position.as_vec3();
    let radius = field.config().radius_m;
    let altitude = radial_altitude(p, radius);
    // The center is strictly interior and has no surface direction. Everywhere
    // else retain the unsaturated potential for coarse edge interpolation.
    let height = if p == Vec3::ZERO {
        0.0
    } else {
        field.height(p.normalized(), 0.0)
    };
    height - altitude
}

/// Avoid subtracting two rounded planet radii. Compensated squared products
/// retain the small radial residual; division uses the well-conditioned sum.
/// Uses only f32 and ordinary operations so a future WGSL mirror can match it.
fn radial_altitude(p: Vec3, radius: f32) -> f32 {
    let difference = square(p.x)
        .add(square(p.y))
        .add(square(p.z))
        .add(square(radius).neg());
    (difference.high + difference.low) / (p.length() + radius)
}
#[derive(Clone, Copy)]
struct Compensated {
    high: f32,
    low: f32,
}
impl Compensated {
    fn add(self, rhs: Self) -> Self {
        let sum = self.high + rhs.high;
        let virtual_rhs = sum - self.high;
        let error = (self.high - (sum - virtual_rhs)) + (rhs.high - virtual_rhs);
        let tail = error + (self.low + rhs.low);
        let high = sum + tail;
        Self {
            high,
            low: tail - (high - sum),
        }
    }
    fn neg(self) -> Self {
        Self {
            high: -self.high,
            low: -self.low,
        }
    }
}
fn square(value: f32) -> Compensated {
    // Dekker split at half of f32's significand. No FMA or f64 requirement.
    let split = 4097.0 * value;
    let high = split - (split - value);
    let low = value - high;
    let product = value * value;
    let error = ((high * high - product) + high * low + low * high) + low * low;
    Compensated {
        high: product,
        low: error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkIndex, PlanetDesignConfig};
    #[test]
    fn meter_altitudes_survive_large_radii_and_oblique_positions() {
        // f64 is an independent test oracle only. Two millimeters covers f32
        // denominator error across the supported +/-20 km relief envelope.
        for radius in [100_000.0, 2_000_000.0, 8_000_000.0] {
            for direction in [
                Vec3::X,
                Vec3::new(1.0, 1.0, 0.0).normalized(),
                Vec3::new(1.0, 1.0, 1.0).normalized(),
            ] {
                for offset in [-20_000.0, -1.0, 0.0, 1.0, 20_000.0] {
                    let p = direction * (radius + offset);
                    let expected =
                        ((p.x as f64).powi(2) + (p.y as f64).powi(2) + (p.z as f64).powi(2)).sqrt()
                            - radius as f64;
                    assert!(
                        (radial_altitude(p, radius) as f64 - expected).abs() < 0.002,
                        "radius={radius} offset={offset} expected={expected} got={}",
                        radial_altitude(p, radius)
                    );
                }
            }
        }
        let mut config = PlanetDesignConfig::starter(42);
        config.radius_m = 8_000_000.0;
        config.octaves.iter_mut().for_each(|o| o.enabled = false);
        let field = config.validate().unwrap();
        for offset in -3..=3 {
            assert_eq!(
                potential_at(
                    &field,
                    VoxelPosition {
                        x_m: 8_000_000 + offset,
                        y_m: 0,
                        z_m: 0
                    }
                ),
                -offset as f32
            );
        }
    }
    #[test]
    fn shared_boundary_halos_and_coarse_nodes_have_identical_density() {
        let field = PlanetDesignConfig::starter(42).validate().unwrap();
        let a = VoxelChunkAddress::containing(
            VoxelPosition {
                x_m: 2_000_000,
                y_m: 0,
                z_m: 0,
            },
            0,
        )
        .unwrap();
        let ai = a.index();
        let left = sample_voxel_chunk(&field, a).unwrap();
        let right = sample_voxel_chunk(
            &field,
            VoxelChunkAddress::new(0, ChunkIndex { x: ai.x + 1, ..ai }).unwrap(),
        )
        .unwrap();
        for z in -1..=33 {
            for y in -1..=33 {
                for x in 31..=33 {
                    assert_eq!(
                        left.density(VoxelSampleIndex { x, y, z }),
                        right.density(VoxelSampleIndex { x: x - 32, y, z })
                    );
                }
            }
        }
        let coarse = sample_voxel_chunk(&field, a.parent().unwrap()).unwrap();
        for z in (0..=32).step_by(2) {
            for y in (0..=32).step_by(2) {
                for x in (0..=32).step_by(2) {
                    let parent_index = VoxelSampleIndex {
                        x: (ai.x % 2) as i32 * 16 + x / 2,
                        y: (ai.y % 2) as i32 * 16 + y / 2,
                        z: (ai.z % 2) as i32 * 16 + z / 2,
                    };
                    assert_eq!(
                        left.density(VoxelSampleIndex { x, y, z }),
                        coarse.density(parent_index)
                    );
                }
            }
        }
    }
    #[test]
    fn sampling_is_repeatable_across_schedules_and_validates_its_storage() {
        let field = PlanetDesignConfig::starter(4_294_967_338)
            .validate()
            .unwrap();
        let address = VoxelChunkAddress::containing(
            VoxelPosition {
                x_m: 1_414_208,
                y_m: 1_414_208,
                z_m: 0,
            },
            0,
        )
        .unwrap();
        let run = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| sample_voxel_chunk(&field, address).unwrap())
        };
        let mut a = run(1);
        assert_eq!(a.potentials, run(4).potentials);
        assert_eq!(a.allocated_bytes(), VOXEL_DENSITY_BYTES);
        a.potentials[5] = f32::NAN;
        assert!(a.validate().is_err());
        a.potentials.clear();
        assert!(a.validate().is_err());
    }
}
