//! Delaunay and Voronoi topology for points on a unit sphere.

mod hull;
mod mesh;

pub use hull::{SphericalDelaunay, TopologyError};
pub use mesh::{SphereMesh, VoronoiEdge};

use procgen_core::Vec3;

/// Builds both sides of the spherical topology from unit-length cell centers.
pub fn build_sphere_mesh(points: Vec<Vec3>, radius: f32) -> Result<SphereMesh, TopologyError> {
    let delaunay = SphericalDelaunay::build(points)?;
    SphereMesh::from_delaunay(&delaunay, radius)
}
