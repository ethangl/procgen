//! March a conforming cell-center/face-center tetrahedral complex.
use crate::voxel_surface::{MAX_SURFACE_TRIANGLES, MAX_SURFACE_VERTICES};
use crate::voxel_surface_faces::Faces;
use crate::voxel_surface_grid::{Cell, Grid, Node, check_cancel};
use crate::{VoxelPosition, VoxelSurface, VoxelSurfaceError, VoxelTriangle, VoxelVolume};
use procgen_core::Vec3;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::AtomicBool,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Root([Node; 2]);
#[derive(Clone, Copy)]
struct Sample {
    node: Node,
    value: f32,
}
fn crosses(a: Sample, b: Sample) -> bool {
    (a.value >= 0.0) != (b.value >= 0.0)
}
fn root(a: Sample, b: Sample) -> Root {
    if a.value == 0.0 {
        Root([a.node; 2])
    } else if b.value == 0.0 {
        Root([b.node; 2])
    } else {
        let mut pair = [a.node, b.node];
        pair.sort();
        Root(pair)
    }
}
struct Builder<'a> {
    grid: Grid<'a>,
    samples: BTreeMap<Node, f32>,
    vertices: BTreeMap<Root, u32>,
    surface: VoxelSurface,
}
impl Builder<'_> {
    fn sample(&mut self, node: Node) -> Sample {
        let value = *self
            .samples
            .entry(node)
            .or_insert_with(|| self.grid.potential(node));
        Sample { node, value }
    }
    fn vertex(&mut self, r: Root) -> Result<u32, VoxelSurfaceError> {
        if let Some(&id) = self.vertices.get(&r) {
            return Ok(id);
        }
        if self.surface.positions.len() == MAX_SURFACE_VERTICES {
            return Err(VoxelSurfaceError::Capacity);
        }
        let a = self.sample(r.0[0]);
        let b = self.sample(r.0[1]);
        // Interpolate from the endpoint nearest the zero. At large radii a
        // sphere's sub-meter curvature can be tiny relative to a voxel edge;
        // subtracting a fraction near one collapses distinct roots onto nodes.
        let (near, far) = if a.value.abs() <= b.value.abs() {
            (a, b)
        } else {
            (b, a)
        };
        let p = near.node.relative(self.surface.origin_m);
        let position = if near.node == far.node {
            p
        } else {
            let delta = Vec3::new(
                (far.node.0[0] - near.node.0[0]) as f32,
                (far.node.0[1] - near.node.0[1]) as f32,
                (far.node.0[2] - near.node.0[2]) as f32,
            ) * 0.5;
            p + delta * (near.value / (near.value - far.value))
        };
        let id = self.surface.positions.len() as u32;
        self.surface.positions.push(position);
        self.vertices.insert(r, id);
        Ok(id)
    }
    fn tetrahedron(&mut self, nodes: [Node; 4], cell: Cell) -> Result<(), VoxelSurfaceError> {
        let values = nodes.map(|p| self.sample(p));
        let solid: Vec<_> = values.into_iter().filter(|s| s.value >= 0.0).collect();
        let air: Vec<_> = values.into_iter().filter(|s| s.value < 0.0).collect();
        let mut ring = match (solid.len(), air.len()) {
            (1, 3) => air.iter().map(|&a| root(solid[0], a)).collect::<Vec<_>>(),
            (3, 1) => solid.iter().map(|&s| root(s, air[0])).collect(),
            (2, 2) => vec![
                root(solid[0], air[0]),
                root(solid[0], air[1]),
                root(solid[1], air[1]),
                root(solid[1], air[0]),
            ],
            _ => return Ok(()),
        };
        // Exact-zero endpoints share a node identity. Collapsed intersections
        // represent an edge or point, not a triangle; no density epsilon is used.
        let mut seen = BTreeSet::new();
        ring.retain(|r| seen.insert(*r));
        if ring.len() < 3 {
            return Ok(());
        }
        let first = ring.iter().enumerate().min_by_key(|(_, r)| **r).unwrap().0;
        ring.rotate_left(first);
        let mut ids = ring
            .into_iter()
            .map(|r| self.vertex(r))
            .collect::<Result<Vec<_>, _>>()?;
        let centroid = |set: &[Sample]| {
            set.iter().fold(Vec3::ZERO, |sum, s| {
                sum + s.node.relative(self.surface.origin_m)
            }) * (set.len() as f32).recip()
        };
        let outward = centroid(&air) - centroid(&solid);
        let [a, b, c] = [ids[0], ids[1], ids[2]].map(|i| self.surface.positions[i as usize]);
        if (b - a).cross(c - a).dot(outward) < 0.0 {
            ids[1..].reverse();
        }
        for i in 1..ids.len() - 1 {
            if self.surface.triangles.len() == MAX_SURFACE_TRIANGLES {
                return Err(VoxelSurfaceError::Capacity);
            }
            self.surface.triangles.push(VoxelTriangle {
                vertices: [ids[0], ids[i], ids[i + 1]],
                chunk: cell.chunk,
            });
        }
        Ok(())
    }
    fn boundary(&mut self, nodes: [Node; 3]) -> Result<(), VoxelSurfaceError> {
        let samples = nodes.map(|p| self.sample(p));
        let roots: BTreeSet<_> = (0..3)
            .filter_map(|i| {
                let a = samples[i];
                let b = samples[(i + 1) % 3];
                crosses(a, b).then(|| root(a, b))
            })
            .collect();
        if roots.len() == 2 {
            let ids = roots
                .into_iter()
                .map(|r| self.vertex(r))
                .collect::<Result<Vec<_>, _>>()?;
            self.surface
                .boundary
                .insert([ids[0].min(ids[1]), ids[0].max(ids[1])]);
        }
        Ok(())
    }
}

/// CPU canonical mesh, with full-band unsaturated source potentials. Derived
/// centers interpolate the source grid; they do not sample extra noise octaves.
/// The local origin must lie inside the octree root.
/// Cancellation discards the entire patch. Callers retain prior render coverage
/// until a complete replacement is ready.
pub fn build_voxel_surface(
    volumes: &[&VoxelVolume],
    origin_m: VoxelPosition,
    cancel: &AtomicBool,
) -> Result<VoxelSurface, VoxelSurfaceError> {
    check_cancel(cancel)?;
    // A local terrain origin belongs to the addressable world. This also keeps
    // doubled integer coordinates and relative subtraction within i32.
    crate::VoxelChunkAddress::containing(origin_m, 0)?;
    let grid = Grid::new(volumes)?;
    let cells = grid.active_cells(cancel)?;
    let faces = Faces::build(&grid, cells, cancel)?;
    let mut builder = Builder {
        grid,
        samples: BTreeMap::new(),
        vertices: BTreeMap::new(),
        surface: VoxelSurface {
            origin_m,
            positions: Vec::new(),
            triangles: Vec::new(),
            boundary: BTreeSet::new(),
        },
    };
    for (&face, cells) in &faces.incident {
        check_cancel(cancel)?;
        let perimeter = faces.perimeter(face);
        for &cell in cells {
            let exterior = Faces::exterior(&builder.grid, face, cell);
            for i in 0..perimeter.len() {
                let triangle = [
                    face.center(),
                    perimeter[i],
                    perimeter[(i + 1) % perimeter.len()],
                ];
                builder
                    .tetrahedron([cell.center(), triangle[0], triangle[1], triangle[2]], cell)?;
                if exterior {
                    builder.boundary(triangle)?;
                }
            }
        }
    }
    builder.surface.validate()?;
    check_cancel(cancel)?;
    Ok(builder.surface)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkIndex, VoxelChunkAddress};
    fn mixed() -> Vec<VoxelChunkAddress> {
        let root = VoxelChunkAddress::containing(
            VoxelPosition {
                x_m: 0,
                y_m: 0,
                z_m: 0,
            },
            2,
        )
        .unwrap();
        let mut chunks = root.children().unwrap().to_vec();
        let fine = chunks.remove(0).children().unwrap();
        chunks.extend(fine);
        chunks
    }
    #[test]
    fn mixed_faces_edges_and_corners_are_closed_and_order_independent() {
        let addresses = mixed();
        let make = |center: Vec3, radius: f32| {
            addresses
                .iter()
                .map(|&a| VoxelVolume::fixture(a, |p| radius - (p.as_vec3() - center).length()))
                .collect::<Vec<_>>()
        };
        for (center, radius) in [
            (Vec3::new(64.1, 64.2, 64.3), 49.3),
            (Vec3::new(64.0, 31.0, 31.0), 0.6),
        ] {
            let volumes = make(center, radius);
            let mut refs = volumes.iter().collect::<Vec<_>>();
            let origin = VoxelPosition {
                x_m: 0,
                y_m: 0,
                z_m: 0,
            };
            let a = build_voxel_surface(&refs, origin, &AtomicBool::new(false)).unwrap();
            assert!(!a.triangles().is_empty());
            assert_eq!(a.topology().boundary_edges, 0);
            refs.reverse();
            let b = build_voxel_surface(&refs, origin, &AtomicBool::new(false)).unwrap();
            assert_eq!(a.positions(), b.positions());
            assert_eq!(a.triangles(), b.triangles());
            assert!(build_voxel_surface(&refs, origin, &AtomicBool::new(true)).is_err());
        }
    }
    #[test]
    fn a_small_fine_feature_closes_against_a_thirty_two_times_coarser_chunk() {
        let coarse = VoxelChunkAddress::containing(
            VoxelPosition {
                x_m: 0,
                y_m: 0,
                z_m: 0,
            },
            5,
        )
        .unwrap();
        let fine = VoxelChunkAddress::containing(
            VoxelPosition {
                x_m: 1024,
                y_m: 32,
                z_m: 32,
            },
            0,
        )
        .unwrap();
        let volumes = [coarse, fine].map(|a| {
            VoxelVolume::fixture(a, |p| {
                10.3 - (p.as_vec3() - Vec3::new(1024.0, 48.0, 48.0)).length()
            })
        });
        let mesh = build_voxel_surface(
            &volumes.iter().collect::<Vec<_>>(),
            VoxelPosition {
                x_m: 1024,
                y_m: 32,
                z_m: 32,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(mesh.topology().boundary_edges, 0);
        assert!(!mesh.triangles().is_empty());
    }
    #[test]
    fn unsaturated_coarse_samples_keep_the_plane_at_its_true_altitude() {
        let address = VoxelChunkAddress::containing(
            VoxelPosition {
                x_m: 0,
                y_m: 0,
                z_m: 0,
            },
            4,
        )
        .unwrap();
        let volume = VoxelVolume::fixture(address, |p| 10.25 - p.x_m as f32);
        let origin = VoxelPosition {
            x_m: 0,
            y_m: 0,
            z_m: 0,
        };
        let mesh = build_voxel_surface(&[&volume], origin, &AtomicBool::new(false)).unwrap();
        assert!(mesh.positions().iter().all(|p| p.x == 10.25));
        assert!(matches!(
            build_voxel_surface(&[&volume, &volume], origin, &AtomicBool::new(false)),
            Err(VoxelSurfaceError::Overlap)
        ));
        assert!(matches!(
            build_voxel_surface(
                &vec![&volume; crate::MAX_SURFACE_CHUNKS + 1],
                origin,
                &AtomicBool::new(false)
            ),
            Err(VoxelSurfaceError::Chunks)
        ));
        assert!(
            build_voxel_surface(
                &[&volume],
                VoxelPosition {
                    x_m: i32::MAX,
                    ..origin
                },
                &AtomicBool::new(false)
            )
            .is_err()
        );
    }
    #[test]
    fn exact_zero_planes_have_only_support_boundaries_and_keep_large_origins_local() {
        for offset in [0, 8_000_000] {
            let a = VoxelChunkAddress::containing(
                VoxelPosition {
                    x_m: offset,
                    y_m: 0,
                    z_m: 0,
                },
                0,
            )
            .unwrap();
            let ai = a.index();
            let b = VoxelChunkAddress::new(0, ChunkIndex { x: ai.x + 1, ..ai }).unwrap();
            let volumes = [a, b].map(|a| VoxelVolume::fixture(a, |p| 16.0 - p.y_m as f32));
            let origin = VoxelPosition {
                x_m: offset,
                y_m: 0,
                z_m: 0,
            };
            let surface = build_voxel_surface(
                &volumes.iter().collect::<Vec<_>>(),
                origin,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!(surface.topology().boundary_edges > 0);
            assert!(surface.positions().iter().all(|p| p.y == 16.0));
            for t in surface.triangles() {
                let [a, b, c] = t.vertices.map(|i| surface.positions()[i as usize]);
                assert!((b - a).cross(c - a).y > 0.0);
            }
        }
    }
}
