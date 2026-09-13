//! Integer meters and a sub-meter offset for physical points.
use crate::VoxelPosition;
use procgen_core::Vec3;

/// Integer anchor plus sub-meter offset. Translation rebases before the next
/// float addition, so a centimeter step survives at an eight-million-meter radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterPosition {
    anchor: VoxelPosition,
    offset: Vec3,
}
impl MeterPosition {
    pub fn new(anchor: VoxelPosition, offset: Vec3) -> Self {
        assert!(offset.is_finite());
        let shift = [offset.x.round(), offset.y.round(), offset.z.round()];
        let add = |a: i32, b: f32| {
            assert!(b >= i32::MIN as f32 && b < i32::MAX as f32);
            a.checked_add(b as i32)
                .expect("position exceeds integer meter domain")
        };
        Self {
            anchor: VoxelPosition {
                x_m: add(anchor.x_m, shift[0]),
                y_m: add(anchor.y_m, shift[1]),
                z_m: add(anchor.z_m, shift[2]),
            },
            offset: offset - Vec3::new(shift[0], shift[1], shift[2]),
        }
    }
    pub fn anchor(self) -> VoxelPosition {
        self.anchor
    }
    pub fn relative_to(self, origin: VoxelPosition) -> Vec3 {
        self.anchor.relative_to(origin).as_vec3() + self.offset
    }
    pub fn translated(self, delta: Vec3) -> Self {
        Self::new(self.anchor, self.offset + delta)
    }
    pub fn direction(self) -> Vec3 {
        (self.anchor.as_vec3() + self.offset).normalized()
    }
    /// Host navigation only; density/kernel arithmetic remains f32.
    pub fn altitude_m(self, radius_m: f32) -> f64 {
        let p = [
            self.anchor.x_m as f64 + self.offset.x as f64,
            self.anchor.y_m as f64 + self.offset.y as f64,
            self.anchor.z_m as f64 + self.offset.z as f64,
        ];
        p.iter().map(|v| v * v).sum::<f64>().sqrt() - radius_m as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn centimeter_motion_survives_large_origins_and_negative_rebasing() {
        let origin = VoxelPosition {
            x_m: 8_000_000,
            y_m: -8_000_000,
            z_m: 0,
        };
        let mut p = MeterPosition::new(origin, Vec3::ZERO);
        for _ in 0..1000 {
            p = p.translated(Vec3::new(0.01, -0.01, 0.0));
        }
        let d = p.relative_to(origin);
        assert!((d.x - 10.0).abs() < 0.001 && (d.y + 10.0).abs() < 0.001);
    }
}
