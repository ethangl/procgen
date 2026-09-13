//! Headless checks over one source chunk, a neighboring face, and its parent.
use crate::{
    ChunkIndex, PlanetDesignField, VOXEL_CHUNK_CELLS, VOXEL_HALO, VOXEL_SAMPLE_COUNT,
    VoxelAddressError, VoxelChunkAddress, VoxelPosition, VoxelSampleIndex, VoxelVolumeError,
    sample_voxel_chunk,
};
use serde::Serialize;
use std::{error::Error, fmt};

pub struct VoxelAuditConfig {
    pub point: VoxelPosition,
    pub lod: u8,
}
#[derive(Debug)]
pub enum VoxelAuditError {
    Address(VoxelAddressError),
    Volume(VoxelVolumeError),
}
impl From<VoxelAddressError> for VoxelAuditError {
    fn from(e: VoxelAddressError) -> Self {
        Self::Address(e)
    }
}
impl From<VoxelVolumeError> for VoxelAuditError {
    fn from(e: VoxelVolumeError) -> Self {
        Self::Volume(e)
    }
}
impl fmt::Display for VoxelAuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address(e) => e.fmt(f),
            Self::Volume(e) => e.fmt(f),
        }
    }
}
impl Error for VoxelAuditError {}

#[derive(Serialize)]
pub struct VoxelChunkAudit {
    pub root: VoxelChunkAddress,
    pub address: VoxelChunkAddress,
    pub children: Option<[VoxelChunkAddress; 8]>,
    pub origin_m: VoxelPosition,
    pub anchor_local_m: VoxelPosition,
    pub spacing_m: i32,
    pub span_m: i32,
    pub samples: usize,
    pub density_bytes: usize,
    pub max_paired_density_bytes: usize,
    pub density_min_m: f32,
    pub density_max_m: f32,
    pub solid_samples: usize,
    pub shared_face_and_halo_samples: usize,
    pub max_shared_density_difference_m: f32,
    pub shared_parent_samples: usize,
    pub max_parent_density_difference_m: f32,
}

pub fn audit_voxel_chunk(
    field: &PlanetDesignField,
    config: VoxelAuditConfig,
) -> Result<VoxelChunkAudit, VoxelAuditError> {
    let address = VoxelChunkAddress::containing(config.point, config.lod)?;
    let volume = sample_voxel_chunk(field, address)?;
    let density_min_m = volume
        .samples()
        .map(|(_, d)| d)
        .fold(f32::INFINITY, f32::min);
    let density_max_m = volume
        .samples()
        .map(|(_, d)| d)
        .fold(f32::NEG_INFINITY, f32::max);
    let solid_samples = volume.samples().filter(|(_, d)| *d >= 0.0).count();
    let mut result = VoxelChunkAudit {
        root: VoxelChunkAddress::root(),
        address,
        children: address.children(),
        origin_m: address.origin(),
        anchor_local_m: config.point.relative_to(address.origin()),
        spacing_m: address.spacing_m(),
        span_m: address.span_m(),
        samples: VOXEL_SAMPLE_COUNT,
        density_bytes: volume.allocated_bytes(),
        max_paired_density_bytes: volume.allocated_bytes(),
        density_min_m,
        density_max_m,
        solid_samples,
        shared_face_and_halo_samples: 0,
        max_shared_density_difference_m: 0.0,
        shared_parent_samples: 0,
        max_parent_density_difference_m: 0.0,
    };
    // Prefer +X; use -X at the root boundary. The root itself has no neighbor.
    let index = address.index();
    let neighbor = match VoxelChunkAddress::new(
        address.lod(),
        ChunkIndex {
            x: index.x + 1,
            ..index
        },
    ) {
        Ok(a) => Some((a, VOXEL_CHUNK_CELLS)),
        Err(VoxelAddressError::Index) if index.x > 0 => Some((
            VoxelChunkAddress::new(
                address.lod(),
                ChunkIndex {
                    x: index.x - 1,
                    ..index
                },
            )?,
            -VOXEL_CHUNK_CELLS,
        )),
        Err(VoxelAddressError::Index) => None,
        Err(e) => return Err(e.into()),
    };
    if let Some((neighbor, shift)) = neighbor {
        let other = sample_voxel_chunk(field, neighbor)?;
        result.max_paired_density_bytes = result
            .max_paired_density_bytes
            .max(volume.allocated_bytes() + other.allocated_bytes());
        let boundary = if shift > 0 { VOXEL_CHUNK_CELLS } else { 0 };
        for z in -VOXEL_HALO..=VOXEL_CHUNK_CELLS + VOXEL_HALO {
            for y in -VOXEL_HALO..=VOXEL_CHUNK_CELLS + VOXEL_HALO {
                for x in boundary - VOXEL_HALO..=boundary + VOXEL_HALO {
                    let here = VoxelSampleIndex { x, y, z };
                    let there = VoxelSampleIndex { x: x - shift, y, z };
                    result.max_shared_density_difference_m = result
                        .max_shared_density_difference_m
                        .max((volume.density(here) - other.density(there)).abs());
                    result.shared_face_and_halo_samples += 1;
                }
            }
        }
    }
    // The neighboring payload has been released before allocating the parent.
    if let Some(parent) = address.parent() {
        let coarse = sample_voxel_chunk(field, parent)?;
        result.max_paired_density_bytes = result
            .max_paired_density_bytes
            .max(volume.allocated_bytes() + coarse.allocated_bytes());
        let half = VOXEL_CHUNK_CELLS / 2;
        for z in (0..=VOXEL_CHUNK_CELLS).step_by(2) {
            for y in (0..=VOXEL_CHUNK_CELLS).step_by(2) {
                for x in (0..=VOXEL_CHUNK_CELLS).step_by(2) {
                    let parent_sample = VoxelSampleIndex {
                        x: (index.x % 2) as i32 * half + x / 2,
                        y: (index.y % 2) as i32 * half + y / 2,
                        z: (index.z % 2) as i32 * half + z / 2,
                    };
                    result.max_parent_density_difference_m =
                        result.max_parent_density_difference_m.max(
                            (volume.density(VoxelSampleIndex { x, y, z })
                                - coarse.density(parent_sample))
                            .abs(),
                        );
                    result.shared_parent_samples += 1;
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PlanetDesignConfig, VOXEL_DENSITY_BYTES, VOXEL_ROOT_LOD};
    #[test]
    fn audit_has_constant_payload_and_checks_real_boundary_samples() {
        for radius in [100_000.0, 8_000_000.0] {
            let mut config = PlanetDesignConfig::starter(42);
            config.radius_m = radius;
            config.octaves.iter_mut().for_each(|o| o.enabled = false);
            let field = config.validate().unwrap();
            let result = audit_voxel_chunk(
                &field,
                VoxelAuditConfig {
                    point: VoxelPosition {
                        x_m: radius as i32,
                        y_m: 0,
                        z_m: 0,
                    },
                    lod: 0,
                },
            )
            .unwrap();
            assert_eq!(result.max_paired_density_bytes, 2 * VOXEL_DENSITY_BYTES);
            assert_eq!(result.shared_face_and_halo_samples, 3675);
            assert_eq!(result.shared_parent_samples, 4913);
            assert_eq!(result.max_shared_density_difference_m, 0.0);
            assert_eq!(result.max_parent_density_difference_m, 0.0);
            assert!(result.density_min_m < 0.0 && result.density_max_m > 0.0);
        }
        let field = PlanetDesignConfig::starter(42).validate().unwrap();
        let root = audit_voxel_chunk(
            &field,
            VoxelAuditConfig {
                point: VoxelPosition {
                    x_m: 0,
                    y_m: 0,
                    z_m: 0,
                },
                lod: VOXEL_ROOT_LOD,
            },
        )
        .unwrap();
        assert_eq!(root.max_paired_density_bytes, VOXEL_DENSITY_BYTES);
        assert_eq!(root.shared_face_and_halo_samples, 0);
        assert_eq!(root.shared_parent_samples, 0);
    }
}
