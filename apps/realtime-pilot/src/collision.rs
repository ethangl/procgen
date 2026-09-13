//! Two-sided fine-triangle queries. No inside/outside or manifold assumption.
use crate::DetailSource;
use procgen_core::Vec3;
use std::{ops::Range, sync::Arc};

pub const USABLE_MEMORY_RESERVATION: usize = 32 * 1024 * 1024;
pub const COLLISION_REACH: f32 = 0.6;
const MAX_PATCH_TRIANGLES: usize = 32768;
pub const CONTACT_SKIN: f32 = 0.0001;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactError {
    Capacity,
    MissingCoverage,
    InvalidMotion,
    NoLanding,
    SweepLimit,
}
impl std::fmt::Display for ContactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Capacity => "collision working set exceeds its reserved capacity",
            Self::MissingCoverage => "movement stopped at missing collision coverage",
            Self::InvalidMotion => "collision motion must be finite and bounded",
            Self::NoLanding => "no clear, walkable landing was found",
            Self::SweepLimit => "movement stopped at the collision sweep iteration limit",
        })
    }
}
impl std::error::Error for ContactError {}

#[derive(Clone, Copy)]
struct Bounds {
    min: Vec3,
    max: Vec3,
}
impl Bounds {
    fn around(p: Vec3, radius: f32) -> Self {
        let r = Vec3::new(radius, radius, radius);
        Self {
            min: p - r,
            max: p + r,
        }
    }
    fn distance_squared(self, p: Vec3) -> f32 {
        let closest = components(components(p, self.min, f32::max), self.max, f32::min);
        closest.distance_squared(p)
    }
    fn include(self, other: Self) -> Self {
        Self {
            min: components(self.min, other.min, f32::min),
            max: components(self.max, other.max, f32::max),
        }
    }
    fn intersects(self, other: Self) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
            && self.min.z <= other.max.z
            && self.max.z >= other.min.z
    }
}
fn components(a: Vec3, b: Vec3, f: fn(f32, f32) -> f32) -> Vec3 {
    Vec3::new(f(a.x, b.x), f(a.y, b.y), f(a.z, b.z))
}
struct Node {
    bounds: Bounds,
    range: Range<usize>,
    children: Option<[usize; 2]>,
}
pub struct TerrainQueries {
    source: Arc<DetailSource>,
    order: Vec<usize>,
    nodes: Vec<Node>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceContact {
    pub position: Vec3,
    pub normal: Vec3,
    pub triangle: usize,
}
impl TerrainQueries {
    pub fn new(source: Arc<DetailSource>) -> Result<Self, ContactError> {
        let mut result = Self {
            order: (0..source.mesh.triangles.len()).collect(),
            source,
            nodes: Vec::new(),
        };
        result.partition(0..result.order.len());
        if result.allocated_bytes() > USABLE_MEMORY_RESERVATION / 2 {
            return Err(ContactError::Capacity);
        }
        Ok(result)
    }
    pub fn allocated_bytes(&self) -> usize {
        self.order.capacity() * std::mem::size_of::<usize>()
            + self.nodes.capacity() * std::mem::size_of::<Node>()
    }
    fn triangle(&self, id: usize) -> [Vec3; 3] {
        self.source.mesh.triangles[id]
            .vertices
            .map(|i| self.source.mesh.positions[i as usize])
    }
    fn bounds(&self, id: usize) -> Bounds {
        self.triangle(id)
            .into_iter()
            .map(|p| Bounds::around(p, 0.0))
            .reduce(Bounds::include)
            .expect("three vertices")
    }
    fn partition(&mut self, range: Range<usize>) -> usize {
        let bounds = self.order[range.clone()]
            .iter()
            .map(|&id| self.bounds(id))
            .reduce(Bounds::include)
            .expect("nonempty contour");
        let id = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            range: range.clone(),
            children: None,
        });
        if range.len() > 16 {
            let extent = bounds.max - bounds.min;
            let axis = if extent.x >= extent.y && extent.x >= extent.z {
                0
            } else if extent.y >= extent.z {
                1
            } else {
                2
            };
            let mesh = &self.source.mesh;
            self.order[range.clone()].sort_unstable_by(|&a, &b| {
                let center = |id: usize| {
                    let p = mesh.triangles[id]
                        .vertices
                        .map(|i| mesh.positions[i as usize]);
                    let v = p[0] + p[1] + p[2];
                    [v.x, v.y, v.z][axis]
                };
                center(a).total_cmp(&center(b)).then(a.cmp(&b))
            });
            let mid = range.start + range.len() / 2;
            let left = self.partition(range.start..mid);
            let right = self.partition(mid..range.end);
            self.nodes[id].children = Some([left, right]);
        }
        id
    }
    fn visit(&self, bounds: Bounds, mut visit: impl FnMut(usize)) {
        let mut stack = vec![0];
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id];
            if !node.bounds.intersects(bounds) {
                continue;
            }
            if let Some([a, b]) = node.children {
                stack.push(b);
                stack.push(a);
            } else {
                for &triangle in &self.order[node.range.clone()] {
                    if self.bounds(triangle).intersects(bounds) {
                        visit(triangle);
                    }
                }
            }
        }
    }
    /// Nearest two-sided intersection on a finite segment; ties use source IDs.
    pub fn ray(&self, start: Vec3, end: Vec3) -> Option<SurfaceContact> {
        let delta = end - start;
        let mut best: Option<(f32, SurfaceContact)> = None;
        self.visit(
            Bounds::around(start, 0.0).include(Bounds::around(end, 0.0)),
            |id| {
                let [a, b, c] = self.triangle(id);
                let n = (b - a).cross(c - a);
                let denominator = n.dot(delta);
                if denominator.abs() < 1e-12 {
                    return;
                }
                let t = n.dot(a - start) / denominator;
                if !(0.0..=1.0).contains(&t) {
                    return;
                }
                let p = start + delta * t;
                if !inside(p, [a, b, c], n) {
                    return;
                }
                let hit = SurfaceContact {
                    position: p,
                    normal: n.normalized(),
                    triangle: id,
                };
                if best.is_none_or(|(old, h)| t < old || (t == old && id < h.triangle)) {
                    best = Some((t, hit));
                }
            },
        );
        best.map(|(_, hit)| hit)
    }
    pub fn patch(&self, center: Vec3) -> Result<CollisionPatch, ContactError> {
        let mut triangles = Vec::new();
        let mut overflow = false;
        self.visit(Bounds::around(center, COLLISION_REACH), |id| {
            if triangles.len() == MAX_PATCH_TRIANGLES {
                overflow = true;
            } else {
                triangles.push(id);
            }
        });
        if overflow {
            return Err(ContactError::Capacity);
        }
        triangles.sort_unstable();
        Ok(CollisionPatch { center, triangles })
    }
    // Traverse nearby BVH nodes first and prune by their distance lower bound.
    // The patch's sorted IDs retain the exact candidate set used by the sweep.
    fn nearest(&self, p: Vec3, candidates: &[usize]) -> Option<(f32, Vec3)> {
        if candidates.is_empty() {
            return None;
        }
        let mut best: Option<(f32, usize, Vec3)> = None;
        let mut stack = vec![0];
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id];
            if best.is_some_and(|(d, _, _)| node.bounds.distance_squared(p) > d) {
                continue;
            }
            if let Some([a, b]) = node.children {
                let (near, far) = if self.nodes[a].bounds.distance_squared(p)
                    <= self.nodes[b].bounds.distance_squared(p)
                {
                    (a, b)
                } else {
                    (b, a)
                };
                stack.push(far);
                stack.push(near);
            } else {
                for &triangle in &self.order[node.range.clone()] {
                    if candidates.binary_search(&triangle).is_err() {
                        continue;
                    }
                    if best.is_some_and(|(d, _, _)| self.bounds(triangle).distance_squared(p) > d) {
                        continue;
                    }
                    let q = closest(p, self.triangle(triangle));
                    let distance = p.distance_squared(q);
                    if best
                        .is_none_or(|(d, id, _)| distance < d || (distance == d && triangle < id))
                    {
                        best = Some((distance, triangle, q));
                    }
                }
            }
        }
        best.map(|(d, _, q)| (d.sqrt(), q))
    }
    pub fn exterior(&self, direction: Vec3) -> Option<SurfaceContact> {
        // Source vertices lie within this fixed pilot's validated spherical shell.
        let bounds = self.nodes[0].bounds;
        let outer = bounds.min.length().max(bounds.max.length());
        self.ray(direction.normalized() * outer, Vec3::ZERO)
    }
}
fn inside(p: Vec3, [a, b, c]: [Vec3; 3], n: Vec3) -> bool {
    (b - a).cross(p - a).dot(n) >= 0.0
        && (c - b).cross(p - b).dot(n) >= 0.0
        && (a - c).cross(p - c).dot(n) >= 0.0
}
fn closest(p: Vec3, t: [Vec3; 3]) -> Vec3 {
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
pub struct CollisionPatch {
    center: Vec3,
    triangles: Vec<usize>,
}
pub struct Sweep {
    pub position: Vec3,
    pub normal: Option<Vec3>,
}
impl CollisionPatch {
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }
    pub fn covers(&self, p: Vec3, radius: f32) -> bool {
        let d = p - self.center;
        d.x.abs() + radius < COLLISION_REACH
            && d.y.abs() + radius < COLLISION_REACH
            && d.z.abs() + radius < COLLISION_REACH
    }
    pub fn clearance(&self, terrain: &TerrainQueries, p: Vec3) -> Option<(f32, Vec3)> {
        terrain.nearest(p, &self.triangles)
    }
    /// Conservative advancement uses exact triangle distance, never density magnitude.
    /// Iteration exhaustion rejects this sweep; the caller retains its safe start.
    pub fn sweep(
        &self,
        terrain: &TerrainQueries,
        start: Vec3,
        delta: Vec3,
        radius: f32,
    ) -> Result<Sweep, ContactError> {
        if !start.is_finite() || !delta.is_finite() || !radius.is_finite() || radius <= 0.0 {
            return Err(ContactError::InvalidMotion);
        }
        if !self.covers(start, radius) || !self.covers(start + delta, radius) {
            return Err(ContactError::MissingCoverage);
        }
        let length = delta.length();
        let mut travelled = 0.0;
        let mut p = start;
        for _ in 0..64 {
            let Some((distance, q)) = self.clearance(terrain, p) else {
                return Ok(Sweep {
                    position: start + delta,
                    normal: None,
                });
            };
            if distance <= radius + CONTACT_SKIN {
                return Ok(Sweep {
                    position: p,
                    normal: Some((p - q).normalized()),
                });
            }
            let safe = distance - radius;
            if travelled + safe >= length {
                return Ok(Sweep {
                    position: start + delta,
                    normal: None,
                });
            }
            travelled += safe * 0.9;
            p = start + delta * (travelled / length);
        }
        Err(ContactError::SweepLimit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RegionAddress, SurfaceMesh, SurfaceTriangle};
    use procgen_cubesphere::CubeFace;
    fn plane() -> TerrainQueries {
        TerrainQueries::new(crate::detail::test_source(SurfaceMesh {
            positions: vec![
                Vec3::new(-1.0, -1.0, 0.0),
                Vec3::new(1.0, -1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            triangles: vec![SurfaceTriangle {
                vertices: [0, 1, 2],
                region: RegionAddress {
                    face: CubeFace::PositiveZ,
                },
            }],
        }))
        .unwrap()
    }
    #[test]
    fn face_edge_vertex_distances_and_two_sided_fast_sweep() {
        let terrain = plane();
        let t = terrain.triangle(0);
        assert_eq!(closest(Vec3::new(0.0, 0.0, 0.2), t), Vec3::ZERO);
        assert_eq!(
            closest(Vec3::new(0.0, -2.0, 0.0), t),
            Vec3::new(0.0, -1.0, 0.0)
        );
        assert_eq!(closest(Vec3::new(0.0, 2.0, 0.0), t), Vec3::Y);
        let patch = terrain.patch(Vec3::ZERO).unwrap();
        for sign in [-1.0, 1.0] {
            let hit = patch
                .sweep(
                    &terrain,
                    Vec3::Z * (0.4 * sign),
                    Vec3::Z * (-0.8 * sign),
                    0.035,
                )
                .unwrap();
            assert!((hit.position.z.abs() - 0.035).abs() < CONTACT_SKIN * 2.0);
            assert!(hit.normal.unwrap().z * sign > 0.99);
            let ray = terrain.ray(Vec3::Z * sign, -Vec3::Z * sign).unwrap();
            assert_eq!(ray.position, Vec3::ZERO);
        }
        assert!(matches!(
            patch.sweep(&terrain, Vec3::Z * 0.4, Vec3::X, 0.035),
            Err(ContactError::MissingCoverage)
        ));
    }
    #[test]
    fn nearest_tree_search_matches_exhaustive_patch_distance() {
        let positions: Vec<_> = crate::test_support::positions().collect();
        let mesh = SurfaceMesh {
            triangles: (0..positions.len() as u32 / 3)
                .map(|i| SurfaceTriangle {
                    vertices: [i * 3, i * 3 + 1, i * 3 + 2],
                    region: RegionAddress {
                        face: CubeFace::PositiveX,
                    },
                })
                .collect(),
            positions,
        };
        let terrain = TerrainQueries::new(crate::detail::test_source(mesh)).unwrap();
        for center in [Vec3::ZERO, Vec3::X * 0.8, Vec3::Y * -0.8] {
            let patch = terrain.patch(center).unwrap();
            for p in crate::test_support::positions() {
                let expected = patch
                    .triangles
                    .iter()
                    .map(|&id| p.distance_squared(closest(p, terrain.triangle(id))))
                    .min_by(f32::total_cmp)
                    .unwrap()
                    .sqrt();
                let (actual, q) = patch.clearance(&terrain, p).unwrap();
                assert!(
                    (actual - expected).abs() < 0.000001,
                    "actual={actual} expected={expected}"
                );
                assert!(((p - q).length() - actual).abs() < 0.000001);
            }
        }
        assert!(terrain.nearest(Vec3::ZERO, &[]).is_none());
    }
}
