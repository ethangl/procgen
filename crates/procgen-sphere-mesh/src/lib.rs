//! Delaunay and Voronoi topology for points on a unit sphere.
//!
//! `SphericalDelaunay` protects its construction invariants behind accessors;
//! `SphereMesh` exposes flat arrays directly for later stages and GPU upload.

mod hull;
mod initial;
mod location;
mod mesh;
mod resolution;

pub use hull::SphericalDelaunay;
pub use location::DelaunayLocation;
pub use mesh::{
    CellCorner, SphereMesh, VoronoiEdge, connected_components, edge_cell_distances,
    multi_source_distances,
};
pub use resolution::{
    DEFAULT_CELL_COUNT, UNIT_SPHERE_AREA, default_cell_area, default_hop_length, hop_length, hops,
    mean_cell_area, mean_cell_width,
};

use procgen_core::Vec3;
use std::fmt;

const UNIT_SPHERE_TOLERANCE: f32 = 1.0e-4;

#[derive(Clone, Debug, PartialEq)]
pub enum TopologyError {
    TooFewPoints,
    NonFinitePoint { index: usize },
    PointNotOnUnitSphere { index: usize, length: f32 },
    DegeneratePoints,
    BrokenHorizon,
    InvalidRadius,
    InvalidMesh,
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
            Self::InvalidRadius => formatter.write_str("sphere radius must be finite and positive"),
            Self::InvalidMesh => formatter.write_str("sphere mesh topology is inconsistent"),
        }
    }
}

impl std::error::Error for TopologyError {}

/// Builds both sides of the spherical topology from unit-length cell centers.
pub fn build_sphere_mesh(points: Vec<Vec3>, radius: f32) -> Result<SphereMesh, TopologyError> {
    let delaunay = SphericalDelaunay::build(points)?;
    SphereMesh::from_delaunay(&delaunay, radius)
}
