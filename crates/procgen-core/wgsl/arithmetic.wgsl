// Runtime identities preserve CPU expression boundaries when the consumer
// needs stable f32 operations. Both identity values must be exactly one and
// zero. Passing literals instead permits the usual shader optimizations.
struct F32Arithmetic { one: f32, zero: f32, }
fn f32_add(a: f32, b: f32, math: F32Arithmetic) -> f32 {
    return fma(a, math.one, b);
}
fn f32_mul(a: f32, b: f32, math: F32Arithmetic) -> f32 {
    return fma(a, b, math.zero);
}
fn f32_scale_add(a: vec3<f32>, scale: f32, b: vec3<f32>, math: F32Arithmetic) -> vec3<f32> {
    return fma(fma(a, vec3(scale), vec3(math.zero)), vec3(math.one), b);
}
