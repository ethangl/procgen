//! Freudenthal tetrahedra and canonical edge slots, shared by CPU and shader assembly.
use std::sync::LazyLock;

use procgen_core::Vec3;

use crate::{VOXEL_CHUNK_CELLS, VoxelSampleIndex, VoxelVolume};

pub(crate) const CELLS: usize = VOXEL_CHUNK_CELLS as usize;
pub(crate) const NODES: usize = CELLS + 1;
pub(crate) const CELL_COUNT: usize = CELLS * CELLS * CELLS;
// Seven positive monotone edges and one exact-zero node per grid node.
pub const VOXEL_MESH_VERTEX_SLOTS: usize = NODES * NODES * NODES * 8;
pub const VOXEL_MESH_MAX_TRIANGLES: usize = CELL_COUNT * 12;
pub(crate) const TETS: [[u32; 4]; 6] = [
    [0, 1, 3, 7],
    [0, 1, 5, 7],
    [0, 2, 3, 7],
    [0, 2, 6, 7],
    [0, 4, 5, 7],
    [0, 4, 6, 7],
];
pub(crate) const NONE: u32 = u32::MAX;

pub(crate) fn point(id: usize, side: usize) -> [i32; 3] {
    [
        (id % side) as i32,
        ((id / side) % side) as i32,
        (id / (side * side)) as i32,
    ]
}
pub(crate) fn node_id(p: [i32; 3]) -> usize {
    p[0] as usize + NODES * (p[1] as usize + NODES * p[2] as usize)
}
pub(crate) fn corner(mask: u32) -> [i32; 3] {
    [
        (mask & 1) as i32,
        ((mask >> 1) & 1) as i32,
        ((mask >> 2) & 1) as i32,
    ]
}
pub(crate) fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|i| a[i] + b[i])
}
pub(crate) fn potential(volume: &VoxelVolume, p: [i32; 3]) -> f32 {
    volume.potential(VoxelSampleIndex {
        x: p[0],
        y: p[1],
        z: p[2],
    })
}
fn vec(mask: u32) -> Vec3 {
    let p = corner(mask);
    Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)
}

// Oriented edge rings for each tetrahedron and sign mask. Orientation is decided
// once from integer unit corners, never from backend-dependent density math.
pub(crate) static RINGS: LazyLock<[[u32; 4]; 96]> = LazyLock::new(|| {
    std::array::from_fn(|entry| {
        let tet = TETS[entry / 16];
        let case = entry % 16;
        let solid: Vec<_> = (0..4).filter(|i| case & (1 << i) != 0).collect();
        let air: Vec<_> = (0..4).filter(|i| case & (1 << i) == 0).collect();
        let edge = |s: usize, a: usize| tet[s] * 8 + tet[a];
        let mut ring = match (solid.as_slice(), air.as_slice()) {
            ([s], [a, b, c]) => vec![edge(*s, *a), edge(*s, *b), edge(*s, *c)],
            ([a, b, c], [s]) => vec![edge(*a, *s), edge(*b, *s), edge(*c, *s)],
            ([s, t], [a, b]) => vec![edge(*s, *a), edge(*s, *b), edge(*t, *b), edge(*t, *a)],
            _ => return [NONE; 4],
        };
        let midpoint = |e: u32| (vec(e / 8) + vec(e % 8)) * 0.5;
        let normal =
            (midpoint(ring[1]) - midpoint(ring[0])).cross(midpoint(ring[2]) - midpoint(ring[0]));
        let outward = vec(tet[air[0]]) - vec(tet[solid[0]]);
        if normal.dot(outward) < 0.0 {
            ring.reverse();
        }
        std::array::from_fn(|i| ring.get(i).copied().unwrap_or(NONE))
    })
});

pub(crate) fn root_slot(a: [i32; 3], b: [i32; 3], da: f32, db: f32) -> u32 {
    if da == 0.0 {
        return (node_id(a) * 8 + 7) as u32;
    }
    if db == 0.0 {
        return (node_id(b) * 8 + 7) as u32;
    }
    let low = std::array::from_fn(|i| a[i].min(b[i]));
    let d: [i32; 3] = std::array::from_fn(|i| (a[i] - b[i]).abs());
    (node_id(low) * 8) as u32 + (d[0] + 2 * d[1] + 4 * d[2]) as u32 - 1
}

pub(crate) struct CellTriangles {
    pub roots: [[u32; 3]; 12],
    pub count: usize,
}
pub(crate) fn cell_triangles(volume: &VoxelVolume, cell: usize) -> CellTriangles {
    let base = point(cell, CELLS);
    let nodes: [[i32; 3]; 8] = std::array::from_fn(|i| add(base, corner(i as u32)));
    let d = nodes.map(|p| potential(volume, p));
    let mut result = CellTriangles {
        roots: [[0; 3]; 12],
        count: 0,
    };
    for (t, tet) in TETS.iter().enumerate() {
        let case = tet.iter().enumerate().fold(0, |mask, (i, &c)| {
            mask | (usize::from(d[c as usize] >= 0.0) << i)
        });
        let mut roots = [NONE; 4];
        let mut count = 0;
        for &edge in &RINGS[t * 16 + case] {
            if edge == NONE {
                break;
            }
            let (a, b) = ((edge / 8) as usize, (edge % 8) as usize);
            let root = root_slot(nodes[a], nodes[b], d[a], d[b]);
            if !roots[..count].contains(&root) {
                roots[count] = root;
                count += 1;
            }
        }
        for i in 1..count.saturating_sub(1) {
            result.roots[result.count] = [roots[0], roots[i], roots[i + 1]];
            result.count += 1;
        }
    }
    result
}

// The first Freudenthal tetrahedron has positive orientation. Transition cell
// cones use that same orientation, with edges expressed as local vertex pairs.
pub(crate) static SIMPLEX_RINGS: LazyLock<[[u32; 4]; 16]> = LazyLock::new(|| {
    std::array::from_fn(|case| {
        RINGS[case].map(|e| {
            if e == NONE {
                NONE
            } else {
                let index = |c| TETS[0].iter().position(|&v| v == c).unwrap() as u32;
                index(e / 8) * 4 + index(e % 8)
            }
        })
    })
});
