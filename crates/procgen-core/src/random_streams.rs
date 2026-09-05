//! Registry of stable random-stream identifiers shared by generation stages.

pub const FIRST_MAJOR_PLATE_SEED: u64 = 0;
pub const PLATE_ROTATION_AXIS: u64 = 1;
pub const PLATE_ANGULAR_SPEED: u64 = 2;
pub const CRUST_PLATE_ORDER: u64 = 3;
pub const HOTSPOT_POSITION: u64 = 4;

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
        ];
        ids.sort_unstable();
        assert!(ids.windows(2).all(|pair| pair[0] != pair[1]));
    }
}
