//! Delaunay and Voronoi topology for points on a unit sphere.

mod hull;
mod mesh;

pub use hull::SphericalDelaunay;
pub use mesh::{SphereMesh, VoronoiEdge};

use procgen_core::Vec3;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum TopologyError {
    TooFewPoints,
    NonFinitePoint { index: usize },
    PointNotOnUnitSphere { index: usize, length: f32 },
    DegeneratePoints,
    BrokenHorizon,
    OpenHull,
    InvalidRadius,
}

impl fmt::Display for TopologyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooFewPoints => formatter.write_str("a hull requires at least four points"),
            Self::NonFinitePoint { index } => write!(formatter, "point {index} is not finite"),
            Self::PointNotOnUnitSphere { index, length } => {
                write!(
                    formatter,
                    "point {index} has length {length}, expected unit length"
                )
            }
            Self::DegeneratePoints => formatter.write_str("points do not define a 3D hull"),
            Self::BrokenHorizon => formatter.write_str("visible hull faces have a broken horizon"),
            Self::OpenHull => formatter.write_str("convex hull contains an unpaired half-edge"),
            Self::InvalidRadius => formatter.write_str("sphere radius must be finite and positive"),
        }
    }
}

impl std::error::Error for TopologyError {}

/// Builds both sides of the spherical topology from unit-length cell centers.
pub fn build_sphere_mesh(points: Vec<Vec3>, radius: f32) -> Result<SphereMesh, TopologyError> {
    let delaunay = SphericalDelaunay::build(points)?;
    SphereMesh::from_delaunay(&delaunay, radius)
}
