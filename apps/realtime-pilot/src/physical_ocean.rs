//! Camera-relative ocean uniforms. No terrain samples or generation work.
use procgen_realtime_pilot::MeterPosition;

pub struct OceanCamera {
    pub radial: [f32; 4],
    pub sphere: [f32; 4],
}
impl OceanCamera {
    pub fn new(eye: MeterPosition, reference_radius_m: f32, sea_level_m: f32) -> Self {
        // Host positioning retains sub-meter altitude before packing f32 uniforms.
        // Never subtract two squared planet radii in the shader.
        let radius = reference_radius_m as f64 + sea_level_m as f64;
        let altitude = eye.altitude_m(reference_radius_m) - sea_level_m as f64;
        let distance = radius + altitude;
        let radial = eye.direction();
        Self {
            radial: [radial.x, radial.y, radial.z, distance as f32],
            sphere: [
                radius as f32,
                altitude as f32,
                (altitude * (2.0 * radius + altitude)) as f32,
                1.0,
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_core::Vec3;
    use procgen_realtime_pilot::VoxelPosition;

    #[test]
    fn sea_level_and_centimeter_motion_survive_large_radii() {
        for radius in [100_000, 300_000, 8_000_000] {
            for offset in [-0.02, 0.0, 0.02] {
                let eye = MeterPosition::new(
                    VoxelPosition {
                        x_m: radius,
                        y_m: 0,
                        z_m: 0,
                    },
                    Vec3::X * offset,
                );
                let camera = OceanCamera::new(eye, radius as f32, 0.01);
                assert!((camera.sphere[1] - (offset - 0.01)).abs() < 0.000001);
                assert_eq!(camera.sphere[2].is_sign_negative(), offset < 0.01);
            }
        }
    }
}
