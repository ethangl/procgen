use crate::TopologyError;
use procgen_core::Vec3;

const UNIT_SPHERE_TOLERANCE: f32 = 1.0e-4;

pub(super) fn validate_points(points: &[Vec3]) -> Result<(), TopologyError> {
    if points.len() < 4 {
        return Err(TopologyError::TooFewPoints);
    }

    for (index, point) in points.iter().enumerate() {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return Err(TopologyError::NonFinitePoint { index });
        }
        let length = point.length();
        if (length - 1.0).abs() > UNIT_SPHERE_TOLERANCE {
            return Err(TopologyError::PointNotOnUnitSphere { index, length });
        }
    }

    Ok(())
}

pub(super) fn initial_tetrahedron(points: &[Vec3]) -> Result<[usize; 4], TopologyError> {
    let i0 = (0..points.len())
        .max_by(|&a, &b| points[a].x.total_cmp(&points[b].x))
        .ok_or(TopologyError::DegeneratePoints)?;
    let i1 = (0..points.len())
        .filter(|&index| index != i0)
        .max_by(|&a, &b| {
            points[a]
                .distance_squared(points[i0])
                .total_cmp(&points[b].distance_squared(points[i0]))
        })
        .ok_or(TopologyError::DegeneratePoints)?;

    let baseline = points[i1] - points[i0];
    let i2 = (0..points.len())
        .filter(|&index| index != i0 && index != i1)
        .max_by(|&a, &b| {
            baseline
                .cross(points[a] - points[i0])
                .length_squared()
                .total_cmp(&baseline.cross(points[b] - points[i0]).length_squared())
        })
        .ok_or(TopologyError::DegeneratePoints)?;

    let normal = baseline.cross(points[i2] - points[i0]);
    let (i3, signed_volume) = (0..points.len())
        .filter(|&index| index != i0 && index != i1 && index != i2)
        .map(|index| (index, normal.dot(points[index] - points[i0])))
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .ok_or(TopologyError::DegeneratePoints)?;

    if signed_volume.abs() < 1.0e-10 {
        return Err(TopologyError::DegeneratePoints);
    }
    if signed_volume > 0.0 {
        Ok([i0, i1, i2, i3])
    } else {
        Ok([i0, i2, i1, i3])
    }
}
