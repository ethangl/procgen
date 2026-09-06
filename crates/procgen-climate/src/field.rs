use procgen_sphere_mesh::SphereMesh;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
