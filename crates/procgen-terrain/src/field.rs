//! Terrain-detail control values and aggregate validation.

use procgen_core::Vec3;
use procgen_geology::GeologyInputError;
use procgen_sphere_mesh::SphereMesh;
use procgen_tectonics::StageInputError;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainControlError {
    InvalidConfig,
    InvalidCells,
    InvalidStamps,
    Tectonics(StageInputError),
    Geology(GeologyInputError),
}

impl fmt::Display for TerrainControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str("terrain-control configuration is invalid"),
            Self::InvalidCells => {
                formatter.write_str("terrain-control cells are inconsistent with the mesh")
            }
            Self::InvalidStamps => formatter.write_str("terrain-control stamps are invalid"),
            Self::Tectonics(error) => error.fmt(formatter),
            Self::Geology(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for TerrainControlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidConfig | Self::InvalidCells | Self::InvalidStamps => None,
            Self::Tectonics(error) => Some(error),
            Self::Geology(error) => Some(error),
        }
    }
}

impl From<StageInputError> for TerrainControlError {
    fn from(error: StageInputError) -> Self {
        Self::Tectonics(error)
    }
}

impl From<GeologyInputError> for TerrainControlError {
    fn from(error: GeologyInputError) -> Self {
        Self::Geology(error)
    }
}

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
                || stamp.position == Vec3::ZERO
                || !stamp.strength.is_finite()
                || !(0.0..=1.0).contains(&stamp.strength)
        }) || !self
            .stamps
            .is_sorted_by_key(|stamp| (stamp.cell, stamp.kind, stamp.source_index))
        {
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

        let cells = vec![TerrainCellControls::default(); mesh.cell_count()];
        let stamps = vec![
            TerrainStampInput {
                cell: 1,
                kind: TerrainStampKind::Hotspot,
                source_index: 0,
                position: Vec3::X,
                strength: 1.0,
            },
            TerrainStampInput {
                cell: 0,
                kind: TerrainStampKind::Hotspot,
                source_index: 0,
                position: Vec3::Y,
                strength: 1.0,
            },
        ];
        assert_eq!(
            TerrainControls { cells, stamps }.validate(&mesh),
            Err(TerrainControlError::InvalidStamps)
        );
    }
}
