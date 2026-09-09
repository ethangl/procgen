//! Registry of stable random-stream identifiers shared by generation stages.

pub const FIRST_MAJOR_PLATE_SEED: u64 = 0;
pub const PLATE_ROTATION_AXIS: u64 = 1;
pub const PLATE_ANGULAR_SPEED: u64 = 2;
pub const CRUST_PLATE_ORDER: u64 = 3;
pub const HOTSPOT_POSITION: u64 = 4;
pub const OCEANIC_PEAK_PRESENCE: u64 = 5;
pub const OCEANIC_PEAK_POSITION: u64 = 6;
pub const PLATE_GROWTH_COST: u64 = 7;
pub const TERRAIN_DETAIL_NOISE: u64 = 8;
pub const TERRAIN_ABYSSAL_NOISE: u64 = 9;
pub const TERRAIN_COAST_WARP_X: u64 = 10;
pub const TERRAIN_COAST_WARP_Y: u64 = 11;
pub const TERRAIN_COAST_WARP_Z: u64 = 12;
pub const PLATE_CRACK_ARC: u64 = 13;
pub const PLATE_FACE_SUBDIVISION: u64 = 14;
pub const PLATE_FLOW_FIELD: u64 = 15;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_stream_ids_are_unique() {
        let mut ids = [
            FIRST_MAJOR_PLATE_SEED,
            PLATE_ROTATION_AXIS,
            PLATE_ANGULAR_SPEED,
            CRUST_PLATE_ORDER,
            HOTSPOT_POSITION,
            OCEANIC_PEAK_PRESENCE,
            OCEANIC_PEAK_POSITION,
            PLATE_GROWTH_COST,
            TERRAIN_DETAIL_NOISE,
            TERRAIN_ABYSSAL_NOISE,
            TERRAIN_COAST_WARP_X,
            TERRAIN_COAST_WARP_Y,
            TERRAIN_COAST_WARP_Z,
            PLATE_CRACK_ARC,
            PLATE_FACE_SUBDIVISION,
            PLATE_FLOW_FIELD,
        ];
        ids.sort_unstable();
        assert!(ids.windows(2).all(|pair| pair[0] != pair[1]));
    }
}
