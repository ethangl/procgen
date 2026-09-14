//! Radial camera protection for a height-field viewer; not swept collision.
use crate::{MeterPosition, PlanetDesignField};

pub const CAMERA_CLEARANCE_M: f32 = 5.0;

/// Raise an underground/near-surface camera along its radial direction. Fast
/// lateral movement can cross ridges; this only constrains the endpoint.
pub fn keep_camera_above_terrain(field: &PlanetDesignField, eye: MeterPosition) -> MeterPosition {
    let direction = eye.direction();
    let height = field.elevation_m(direction, 0.0).expect("camera direction");
    let clearance = eye.altitude_m(field.config().radius_m) - height as f64;
    if clearance < CAMERA_CLEARANCE_M as f64 {
        eye.translated(direction * (CAMERA_CLEARANCE_M as f64 - clearance) as f32)
    } else {
        eye
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PlanetDesignConfig, VoxelPosition};
    use procgen_core::Vec3;

    #[test]
    fn protection_raises_camera_after_terrain_edits_without_moving_safe_positions() {
        let mut config = PlanetDesignConfig::starter(42);
        config.radius_m = 7_999_900.0;
        config.octaves.iter_mut().for_each(|o| o.enabled = false);
        let field = config.validate().unwrap();
        let eye = MeterPosition::new(
            VoxelPosition {
                x_m: 7_999_900,
                y_m: 0,
                z_m: 0,
            },
            Vec3::X * 0.25,
        );
        let safe = keep_camera_above_terrain(&field, eye);
        let clearance = safe.altitude_m(config.radius_m)
            - field.elevation_m(safe.direction(), 0.0).unwrap() as f64;
        assert!(clearance >= CAMERA_CLEARANCE_M as f64 - 0.001);
        let high = safe.translated(Vec3::X * 100.0);
        assert_eq!(keep_camera_above_terrain(&field, high), high);
        config.radius_m += 100.0;
        let changed = config.validate().unwrap();
        let adjusted = keep_camera_above_terrain(&changed, safe);
        assert!((adjusted.altitude_m(config.radius_m) - CAMERA_CLEARANCE_M as f64).abs() < 0.001);
    }
}
