use procgen_core::Vec3;
use procgen_cubesphere::TileAddress;

use crate::{
    TerrainCellControls, TerrainControlBake, TerrainHeightInputs, TerrainNoiseKeys,
    TerrainStampInput, TerrainTileInputs,
};

pub(crate) const TERRAIN_TEST_SEED: u64 = 0x0123_4567_89AB_CDEF;

pub(crate) fn constant_bake(controls: TerrainCellControls) -> TerrainControlBake {
    let texel = controls.to_channels();
    TerrainControlBake::from_face_texels(4, std::array::from_fn(|_| vec![texel; 16])).unwrap()
}

pub(crate) fn height_inputs<'a>(
    direction: Vec3,
    controls: &'a TerrainControlBake,
    stamps: &'a [TerrainStampInput],
) -> TerrainHeightInputs<'a> {
    TerrainHeightInputs {
        direction,
        controls,
        stamps,
        noise_keys: TerrainNoiseKeys::new(TERRAIN_TEST_SEED),
    }
}

pub(crate) fn tile_inputs<'a>(
    address: TileAddress,
    controls: &'a TerrainControlBake,
    stamps: &'a [TerrainStampInput],
) -> TerrainTileInputs<'a> {
    TerrainTileInputs {
        address,
        controls,
        stamps,
        noise_keys: TerrainNoiseKeys::new(TERRAIN_TEST_SEED),
    }
}
