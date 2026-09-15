//! Canonical full-band density samples at integer meter positions.
use crate::noise::{normalized_noise, shape};
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
pub(crate) const SQRT_SEED: u32 = 0x5f375a86;
pub(crate) const SQRT_ITERATIONS: u32 = 5;

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
    /// Construct a sampled volume, including GPU audit readback, in halo-grid order.
    pub fn from_potentials(
        address: VoxelChunkAddress,
        potentials: Vec<f32>,
    ) -> Result<Self, VoxelVolumeError> {
        let result = Self {
            address,
            potentials,
        };
        result.validate()?;
        Ok(result)
    }
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
    pub fn potential(&self, index: VoxelSampleIndex) -> f32 {
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

/// One canonical potential at one integer meter position: the same value
/// `sample_voxel_chunk` stores and the WGSL kernel computes, for callers that
/// need a handful of points rather than a whole chunk.
pub fn sample_voxel_potential(field: &PlanetDesignField, position: VoxelPosition) -> f32 {
    potential_at(field, position)
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

pub(crate) fn potential_at(field: &PlanetDesignField, position: VoxelPosition) -> f32 {
    // All root and halo coordinates fit exactly in f32 at their LOD spacing.
    let p = position.as_vec3();
    let radius = field.config().radius_m;
    let distance = voxel_sqrt(p.length_squared());
    let altitude = radial_altitude(p, radius, distance);
    // The center is strictly interior and has no surface direction. Everywhere
    // else retain the unsaturated potential for coarse edge interpolation.
    let height = if p == Vec3::ZERO {
        0.0
    } else {
        field.height(p * distance.recip(), 0.0)
    };
    let d = height - altitude;
    d + volume_at(field, p, d)
}

/// The surface-relative 3D detail term. `d` is positive below the surface, so
/// the fade is one inside the solid and reaches zero `fade_m` above it: nothing
/// this term adds can float free in the air more than that far up.
///
/// The shaped noise is bounded by the design's own `x / sqrt(1 + x*x)` before
/// the amplitude scales it, the same soft bound the height stack applies to its
/// accumulated relief. The shape transform has unit deviation, not a unit
/// bound, so without this the amplitude would be a one-sigma scale and the term
/// would reach several times it - measured at -3.3 m to +26.0 m for a 6 m
/// amplitude at sharpness 0.5, and the other way round for a negative one. The
/// render bias in `local_draw_layout` assumes the voxel surface sits at most
/// `amplitude_m` below the height surface, so that has to be a true bound and
/// not a statistical one, or the height surface shows through undercuts.
///
/// There is deliberately no spacing filter here, unlike the height octaves. The
/// density path always evaluates the full stack, and coincident halo and parent
/// samples must stay bit-identical across LODs, which a spacing-dependent term
/// would break and open seams between chunk levels. Coarse chunks alias this
/// wavelength instead; the wavelength is bounded from below by validation.
fn volume_at(field: &PlanetDesignField, p: Vec3, d: f32) -> f32 {
    let volume = &field.config().volume;
    if !volume.enabled || volume.amplitude_m == 0.0 {
        return 0.0;
    }
    let mut fade = 1.0;
    if d < 0.0 {
        if d <= -volume.fade_m {
            return 0.0;
        }
        let t = ((d + volume.fade_m) / volume.fade_m).clamp(0.0, 1.0);
        fade = t * t * (3.0 - 2.0 * t);
    }
    let q = Vec3::new(
        p.x / volume.wavelength_m,
        p.y / volume.wavelength_m,
        p.z / volume.wavelength_m,
    );
    let shaped = shape(normalized_noise(field.volume_key(), q), volume.sharpness).value;
    volume.amplitude_m * (shaped / (1.0 + shaped * shaped).sqrt()) * fade
}

/// Avoid subtracting two rounded planet radii. Compensated squared products
/// retain the small radial residual; division uses the well-conditioned sum.
/// Uses only f32; the WGSL mirror preserves the same operation boundaries.
fn radial_altitude(p: Vec3, radius: f32, distance: f32) -> f32 {
    let difference = square(p.x)
        .add(square(p.y))
        .add(square(p.z))
        .add(square(radius).neg());
    (difference.high + difference.low) / (distance + radius)
}

/// Fixed polynomial iteration shared with WGSL: no backend sqrt intrinsic
/// determines the direction that selects integer noise lattice cells.
fn voxel_sqrt(value: f32) -> f32 {
    if value == 0.0 {
        return 0.0;
    }
    let mut inverse = f32::from_bits(SQRT_SEED - (value.to_bits() >> 1));
    for _ in 0..SQRT_ITERATIONS {
        inverse *= 1.5 - ((0.5 * value) * inverse) * inverse;
    }
    let root = value * inverse;
    (-root.mul_add(root, -value)).mul_add(0.5 * inverse, root)
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
    fn polynomial_root_matches_independent_oracle_across_voxel_coordinate_scales() {
        assert_eq!(voxel_sqrt(0.0), 0.0);
        for exponent in 0..=48 {
            for fraction in 0..128 {
                let value = 2.0_f32.powi(exponent) * (1.0 + fraction as f32 / 128.0);
                let expected = (value as f64).sqrt();
                // One f32 relative epsilon covers final rounding; the f64
                // oracle is independent of the fixed polynomial iteration.
                assert!(
                    (voxel_sqrt(value) as f64 - expected).abs() <= f32::EPSILON as f64 * expected
                );
                assert_eq!(voxel_sqrt(value * 4.0), voxel_sqrt(value) * 2.0);
            }
        }
    }

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
                        (radial_altitude(p, radius, voxel_sqrt(p.length_squared())) as f64
                            - expected)
                            .abs()
                            < 0.002,
                        "radius={radius} offset={offset} expected={expected} got={}",
                        radial_altitude(p, radius, voxel_sqrt(p.length_squared()))
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
    /// The height-only formula this term is added to, evaluated independently
    /// of `potential_at` so a change to one is not a change to both.
    fn height_potential(field: &PlanetDesignField, position: VoxelPosition) -> f32 {
        let p = position.as_vec3();
        let distance = voxel_sqrt(p.length_squared());
        let height = if p == Vec3::ZERO {
            0.0
        } else {
            field.height(p * distance.recip(), 0.0)
        };
        height - radial_altitude(p, field.config().radius_m, distance)
    }
    fn enabled_volume_config(seed: u64) -> PlanetDesignConfig {
        PlanetDesignConfig {
            volume: crate::VolumeConfig {
                enabled: true,
                ..Default::default()
            },
            ..PlanetDesignConfig::starter(seed)
        }
    }

    #[test]
    fn the_volume_term_is_inert_when_disabled_and_fades_out_above_the_surface() {
        let disabled = PlanetDesignConfig::starter(42).validate().unwrap();
        let enabled = enabled_volume_config(42).validate().unwrap();
        let volume = &enabled.config().volume;
        assert_eq!((volume.amplitude_m, volume.fade_m), (6.0, 12.0));
        let radius = disabled.config().radius_m as i32;
        for address in [
            VoxelChunkAddress::containing(
                VoxelPosition {
                    x_m: radius,
                    y_m: 0,
                    z_m: 0,
                },
                0,
            )
            .unwrap(),
            VoxelChunkAddress::root(),
        ] {
            let chunk = sample_voxel_chunk(&disabled, address).unwrap();
            for (i, (p, _)) in chunk.samples().enumerate() {
                // Disabled must reproduce the old formula bit for bit.
                assert_eq!(
                    chunk.potential(VoxelSampleIndex::from_linear(i)),
                    height_potential(&disabled, p),
                    "{p:?}"
                );
            }
        }

        // Well above the surface the fade is exactly zero, so the enabled
        // potential is exactly the height-only potential.
        let direction = Vec3::new(0.3, -1.0, 0.45).normalized();
        let surface = radius as f32 + enabled.elevation_m(direction, 0.0).unwrap();
        let point = |offset: f32| {
            let p = direction * (surface + offset);
            VoxelPosition {
                x_m: p.x.round() as i32,
                y_m: p.y.round() as i32,
                z_m: p.z.round() as i32,
            }
        };
        let mut above = 0;
        for offset in [20.0, 40.0, 200.0, 5_000.0] {
            let p = point(offset);
            let d = height_potential(&enabled, p);
            assert!(d <= -volume.fade_m, "{offset} m up gave d={d}");
            assert_eq!(potential_at(&enabled, p), d, "{p:?}");
            above += 1;
        }
        assert_eq!(above, 4);

        // Below the surface the fade is one, so the difference is exactly the
        // soft-bounded shaped noise times the amplitude, and it is never zero.
        let mut changed = 0;
        for offset in [-1.0, -4.0, -16.0, -64.0, -256.0] {
            let p = point(offset);
            let d = height_potential(&enabled, p);
            assert!(d >= 0.0, "{offset} m down gave d={d}");
            let q = p.as_vec3();
            let q = Vec3::new(
                q.x / volume.wavelength_m,
                q.y / volume.wavelength_m,
                q.z / volume.wavelength_m,
            );
            let shaped = shape(normalized_noise(enabled.volume_key(), q), volume.sharpness).value;
            let expected = volume.amplitude_m * (shaped / (1.0 + shaped * shaped).sqrt());
            // Compared as the sum the kernel forms, not as a difference: the
            // rounding of `d + volume` is part of the potential.
            assert_eq!(potential_at(&enabled, p), d + expected, "{p:?}");
            assert!(expected.abs() < volume.amplitude_m, "{expected} m");
            changed += usize::from(expected != 0.0);
        }
        assert_eq!(changed, 5);
    }

    #[test]
    fn the_volume_term_stays_strictly_inside_its_amplitude() {
        // The render bias treats amplitude_m as the deepest the voxel surface
        // can sit below the height surface, so the bound has to hold over the
        // whole shaped range and at both signs of sharpness, not just on
        // average. Sharpness -1 is the branch whose long tail is the undercut
        // side, which is the case the bias rule cannot survive without this.
        for sharpness in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            let mut config = enabled_volume_config(42);
            config.volume.sharpness = sharpness;
            config.volume.amplitude_m = 64.0;
            let amplitude = config.volume.amplitude_m;
            let field = config.validate().unwrap();
            let flat = PlanetDesignConfig {
                volume: crate::VolumeConfig {
                    enabled: false,
                    ..config.volume.clone()
                },
                ..config
            }
            .validate()
            .unwrap();
            let (mut low, mut high) = (0.0_f32, 0.0_f32);
            for direction in [
                Vec3::X,
                Vec3::new(0.2, -1.0, 0.35).normalized(),
                Vec3::new(-0.6, 0.4, 1.0).normalized(),
            ] {
                let p = direction
                    * (field.config().radius_m + field.elevation_m(direction, 0.0).unwrap());
                let address = VoxelChunkAddress::containing(
                    VoxelPosition {
                        x_m: p.x.round() as i32,
                        y_m: p.y.round() as i32,
                        z_m: p.z.round() as i32,
                    },
                    0,
                )
                .unwrap();
                let with = sample_voxel_chunk(&field, address).unwrap();
                let without = sample_voxel_chunk(&flat, address).unwrap();
                for i in 0..VOXEL_SAMPLE_COUNT {
                    let index = VoxelSampleIndex::from_linear(i);
                    let term = with.potential(index) - without.potential(index);
                    low = low.min(term);
                    high = high.max(term);
                }
            }
            println!("sharpness {sharpness}: term spans {low:.3} m to {high:.3} m of {amplitude}");
            assert!(low > -amplitude && high < amplitude, "{low} .. {high}");
            // A bound that is never approached would be a useless bias budget.
            assert!(
                low < -0.2 * amplitude && high > 0.2 * amplitude,
                "{low} .. {high}"
            );
        }
    }

    #[test]
    fn the_volume_term_keeps_shared_samples_identical_across_levels() {
        // The term has no spacing filter, so this must hold with it enabled
        // exactly as it does without it: the halo and parent agreement is what
        // keeps chunk levels from seaming.
        let field = enabled_volume_config(42).validate().unwrap();
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
        let fine = sample_voxel_chunk(&field, a).unwrap();
        let coarse = sample_voxel_chunk(&field, a.parent().unwrap()).unwrap();
        let mut compared = 0;
        for z in (0..=32).step_by(2) {
            for y in (0..=32).step_by(2) {
                for x in (0..=32).step_by(2) {
                    assert_eq!(
                        fine.potential(VoxelSampleIndex { x, y, z }),
                        coarse.potential(VoxelSampleIndex {
                            x: (ai.x % 2) as i32 * 16 + x / 2,
                            y: (ai.y % 2) as i32 * 16 + y / 2,
                            z: (ai.z % 2) as i32 * 16 + z / 2,
                        })
                    );
                    compared += 1;
                }
            }
        }
        assert_eq!(compared, 17_usize.pow(3));
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
        assert_eq!(
            a.allocated_bytes(),
            VOXEL_SAMPLE_COUNT * std::mem::size_of::<f32>()
        );
        a.potentials[5] = f32::NAN;
        assert!(a.validate().is_err());
        a.potentials.clear();
        assert!(a.validate().is_err());

        // The volume term must not depend on the rayon schedule either, and it
        // has to be sampled where it is actually active: a chunk entirely in
        // the air is past the fade and would agree trivially.
        let with_volume = enabled_volume_config(4_294_967_338).validate().unwrap();
        let direction = Vec3::new(1.0, 1.0, 0.0).normalized();
        let p = direction
            * (with_volume.config().radius_m + with_volume.elevation_m(direction, 0.0).unwrap());
        let surface = VoxelChunkAddress::containing(
            VoxelPosition {
                x_m: p.x.round() as i32,
                y_m: p.y.round() as i32,
                z_m: p.z.round() as i32,
            },
            0,
        )
        .unwrap();
        let run = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| sample_voxel_chunk(&with_volume, surface).unwrap())
        };
        let one = run(1);
        assert_eq!(one.potentials, run(4).potentials);
        assert_ne!(
            one.potentials,
            sample_voxel_chunk(&field, surface).unwrap().potentials,
            "the enabled term must change a surface chunk"
        );
    }
}
