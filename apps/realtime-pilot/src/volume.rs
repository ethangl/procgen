use procgen_core::Vec3;
use rayon::prelude::*;

use crate::{
    field::{FieldError, MAX_GRID_SIDE, MIN_GRID_SIDE, validate_position},
    terrain::TerrainField,
};

/// Raster geometry for the local diagnostic, separate from terrain parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeGrid {
    pub min: Vec3,
    pub max: Vec3,
    pub side: usize,
}

pub const INSPECTION_GRID: VolumeGrid = VolumeGrid {
    min: Vec3::new(-1.0, -1.0, -1.0),
    max: Vec3::new(1.0, 1.0, 1.0),
    side: 64,
};

impl VolumeGrid {
    pub fn validate(self) -> Result<(), FieldError> {
        validate_position(self.min)?;
        validate_position(self.max)?;
        if !(MIN_GRID_SIDE..=MAX_GRID_SIDE).contains(&self.side) || !self.side.is_power_of_two() {
            return Err(FieldError::GridSide);
        }
        if self.min.x >= self.max.x || self.min.y >= self.max.y || self.min.z >= self.max.z {
            return Err(FieldError::GridBounds);
        }
        Ok(())
    }

    /// Coordinates include both ends of the diagnostic box.
    pub(crate) fn position(self, x: usize, y: usize, z: usize) -> Vec3 {
        let scale = 1.0 / (self.side - 1) as f32;
        Vec3::new(
            self.min.x + (self.max.x - self.min.x) * x as f32 * scale,
            self.min.y + (self.max.y - self.min.y) * y as f32 * scale,
            self.min.z + (self.max.z - self.min.z) * z as f32 * scale,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

/// Samples own their shape validation; consumers use slices without index encodings.
pub struct Volume {
    grid: VolumeGrid,
    heights: Vec<f32>,
    densities: Vec<f32>,
}

impl Volume {
    pub fn grid(&self) -> VolumeGrid {
        self.grid
    }
    pub fn heights(&self) -> &[f32] {
        &self.heights
    }

    pub fn validate(&self) -> Result<(), FieldError> {
        self.grid.validate()?;
        if self.heights.len() != self.grid.side.pow(2)
            || self.densities.len() != self.grid.side.pow(3)
            || !self
                .heights
                .iter()
                .chain(&self.densities)
                .all(|v| v.is_finite())
        {
            return Err(FieldError::Volume);
        }
        Ok(())
    }

    /// Image rows ascend the second displayed axis: Y for X/Z cuts, Z for Y cuts.
    pub fn slice(&self, axis: Axis, index: usize) -> Result<Vec<f32>, FieldError> {
        let side = self.grid.side;
        if index >= side {
            return Err(FieldError::SliceIndex { index, side });
        }
        Ok((0..side)
            .flat_map(|row| {
                (0..side).map(move |col| {
                    let (x, y, z) = match axis {
                        Axis::X => (index, row, col),
                        Axis::Y => (col, index, row),
                        Axis::Z => (col, row, index),
                    };
                    self.densities[(z * side + x) * side + y]
                })
            })
            .collect())
    }
}

struct SampledColumn {
    height: f32,
    densities: Vec<f32>,
}

/// Parallel columns reuse surface and cave candidates across Y samples.
pub fn sample_volume(field: &TerrainField, grid: VolumeGrid) -> Result<Volume, FieldError> {
    grid.validate()?;
    let sample_column = |index: usize| {
        let p = grid.position(index % grid.side, 0, index / grid.side);
        let column = field.column(p.x, p.z);
        let density: Vec<_> = (0..grid.side)
            .map(|y| column.density(field, grid.position(0, y, 0).y))
            .collect();
        SampledColumn {
            height: column.height,
            densities: density,
        }
    };
    let rows: Vec<_> = if grid.side >= 16 {
        (0..grid.side.pow(2))
            .into_par_iter()
            .map(sample_column)
            .collect()
    } else {
        (0..grid.side.pow(2)).map(sample_column).collect()
    };
    let mut heights = Vec::with_capacity(rows.len());
    let mut densities = Vec::with_capacity(grid.side.pow(3));
    for column in rows {
        heights.push(column.height);
        densities.extend(column.densities);
    }
    let volume = Volume {
        grid,
        heights,
        densities,
    };
    volume.validate()?;
    Ok(volume)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PRESETS;
    use procgen_core::quantized_fingerprint;

    #[test]
    fn parallel_schedules_and_direct_queries_agree() {
        let grid = VolumeGrid {
            side: 16,
            ..INSPECTION_GRID
        };
        let field = PRESETS[1].config.validate(42).unwrap();
        let run = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| sample_volume(&field, grid).unwrap())
        };
        let one = run(1);
        let four = run(4);
        assert_eq!(one.densities, four.densities);
        assert_eq!(one.heights, four.heights);
        for axis in [Axis::X, Axis::Y, Axis::Z] {
            let cut = one.slice(axis, 7).unwrap();
            for row in 0..grid.side {
                for col in 0..grid.side {
                    let (x, y, z) = match axis {
                        Axis::X => (7, row, col),
                        Axis::Y => (col, 7, row),
                        Axis::Z => (col, row, 7),
                    };
                    assert_eq!(
                        cut[row * grid.side + col],
                        field.density(grid.position(x, y, z)).unwrap()
                    );
                }
            }
        }
    }

    #[test]
    fn presets_have_pinned_distinct_fields() {
        let actual: Vec<_> = PRESETS
            .iter()
            .map(|preset| {
                let field = preset.config.validate(42).unwrap();
                let volume = sample_volume(
                    &field,
                    VolumeGrid {
                        side: 16,
                        ..INSPECTION_GRID
                    },
                )
                .unwrap();
                quantized_fingerprint(volume.densities)
            })
            .collect();
        // Initial slice-1 CPU fields, quantized on the shared 1/1024 grid.
        assert_eq!(
            actual,
            vec![
                3_284_531_776_164_537_981,
                5_992_316_526_205_022_298,
                12_776_664_651_359_129_979
            ]
        );
    }

    #[test]
    fn invalid_grid_and_corrupt_volume_fail_at_the_owner() {
        let field = PRESETS[0].config.validate(0).unwrap();
        assert!(
            sample_volume(
                &field,
                VolumeGrid {
                    side: usize::MAX,
                    ..INSPECTION_GRID
                }
            )
            .is_err()
        );
        assert!(
            sample_volume(
                &field,
                VolumeGrid {
                    min: Vec3::X * 2.0,
                    ..INSPECTION_GRID
                }
            )
            .is_err()
        );
        let mut volume = sample_volume(
            &field,
            VolumeGrid {
                side: 2,
                ..INSPECTION_GRID
            },
        )
        .unwrap();
        assert!(volume.slice(Axis::X, 2).is_err());
        volume.densities[0] = f32::NAN;
        assert_eq!(volume.validate(), Err(FieldError::Volume));
    }
}
