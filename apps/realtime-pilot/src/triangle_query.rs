//! Shared pure, two-sided triangle geometry.
use procgen_core::Vec3;

pub(crate) fn inside(p: Vec3, [a, b, c]: [Vec3; 3], n: Vec3) -> bool {
    (b - a).cross(p - a).dot(n) >= 0.0
        && (c - b).cross(p - b).dot(n) >= 0.0
        && (a - c).cross(p - c).dot(n) >= 0.0
}
pub(crate) fn closest(p: Vec3, t: [Vec3; 3]) -> Vec3 {
    let [a, b, c] = t;
    let n = (b - a).cross(c - a);
    let length = n.length_squared();
    if length > 0.0 {
        let projected = p - n * (n.dot(p - a) / length);
        if inside(projected, t, n) {
            return projected;
        }
    }
    [(a, b), (b, c), (c, a)]
        .map(|(a, b)| {
            let edge = b - a;
            let length = edge.length_squared();
            if length == 0.0 {
                a
            } else {
                a + edge * ((p - a).dot(edge) / length).clamp(0.0, 1.0)
            }
        })
        .into_iter()
        .min_by(|a, b| a.distance_squared(p).total_cmp(&b.distance_squared(p)))
        .expect("three edges")
}
