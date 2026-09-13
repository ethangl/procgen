//! Shared face partitions and edge knots, including arbitrary dyadic LOD ratios.
use crate::voxel_surface::MAX_SURFACE_FACES;
use crate::voxel_surface_grid::{Cell, Grid, Node, check_cancel, contains};
use crate::{VoxelChunkAddress, VoxelSurfaceError};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::AtomicBool,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Face {
    pub axis: usize,
    pub min: [i32; 3],
    pub step: i32,
}
impl Face {
    pub fn axes(self) -> [usize; 2] {
        [(self.axis + 1) % 3, (self.axis + 2) % 3]
    }
    pub fn corners(self) -> [Node; 4] {
        let [u, v] = self.axes();
        [[0, 0], [1, 0], [1, 1], [0, 1]].map(|offset| {
            let mut p = self.min.map(|p| 2 * p);
            p[u] += 2 * self.step * offset[0];
            p[v] += 2 * self.step * offset[1];
            Node(p)
        })
    }
    pub fn center(self) -> Node {
        let [u, v] = self.axes();
        let mut p = self.min.map(|p| 2 * p);
        p[u] += self.step;
        p[v] += self.step;
        Node(p)
    }
    fn children(self) -> [Self; 4] {
        let [u, v] = self.axes();
        std::array::from_fn(|i| {
            let mut min = self.min;
            min[u] += (i as i32 & 1) * (self.step / 2);
            min[v] += ((i as i32 >> 1) & 1) * (self.step / 2);
            Self {
                min,
                step: self.step / 2,
                ..self
            }
        })
    }
    fn intersects(self, chunk: VoxelChunkAddress) -> bool {
        let p = chunk.origin();
        let lo = [p.x_m, p.y_m, p.z_m];
        let span = chunk.span_m();
        let [u, v] = self.axes();
        self.min[self.axis] >= lo[self.axis]
            && self.min[self.axis] <= lo[self.axis] + span
            && [u, v]
                .into_iter()
                .all(|a| self.min[a] < lo[a] + span && self.min[a] + self.step > lo[a])
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Line {
    axis: usize,
    fixed: [i32; 3],
}
impl Line {
    fn between(a: Node, b: Node) -> Self {
        let axis = (0..3)
            .find(|&i| a.0[i] != b.0[i])
            .expect("nonempty axis-aligned edge");
        let mut fixed = a.0;
        fixed[axis] = 0;
        Self { axis, fixed }
    }
}
pub(crate) struct Faces {
    pub incident: BTreeMap<Face, Vec<Cell>>,
    knots: BTreeMap<Line, BTreeSet<i32>>,
}
impl Faces {
    pub fn build(
        grid: &Grid<'_>,
        cells: BTreeSet<Cell>,
        cancel: &AtomicBool,
    ) -> Result<Self, VoxelSurfaceError> {
        let mut incident = BTreeMap::<Face, Vec<Cell>>::new();
        for cell in cells {
            check_cancel(cancel)?;
            for axis in 0..3 {
                for side in 0..2 {
                    let mut min = cell.min;
                    min[axis] += side * cell.step();
                    let face = Face {
                        axis,
                        min,
                        step: cell.step(),
                    };
                    let candidates: Vec<_> = grid
                        .volumes
                        .keys()
                        .copied()
                        .filter(|&a| a.spacing_m() < cell.step() && face.intersects(a))
                        .collect();
                    let mut pending = vec![face];
                    while let Some(face) = pending.pop() {
                        check_cancel(cancel)?;
                        if candidates
                            .iter()
                            .any(|&a| a.spacing_m() < face.step && face.intersects(a))
                        {
                            pending.extend(face.children());
                        } else {
                            incident.entry(face).or_default().push(cell);
                            if incident.len() > MAX_SURFACE_FACES {
                                return Err(VoxelSurfaceError::Capacity);
                            }
                        }
                    }
                }
            }
        }
        let mut knots = BTreeMap::<Line, BTreeSet<i32>>::new();
        // Refined face corners also split neighbouring face edges. Gathering
        // these globally closes edge/corner T junctions, not just face seams.
        for face in incident.keys() {
            let corners = face.corners();
            for i in 0..4 {
                let a = corners[i];
                let b = corners[(i + 1) % 4];
                let line = Line::between(a, b);
                knots
                    .entry(line)
                    .or_default()
                    .extend([a.0[line.axis], b.0[line.axis]]);
            }
        }
        Ok(Self { incident, knots })
    }
    pub fn perimeter(&self, face: Face) -> Vec<Node> {
        let corners = face.corners();
        let mut result = Vec::new();
        for i in 0..4 {
            let a = corners[i];
            let b = corners[(i + 1) % 4];
            let line = Line::between(a, b);
            let lo = a.0[line.axis].min(b.0[line.axis]);
            let hi = a.0[line.axis].max(b.0[line.axis]);
            let mut values: Vec<_> = self.knots[&line].range(lo..=hi).copied().collect();
            if a.0[line.axis] > b.0[line.axis] {
                values.reverse();
            }
            values.pop();
            result.extend(values.into_iter().map(|v| {
                let mut p = line.fixed;
                p[line.axis] = v;
                Node(p)
            }));
        }
        result
    }
    pub fn exterior(grid: &Grid<'_>, face: Face, cell: Cell) -> bool {
        // Half-meter probes on either side of a source face distinguish an
        // intentional support boundary from an internal coarse/fine seam.
        let mut outside = face.center();
        outside.0[face.axis] += if face.min[face.axis] == cell.min[face.axis] {
            -1
        } else {
            1
        };
        !grid.volumes.keys().any(|&a| contains(a, outside))
    }
}
