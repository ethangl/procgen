//! Projection of composed terrain controls into a CPU cube field.

use crate::field::{TerrainCellControls, TerrainControls};
use procgen_cubesphere::{BakeError, CubeField, bake_cube_field, control_face_resolution};
use procgen_sphere_mesh::SphereMesh;

pub type TerrainControlBake = CubeField<{ TerrainCellControls::CHANNELS }>;

/// Bakes all terrain-control channels at the mesh-derived policy resolution.
pub fn bake_terrain_controls(
    mesh: &SphereMesh,
    controls: &TerrainControls,
) -> Result<TerrainControlBake, BakeError> {
    let resolution = control_face_resolution(mesh.cell_count())?;
    let cells: Vec<_> = controls
        .cells
        .iter()
        .copied()
        .map(TerrainCellControls::to_channels)
        .collect();
    bake_cube_field(mesh, &cells, resolution)
}
