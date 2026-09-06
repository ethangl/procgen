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
}
