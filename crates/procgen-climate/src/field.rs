use procgen_sphere_mesh::SphereMesh;
use procgen_tectonics::is_land;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Land,
    Ocean,
}

impl Surface {
    pub fn from_elevation(elevation: f32) -> Self {
        if is_land(elevation) {
            Self::Land
        } else {
            Self::Ocean
        }
    }
}

impl fmt::Display for Surface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Land => formatter.write_str("land"),
            Self::Ocean => formatter.write_str("ocean"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AreaWeightedSummary {
    pub minimum: f32,
    pub maximum: f32,
    pub area_weighted_mean: f64,
}

impl AreaWeightedSummary {
    pub fn from_field(mesh: &SphereMesh, values: &[f32]) -> Self {
        Self {
            minimum: values.iter().copied().fold(f32::INFINITY, f32::min),
            maximum: values.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            area_weighted_mean: mesh.area_weighted_mean(values),
        }
    }

    pub fn is_finite(self) -> bool {
        self.minimum.is_finite() && self.maximum.is_finite() && self.area_weighted_mean.is_finite()
    }
}

pub(crate) fn area_weighted_rms_difference(mesh: &SphereMesh, left: &[f32], right: &[f32]) -> f64 {
    (left
        .iter()
        .zip(right)
        .zip(&mesh.cell_areas)
        .map(|((&left, &right), &area)| f64::from(left - right).powi(2) * f64::from(area))
        .sum::<f64>()
        / mesh.total_area())
    .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_tectonics::SEA_LEVEL;

    #[test]
    fn surface_uses_the_authoritative_elevation_boundary() {
        assert_eq!(Surface::from_elevation(SEA_LEVEL), Surface::Ocean);
        assert_eq!(
            Surface::from_elevation(f32::from_bits(SEA_LEVEL.to_bits() + 1)),
            Surface::Land
        );
    }

    #[test]
    fn finiteness_checks_every_aggregate() {
        assert!(AreaWeightedSummary::default().is_finite());
        for summary in [
            AreaWeightedSummary {
                minimum: f32::NAN,
                ..Default::default()
            },
            AreaWeightedSummary {
                maximum: f32::INFINITY,
                ..Default::default()
            },
            AreaWeightedSummary {
                area_weighted_mean: f64::NAN,
                ..Default::default()
            },
        ] {
            assert!(!summary.is_finite());
        }
    }

    #[test]
    fn rms_difference_uses_spherical_cell_area() {
        use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
        use procgen_sphere_mesh::SphericalDelaunay;

        let points = fibonacci_sphere(FibonacciConfig::new(32)).unwrap();
        let delaunay = SphericalDelaunay::build(points).unwrap();
        let mesh = SphereMesh::from_delaunay(&delaunay, 1.0).unwrap();
        let zeros = vec![0.0; mesh.cell_count()];
        let ones = vec![1.0; mesh.cell_count()];

        assert_eq!(area_weighted_rms_difference(&mesh, &zeros, &zeros), 0.0);
        assert!((area_weighted_rms_difference(&mesh, &zeros, &ones) - 1.0).abs() < 1.0e-7);
    }
}
