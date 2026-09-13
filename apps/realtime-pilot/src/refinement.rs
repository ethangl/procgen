//! One conforming edge split with bounded, field-guided displacement.
use crate::{PlanetField, SurfaceMesh, SurfaceTriangle};
use procgen_core::Vec3;
use rayon::prelude::*;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

pub(crate) struct RefinedSurface {
    pub mesh: SurfaceMesh,
    /// New positions follow the original vertices in this shared-edge order.
    pub edges: Vec<[u32; 2]>,
}

/// Input is the validated contour. Only edges whose two incident triangles
/// permit refinement are split, so a protected collar retains its full stencil.
pub(crate) fn refine_surface(
    field: &PlanetField,
    source: &SurfaceMesh,
    eligible: &[bool],
    source_normals: &[Vec3],
    cancel: &AtomicBool,
) -> Option<RefinedSurface> {
    assert_eq!(eligible.len(), source.triangles.len());
    assert_eq!(source_normals.len(), source.positions.len());
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let mut edges = BTreeMap::<[u32; 2], bool>::new();
    for (t, &eligible) in source.triangles.iter().zip(eligible) {
        for edge in triangle_edges(t.vertices) {
            edges
                .entry(edge)
                .and_modify(|value| *value &= eligible)
                .or_insert(eligible);
        }
    }
    let edges: Vec<_> = edges
        .into_iter()
        .filter_map(|(edge, eligible)| eligible.then_some(edge))
        .collect();
    let ids: BTreeMap<_, _> = edges
        .iter()
        .enumerate()
        .map(|(i, &edge)| (edge, (source.positions.len() + i) as u32))
        .collect();
    let project = |edge: &[u32; 2]| {
        let [a, b] = edge.map(|i| source.positions[i as usize]);
        let direction =
            (source_normals[edge[0] as usize] + source_normals[edge[1] as usize]).normalized();
        project_midpoint(field, a, b, direction)
    };
    let proposed: Vec<_> = if edges.len() >= 1024 {
        edges.par_iter().map(project).collect()
    } else {
        edges.iter().map(project).collect()
    };
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let mut positions = Vec::with_capacity(source.positions.len() + edges.len());
    positions.extend_from_slice(&source.positions);
    positions.extend(proposed);
    let children: Vec<_> = source
        .triangles
        .iter()
        .map(|t| {
            let mids = triangle_edges(t.vertices).map(|edge| ids.get(&edge).copied());
            split_triangle(t.vertices, mids)
        })
        .collect();
    let mut rejected = vec![false; edges.len()];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let mut changed = false;
        for (t, children) in source.triangles.iter().zip(&children) {
            let [a, b, c] = t.vertices.map(|i| source.positions[i as usize]);
            let normal = (b - a).cross(c - a);
            if children.iter().any(|t| {
                let [a, b, c] = t.map(|i| positions[i as usize]);
                (b - a).cross(c - a).dot(normal) <= 0.0
            }) {
                for edge in triangle_edges(t.vertices) {
                    if let Some(&id) = ids.get(&edge) {
                        let index = id as usize - source.positions.len();
                        if !rejected[index] {
                            rejected[index] = true;
                            let [a, b] = edge.map(|i| source.positions[i as usize]);
                            positions[id as usize] = (a + b) * 0.5;
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let triangles = source
        .triangles
        .iter()
        .zip(children)
        .flat_map(|(parent, children)| {
            children.into_iter().map(|vertices| SurfaceTriangle {
                vertices,
                region: parent.region,
            })
        })
        .collect();
    Some(RefinedSurface {
        mesh: SurfaceMesh {
            positions,
            triangles,
        },
        edges,
    })
}

fn triangle_edges([a, b, c]: [u32; 3]) -> [[u32; 2]; 3] {
    [
        [a.min(b), a.max(b)],
        [b.min(c), b.max(c)],
        [c.min(a), c.max(a)],
    ]
}

// A quarter-edge search bounds displacement in model lengths. Missing brackets
// and stationary normals retain linear interpolation; density is not an SDF.
fn project_midpoint(field: &PlanetField, a: Vec3, b: Vec3, direction: Vec3) -> Vec3 {
    let midpoint = (a + b) * 0.5;
    if direction == Vec3::ZERO {
        return midpoint;
    }
    let reach = (b - a).length() * 0.25;
    let mut low = midpoint - direction * reach;
    let mut high = midpoint + direction * reach;
    let mut dl = field.density_at_position(low);
    let dh = field.density_at_position(high);
    if dl == 0.0 {
        return low;
    }
    if dh == 0.0 {
        return high;
    }
    if (dl >= 0.0) == (dh >= 0.0) {
        return midpoint;
    }
    // Ten bisections retain at most 1/1024 of the initial bracket length.
    for _ in 0..10 {
        let p = (low + high) * 0.5;
        let d = field.density_at_position(p);
        if d == 0.0 {
            return p;
        }
        if (d >= 0.0) == (dl >= 0.0) {
            low = p;
            dl = d;
        } else {
            high = p;
        }
    }
    (low + high) * 0.5
}

fn split_triangle(v: [u32; 3], m: [Option<u32>; 3]) -> Vec<[u32; 3]> {
    let [a, b, c] = v;
    match m {
        [None, None, None] => vec![v],
        [Some(ab), Some(bc), Some(ca)] => vec![[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]],
        _ => {
            // Rotate to put either the sole split or sole unsplit edge first.
            let count = m.iter().flatten().count();
            let i = m.iter().position(|e| e.is_some() == (count == 1)).unwrap();
            let [a, b, c] = std::array::from_fn(|j| v[(i + j) % 3]);
            if count == 1 {
                let ab = m[i].unwrap();
                vec![[a, ab, c], [ab, b, c]]
            } else {
                let bc = m[(i + 1) % 3].unwrap();
                let ca = m[(i + 2) % 3].unwrap();
                vec![[a, b, bc], [a, bc, ca], [ca, bc, c]]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PILOT_PLANET, PlanetConfig, TerrainConfig, planet_overview};
    #[test]
    fn every_split_pattern_preserves_area_winding_and_boundary() {
        let p = [
            Vec3::ZERO,
            Vec3::X,
            Vec3::Y,
            Vec3::X * 0.5,
            (Vec3::X + Vec3::Y) * 0.5,
            Vec3::Y * 0.5,
        ];
        for mask in 0..8_u32 {
            let mid = std::array::from_fn(|i| ((mask >> i) & 1 != 0).then_some(i as u32 + 3));
            let triangles = split_triangle([0, 1, 2], mid);
            assert_eq!(triangles.len(), 1 + mask.count_ones() as usize);
            let mut area = 0.0;
            let mut edges = BTreeMap::<[u32; 2], usize>::new();
            for t in triangles {
                let [a, b, c] = t.map(|i| p[i as usize]);
                let normal = (b - a).cross(c - a);
                assert!(normal.z > 0.0);
                area += normal.z;
                for edge in triangle_edges(t) {
                    *edges.entry(edge).or_default() += 1;
                }
            }
            assert_eq!(area, 1.0);
            assert!(edges.values().all(|&count| count <= 2));
            let mut boundary = Vec::new();
            for (i, [a, b]) in [[0, 1], [1, 2], [2, 0]].into_iter().enumerate() {
                if let Some(m) = mid[i] {
                    boundary.extend([[a.min(m), a.max(m)], [b.min(m), b.max(m)]]);
                } else {
                    boundary.push([a.min(b), a.max(b)]);
                }
            }
            boundary.sort();
            assert_eq!(
                edges
                    .into_iter()
                    .filter_map(|(e, n)| (n == 1).then_some(e))
                    .collect::<Vec<_>>(),
                boundary
            );
        }
    }
    #[test]
    fn density_refinement_improves_sphere_and_is_schedule_independent() {
        let field = PlanetConfig {
            terrain: TerrainConfig {
                height_scale: 0.0,
                detail_scale: 0.0,
                cave_density: 0.0,
                ..PILOT_PLANET.terrain
            },
            ..PILOT_PLANET
        }
        .validate(42)
        .unwrap();
        let source = planet_overview(&field, 16).unwrap();
        let eligible = vec![true; source.triangles.len()];
        let normals = source.vertex_normals();
        let cancel = AtomicBool::new(false);
        let generate = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| refine_surface(&field, &source, &eligible, &normals, &cancel).unwrap())
        };
        let a = generate(1);
        let b = generate(4);
        assert_eq!(a.edges, b.edges);
        assert_eq!(a.mesh.positions, b.mesh.positions);
        assert_eq!(a.mesh.triangles, b.mesh.triangles);
        assert!(a.mesh.topology().is_closed_manifold());
        assert_eq!(a.mesh.triangles.len(), source.triangles.len() * 4);
        assert_eq!(a.mesh.positions[..source.positions.len()], source.positions);
        let mut before = 0.0;
        let mut after = 0.0;
        for (edge, &p) in a
            .edges
            .iter()
            .zip(&a.mesh.positions[source.positions.len()..])
        {
            let [u, v] = edge.map(|i| source.positions[i as usize]);
            let midpoint = (u + v) * 0.5;
            before += (midpoint.length() - field.config().radius).abs();
            after += (p.length() - field.config().radius).abs();
            assert!((p - midpoint).length() <= (v - u).length() * 0.25 + 0.000001);
        }
        // The root search should remove most interpolation error, not just add triangles.
        assert!(after < before * 0.1, "before={before} after={after}");
        for triangle in &a.mesh.triangles {
            let [a, b, c] = triangle.vertices.map(|i| a.mesh.positions[i as usize]);
            assert!((b - a).cross(c - a).dot(a) > 0.0);
        }
        cancel.store(true, Ordering::Relaxed);
        assert!(refine_surface(&field, &source, &eligible, &normals, &cancel).is_none());
    }
}
