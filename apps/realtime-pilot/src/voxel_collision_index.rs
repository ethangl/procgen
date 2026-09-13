//! Static triangle BVH owned by the independent collision patch.
use crate::VoxelSurface;
use procgen_core::Vec3;

#[derive(Clone, Copy)]
pub(crate) struct Bounds {
    min: [f32; 3],
    max: [f32; 3],
}
impl Bounds {
    pub fn between(a: Vec3, b: Vec3, radius: f32) -> Self {
        let a = [a.x, a.y, a.z];
        let b = [b.x, b.y, b.z];
        Self {
            min: std::array::from_fn(|i| a[i].min(b[i]) - radius),
            max: std::array::from_fn(|i| a[i].max(b[i]) + radius),
        }
    }
    fn union(self, other: Self) -> Self {
        Self {
            min: std::array::from_fn(|i| self.min[i].min(other.min[i])),
            max: std::array::from_fn(|i| self.max[i].max(other.max[i])),
        }
    }
    fn intersects(self, other: Self) -> bool {
        (0..3).all(|i| self.min[i] <= other.max[i] && self.max[i] >= other.min[i])
    }
}
enum Children {
    Leaf(std::ops::Range<usize>),
    Branch(usize, usize),
}
struct Node {
    bounds: Bounds,
    children: Children,
}
pub(crate) struct CollisionIndex {
    nodes: Vec<Node>,
    triangles: Vec<usize>,
}
impl CollisionIndex {
    pub fn build(surface: &VoxelSurface) -> Self {
        let bounds: Vec<_> = surface
            .triangles()
            .iter()
            .map(|t| {
                let [a, b, c] = t
                    .vertices
                    .map(|i| surface.positions()[i as usize].relative_to(surface.origin_m()));
                Bounds::between(a, b, 0.0).union(Bounds::between(c, c, 0.0))
            })
            .collect();
        let mut index = Self {
            nodes: Vec::new(),
            triangles: (0..bounds.len()).collect(),
        };
        if !bounds.is_empty() {
            index.split(&bounds, 0..bounds.len());
        }
        index
    }
    fn split(&mut self, bounds: &[Bounds], range: std::ops::Range<usize>) -> usize {
        let combined = self.triangles[range.clone()]
            .iter()
            .map(|&i| bounds[i])
            .reduce(Bounds::union)
            .unwrap();
        let id = self.nodes.len();
        self.nodes.push(Node {
            bounds: combined,
            children: Children::Leaf(range.clone()),
        });
        if range.len() > 8 {
            let axis = (0..3)
                .max_by(|&a, &b| {
                    (combined.max[a] - combined.min[a])
                        .total_cmp(&(combined.max[b] - combined.min[b]))
                })
                .unwrap();
            let middle = range.start + range.len() / 2;
            self.triangles[range.clone()].select_nth_unstable_by(range.len() / 2, |&a, &b| {
                (bounds[a].min[axis] + bounds[a].max[axis])
                    .total_cmp(&(bounds[b].min[axis] + bounds[b].max[axis]))
                    .then(a.cmp(&b))
            });
            let left = self.split(bounds, range.start..middle);
            let right = self.split(bounds, middle..range.end);
            self.nodes[id].children = Children::Branch(left, right);
        }
        id
    }
    pub fn query(&self, bounds: Bounds) -> Vec<usize> {
        let mut result = Vec::new();
        if !self.nodes.is_empty() {
            self.visit(0, bounds, &mut result);
        }
        // Preserve the canonical triangle tie-break regardless of tree layout.
        result.sort_unstable();
        result
    }
    fn visit(&self, id: usize, bounds: Bounds, result: &mut Vec<usize>) {
        let node = &self.nodes[id];
        if !node.bounds.intersects(bounds) {
            return;
        }
        match &node.children {
            Children::Leaf(range) => result.extend_from_slice(&self.triangles[range.clone()]),
            Children::Branch(a, b) => {
                self.visit(*a, bounds, result);
                self.visit(*b, bounds, result);
            }
        }
    }
    pub fn payload_bytes(&self) -> usize {
        self.nodes.capacity() * std::mem::size_of::<Node>()
            + self.triangles.capacity() * std::mem::size_of::<usize>()
    }
}
