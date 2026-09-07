use procgen_core::Vec3;

use crate::NoiseSample3;

pub(crate) fn central_difference(
    position: Vec3,
    offset: Vec3,
    sample: impl Fn(Vec3) -> NoiseSample3,
) -> f32 {
    (sample(position + offset).value - sample(position - offset).value) / (2.0 * offset.length())
}

pub(crate) fn sample_bits(sample: NoiseSample3) -> [u32; 4] {
    [
        sample.value.to_bits(),
        sample.derivative.x.to_bits(),
        sample.derivative.y.to_bits(),
        sample.derivative.z.to_bits(),
    ]
}
