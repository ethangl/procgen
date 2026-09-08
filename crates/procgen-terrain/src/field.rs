//! Terrain-detail control values and aggregate validation.

use crate::controls::TerrainControlError;
use procgen_core::Vec3;
use procgen_sphere_mesh::SphereMesh;

/// Per-cell controls consumed by later interpolation and height-function slices.
///
/// `base_elevation` is the final isostatically adjusted normalized elevation and is copied
/// unchanged. `detail_amplitude` and `abyssal_amplitude` are normalized-elevation offsets;
/// `ridge_weight` and `octave_gain` are unitless. All four are clamped to `[0, 1]`, and
/// `octave_gain` is the multiplicative gain used between successive noise octaves. Composition
/// order is fixed: baselines, cratons, volcanic arcs, convergent/divergent/transform boundaries,
/// then basins.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TerrainCellControls {
    pub base_elevation: f32,
    pub detail_amplitude: f32,
    pub ridge_weight: f32,
    pub octave_gain: f32,
    pub abyssal_amplitude: f32,
}

impl TerrainCellControls {
    pub const CHANNELS: usize = 5;

    pub fn to_channels(self) -> [f32; Self::CHANNELS] {
        [
            self.base_elevation,
            self.detail_amplitude,
            self.ridge_weight,
            self.octave_gain,
            self.abyssal_amplitude,
        ]
    }

    pub fn from_channels(channels: [f32; Self::CHANNELS]) -> Self {
        let [
            base_elevation,
            detail_amplitude,
            ridge_weight,
            octave_gain,
            abyssal_amplitude,
        ] = channels;
        Self {
            base_elevation,
            detail_amplitude,
            ridge_weight,
            octave_gain,
            abyssal_amplitude,
        }
    }
}

/// Stable type order used when multiple stamps overlap: hotspot, volcanic arc, seamount,
/// then abyssal hill. Within a type, upstream source order is retained.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TerrainStampKind {
    Hotspot,
    VolcanicArc,
    OceanicSeamount,
    OceanicAbyssalHill,
}

/// Sparse input for a later terrain stamp evaluator.
///
/// `position` uses the mesh's surface-coordinate units. `strength` is unitless and clamped
/// to `[0, 1]`. `source_index` is the stable upstream index, with volcanic-arc peaks indexed by
/// their flattened segment/peak order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainStampInput {
    pub cell: usize,
    pub kind: TerrainStampKind,
    pub source_index: usize,
    pub position: Vec3,
    pub strength: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerrainControls {
    pub cells: Vec<TerrainCellControls>,
    /// Sorted by cell, kind, then source index, defining deterministic overlap evaluation.
    pub stamps: Vec<TerrainStampInput>,
}

impl TerrainControls {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), TerrainControlError> {
        if self.cells.len() != mesh.cell_count()
            || self
                .cells
                .iter()
                .flat_map(|controls| controls.to_channels())
                .any(|value| !value.is_finite())
        {
            return Err(TerrainControlError::InvalidCells);
        }
        if self.stamps.iter().any(|stamp| {
            stamp.cell >= mesh.cell_count()
                || !stamp.position.is_finite()
                || !stamp.strength.is_finite()
                || !(0.0..=1.0).contains(&stamp.strength)
        }) {
            return Err(TerrainControlError::InvalidStamps);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::build_sphere_mesh;

    fn mesh() -> SphereMesh {
        build_sphere_mesh(fibonacci_sphere(FibonacciConfig::new(16)).unwrap(), 1.0).unwrap()
    }

    #[test]
    fn channel_conversion_has_one_stable_round_trip() {
        let controls = TerrainCellControls {
            base_elevation: 0.1,
            detail_amplitude: 0.2,
            ridge_weight: 0.3,
            octave_gain: 0.4,
            abyssal_amplitude: 0.5,
        };
        assert_eq!(controls.to_channels(), [0.1, 0.2, 0.3, 0.4, 0.5]);
        assert_eq!(
            TerrainCellControls::from_channels(controls.to_channels()),
            controls
        );
    }

    #[test]
    fn aggregate_validation_owns_cell_and_stamp_invariants() {
        let mesh = mesh();
        let mut controls = TerrainControls {
            cells: vec![TerrainCellControls::default(); mesh.cell_count()],
            stamps: Vec::new(),
        };
        assert_eq!(controls.validate(&mesh), Ok(()));

        controls.cells[0].ridge_weight = f32::NAN;
        assert_eq!(
            controls.validate(&mesh),
            Err(TerrainControlError::InvalidCells)
        );

        controls.cells[0].ridge_weight = 0.0;
        controls.stamps.push(TerrainStampInput {
            cell: mesh.cell_count(),
            kind: TerrainStampKind::Hotspot,
            source_index: 0,
            position: Vec3::X,
            strength: 1.0,
        });
        assert_eq!(
            controls.validate(&mesh),
            Err(TerrainControlError::InvalidStamps)
        );
    }
}
