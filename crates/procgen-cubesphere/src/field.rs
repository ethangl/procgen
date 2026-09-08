//! Typed CPU cube-field baking and seamless direction-based sampling.

use crate::mapping::{
    CubeFace, FaceCoordinates, MappingError, canonical_face_coordinates, direction_to_face,
    face_coordinate_derivatives, project_direction_onto_face, unit_direction,
};
use procgen_core::Vec3;
use procgen_sphere_mesh::{SphereMesh, TopologyError};
use rayon::prelude::*;
use std::array;
use std::f64::consts::PI;
use std::fmt;

const PARALLEL_ROW_THRESHOLD: usize = 8;

/// Largest power-of-two cube-field edge whose texel count fits in `u32` arithmetic.
pub const MAX_CUBE_FIELD_RESOLUTION: u32 = 1 << 15;

const _: () = assert!(MAX_CUBE_FIELD_RESOLUTION.is_power_of_two());
const _: () = assert!(
    MAX_CUBE_FIELD_RESOLUTION
        .checked_mul(MAX_CUBE_FIELD_RESOLUTION)
        .is_some()
);

/// One typed CPU face of an interpolated multi-channel field.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceField<const N: usize> {
    resolution: u32,
    texels: Vec<[f32; N]>,
}

impl<const N: usize> FaceField<N> {
    pub const fn resolution(&self) -> u32 {
        self.resolution
    }

    pub fn texels(&self) -> &[[f32; N]] {
        &self.texels
    }

    pub fn texel(&self, x: u32, y: u32) -> Option<[f32; N]> {
        let resolution = self.resolution();
        if x >= resolution || y >= resolution {
            return None;
        }
        Some(self.texels[(y * resolution + x) as usize])
    }
}

/// Six typed CPU faces of an interpolated multi-channel field.
#[derive(Clone, Debug, PartialEq)]
pub struct CubeField<const N: usize> {
    faces: [FaceField<N>; 6],
}

/// A filtered cube-field sample and each channel's derivative with respect to
/// the input direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubeFieldSample<const N: usize> {
    pub values: [f32; N],
    pub derivatives: [Vec3; N],
}

struct BilinearFilter<const N: usize> {
    lower_left: [f32; N],
    lower_right: [f32; N],
    upper_left: [f32; N],
    upper_right: [f32; N],
    tx: f32,
    ty: f32,
}

impl<const N: usize> BilinearFilter<N> {
    fn value(&self) -> [f32; N] {
        lerp(
            lerp(self.lower_left, self.lower_right, self.tx),
            lerp(self.upper_left, self.upper_right, self.tx),
            self.ty,
        )
    }

    fn gradient(&self, tx_derivative: Vec3, ty_derivative: Vec3) -> [Vec3; N] {
        array::from_fn(|channel| {
            let along_x = (self.lower_right[channel] - self.lower_left[channel]) * (1.0 - self.ty)
                + (self.upper_right[channel] - self.upper_left[channel]) * self.ty;
            let along_y = (self.upper_left[channel] - self.lower_left[channel]) * (1.0 - self.tx)
                + (self.upper_right[channel] - self.lower_right[channel]) * self.tx;
            tx_derivative * along_x + ty_derivative * along_y
        })
    }
}

impl<const N: usize> CubeField<N> {
    /// Reconstructs a cached field after validating its face dimensions and data.
    pub fn from_face_texels(
        resolution: u32,
        face_texels: [Vec<[f32; N]>; 6],
    ) -> Result<Self, BakeError> {
        validate_resolution(resolution)?;
        let expected = (resolution * resolution) as usize;
        for (face, texels) in face_texels.iter().enumerate() {
            if texels.len() != expected {
                return Err(BakeError::InvalidFaceDimensions { face });
            }
            if texels
                .iter()
                .any(|texel| !texel.iter().all(|value| value.is_finite()))
            {
                return Err(BakeError::NonFiniteFaceData { face });
            }
        }
        Ok(Self {
            faces: face_texels.map(|texels| FaceField { resolution, texels }),
        })
    }

    pub fn resolution(&self) -> u32 {
        self.faces[0].resolution()
    }

    pub fn face(&self, face: CubeFace) -> &FaceField<N> {
        &self.faces[face.index()]
    }

    /// Bilinearly samples a direction, remapping taps outside the selected face
    /// through their directions onto adjacent faces. At a corner the three
    /// incident face texels are averaged, matching seamless cube filtering.
    pub fn sample(&self, direction: Vec3) -> Result<[f32; N], MappingError> {
        Ok(self.sample_face_coordinates(direction_to_face(direction)?))
    }

    /// Bilinearly samples a direction and differentiates the selected face's
    /// filter with respect to that direction.
    ///
    /// The derivative is piecewise analytic. Like hardware cube filtering, it
    /// is undefined exactly where the dominant face or a texel interval changes;
    /// the deterministic selected side is returned at those locations.
    pub fn sample_with_derivatives(
        &self,
        direction: Vec3,
    ) -> Result<CubeFieldSample<N>, MappingError> {
        let coordinates = direction_to_face(direction)?;
        let filter = self.bilinear_filter(coordinates);
        let face_derivatives = face_coordinate_derivatives(direction, coordinates.face);
        let texel_scale = self.resolution() as f32 * 0.5;
        let derivatives = filter.gradient(
            face_derivatives.u * texel_scale,
            face_derivatives.v * texel_scale,
        );

        Ok(CubeFieldSample {
            values: filter.value(),
            derivatives,
        })
    }

    fn sample_face_coordinates(&self, coordinates: FaceCoordinates) -> [f32; N] {
        self.bilinear_filter(coordinates).value()
    }

    fn bilinear_filter(&self, coordinates: FaceCoordinates) -> BilinearFilter<N> {
        let resolution = self.resolution();
        let x = face_to_texel(coordinates.u, resolution);
        let y = face_to_texel(coordinates.v, resolution);
        let x0 = x.floor() as i64;
        let y0 = y.floor() as i64;
        let tx = x - x.floor();
        let ty = y - y.floor();

        BilinearFilter {
            lower_left: self.tap(coordinates.face, x0, y0),
            lower_right: self.tap(coordinates.face, x0 + 1, y0),
            upper_left: self.tap(coordinates.face, x0, y0 + 1),
            upper_right: self.tap(coordinates.face, x0 + 1, y0 + 1),
            tx,
            ty,
        }
    }

    fn tap(&self, source_face: CubeFace, x: i64, y: i64) -> [f32; N] {
        let edge = i64::from(self.resolution());
        if (0..edge).contains(&x) && (0..edge).contains(&y) {
            return self.face(source_face).texels[(y * edge + x) as usize];
        }
        if !(0..edge).contains(&x) && !(0..edge).contains(&y) {
            return self.corner_tap(source_face, x, y);
        }

        let coordinates = FaceCoordinates {
            face: source_face,
            u: texel_to_face(x as f32, self.resolution()),
            v: texel_to_face(y as f32, self.resolution()),
        };
        let adjacent = canonical_face_coordinates(unit_direction(coordinates));
        self.nearest_texel(adjacent)
    }

    fn corner_tap(&self, source_face: CubeFace, x: i64, y: i64) -> [f32; N] {
        let corner = unit_direction(FaceCoordinates {
            face: source_face,
            u: if x < 0 { -1.0 } else { 1.0 },
            v: if y < 0 { -1.0 } else { 1.0 },
        });
        let incident: Vec<_> = CubeFace::ALL
            .into_iter()
            .filter(|face| corner.dot(face.frame().normal) > 0.0)
            .map(|face| self.nearest_texel(project_direction_onto_face(corner, face)))
            .collect();
        debug_assert_eq!(incident.len(), 3);
        array::from_fn(|channel| {
            incident.iter().map(|texel| texel[channel]).sum::<f32>() / incident.len() as f32
        })
    }

    fn nearest_texel(&self, coordinates: FaceCoordinates) -> [f32; N] {
        let resolution = self.resolution();
        let x = face_to_texel(coordinates.u, resolution)
            .round()
            .clamp(0.0, resolution as f32 - 1.0) as u32;
        let y = face_to_texel(coordinates.v, resolution)
            .round()
            .clamp(0.0, resolution as f32 - 1.0) as u32;
        self.face(coordinates.face).texels[(y * resolution + x) as usize]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BakeError {
    InvalidResolution,
    UnsupportedCellCount { cell_count: usize },
    InvalidFaceDimensions { face: usize },
    NonFiniteFaceData { face: usize },
    InvalidMesh(TopologyError),
    CellCountMismatch { mesh: usize, cells: usize },
    NonFiniteCell { cell: usize },
}

impl fmt::Display for BakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidResolution => write!(
                formatter,
                "cube-field resolution must be a power of two between 1 and {MAX_CUBE_FIELD_RESOLUTION}"
            ),
            Self::UnsupportedCellCount { cell_count } => write!(
                formatter,
                "cannot derive a control-face resolution for {cell_count} mesh cells"
            ),
            Self::InvalidFaceDimensions { face } => {
                write!(formatter, "cube-field face {face} has invalid dimensions")
            }
            Self::NonFiniteFaceData { face } => {
                write!(formatter, "cube-field face {face} contains non-finite data")
            }
            Self::InvalidMesh(error) => write!(formatter, "invalid sphere mesh: {error}"),
            Self::CellCountMismatch { mesh, cells } => {
                write!(formatter, "mesh has {mesh} cells but field has {cells}")
            }
            Self::NonFiniteCell { cell } => {
                write!(formatter, "field channels for cell {cell} are not finite")
            }
        }
    }
}

impl std::error::Error for BakeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidMesh(error) => Some(error),
            _ => None,
        }
    }
}

/// Derives the smallest power-of-two face edge with at least four texels per
/// mean mesh-cell width.
///
/// A mean cell spans `sqrt(4 PI / cells)` radians while a cube face spans
/// `PI / 2`, so the unsnapped requirement is `sqrt(PI * cells)` texels.
/// Returns [`BakeError::UnsupportedCellCount`] when the cell count is not
/// mesh-valid or would exceed the supported cube-field resolution.
pub fn control_face_resolution(cell_count: usize) -> Result<u32, BakeError> {
    if cell_count < 4 {
        return Err(BakeError::UnsupportedCellCount { cell_count });
    }
    let required = (PI * cell_count as f64).sqrt().ceil();
    if required > f64::from(MAX_CUBE_FIELD_RESOLUTION) {
        return Err(BakeError::UnsupportedCellCount { cell_count });
    }
    Ok((required as u32).next_power_of_two())
}

/// Bakes every channel at equi-angular face texel centers.
///
/// `resolution` must be a nonzero power of two no greater than
/// [`MAX_CUBE_FIELD_RESOLUTION`]. Resolution policy belongs to the caller;
/// [`control_face_resolution`] provides the terrain-control policy.
pub fn bake_cube_field<const N: usize>(
    mesh: &SphereMesh,
    cells: &[[f32; N]],
    resolution: u32,
) -> Result<CubeField<N>, BakeError> {
    validate_resolution(resolution)?;
    mesh.validate().map_err(BakeError::InvalidMesh)?;
    if cells.len() != mesh.cell_count() {
        return Err(BakeError::CellCountMismatch {
            mesh: mesh.cell_count(),
            cells: cells.len(),
        });
    }
    if let Some(cell) = cells
        .iter()
        .position(|channels| !channels.iter().all(|value| value.is_finite()))
    {
        return Err(BakeError::NonFiniteCell { cell });
    }

    Ok(CubeField {
        faces: CubeFace::ALL.map(|face| bake_face(mesh, cells, face, resolution)),
    })
}

fn validate_resolution(resolution: u32) -> Result<(), BakeError> {
    if resolution == 0 || resolution > MAX_CUBE_FIELD_RESOLUTION || !resolution.is_power_of_two() {
        return Err(BakeError::InvalidResolution);
    }
    Ok(())
}

fn bake_face<const N: usize>(
    mesh: &SphereMesh,
    cells: &[[f32; N]],
    face: CubeFace,
    resolution: u32,
) -> FaceField<N> {
    let texels = (0..resolution)
        .into_par_iter()
        .with_min_len(PARALLEL_ROW_THRESHOLD)
        .flat_map_iter(|y| {
            let mut hint = 0;
            (0..resolution).map(move |x| {
                let direction = unit_direction(texel_center(face, x, y, resolution));
                let location = mesh.locate_delaunay(direction, hint);
                hint = location.triangle;
                weighted(location.cells.map(|cell| cells[cell]), location.weights)
            })
        })
        .collect();
    FaceField { resolution, texels }
}

fn texel_center(face: CubeFace, x: u32, y: u32, resolution: u32) -> FaceCoordinates {
    FaceCoordinates {
        face,
        u: texel_to_face(x as f32, resolution),
        v: texel_to_face(y as f32, resolution),
    }
}

fn texel_to_face(texel: f32, resolution: u32) -> f32 {
    2.0 * (texel + 0.5) / resolution as f32 - 1.0
}

fn face_to_texel(coordinate: f32, resolution: u32) -> f32 {
    (coordinate + 1.0) * 0.5 * resolution as f32 - 0.5
}

fn weighted<const N: usize>(cells: [[f32; N]; 3], weights: [f32; 3]) -> [f32; N] {
    array::from_fn(|channel| {
        // Barycentric weights sum to one, so the first weight is implied.
        let first = cells[0][channel];
        first + (cells[1][channel] - first) * weights[1] + (cells[2][channel] - first) * weights[2]
    })
}

fn lerp<const N: usize>(left: [f32; N], right: [f32; N], amount: f32) -> [f32; N] {
    array::from_fn(|channel| left[channel] + (right[channel] - left[channel]) * amount)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::face_to_direction;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::build_sphere_mesh;

    const CHANNELS: usize = 5;

    fn mesh(cell_count: usize) -> SphereMesh {
        let points = fibonacci_sphere(FibonacciConfig {
            jitter: 0.5,
            seed: 42,
            ..FibonacciConfig::new(cell_count)
        })
        .unwrap();
        build_sphere_mesh(points, 6_371.0).unwrap()
    }

    fn channels(value: f32) -> [f32; CHANNELS] {
        array::from_fn(|channel| value + channel as f32)
    }

    fn bake(
        mesh: &SphereMesh,
        cells: &[[f32; CHANNELS]],
    ) -> Result<CubeField<CHANNELS>, BakeError> {
        bake_cube_field(
            mesh,
            cells,
            control_face_resolution(mesh.cell_count()).unwrap(),
        )
    }

    fn assert_channels_close<const N: usize>(left: [f32; N], right: [f32; N], epsilon: f32) {
        for (left, right) in left.into_iter().zip(right) {
            assert!(
                (left - right).abs() <= epsilon,
                "left={left}, right={right}"
            );
        }
    }

    #[test]
    fn derives_documented_power_of_two_resolutions() {
        assert_eq!(control_face_resolution(16_384), Ok(256));
        assert_eq!(control_face_resolution(65_536), Ok(512));
        assert_eq!(control_face_resolution(4), Ok(4));
        assert_eq!(control_face_resolution(5), Ok(4));
        assert_eq!(control_face_resolution(6), Ok(8));
    }

    #[test]
    fn constant_fields_bake_every_channel() {
        let mesh = mesh(32);
        let expected = channels(0.25);
        let bake = bake(&mesh, &vec![expected; mesh.cell_count()]).unwrap();
        for face in CubeFace::ALL {
            for texel in bake.face(face).texels() {
                assert_eq!(*texel, expected);
            }
        }
    }

    #[test]
    fn bake_uses_delaunay_barycentric_interpolation() {
        let mesh = mesh(32);
        let cells: Vec<_> = (0..mesh.cell_count())
            .map(|cell| channels(cell as f32 * 0.125))
            .collect();
        let bake = bake(&mesh, &cells).unwrap();
        let (face, x, y) = (CubeFace::PositiveZ, 3, 5);
        let direction = unit_direction(texel_center(face, x, y, bake.resolution()));
        let location = mesh.locate_delaunay(direction, 0);
        let expected = weighted(location.cells.map(|cell| cells[cell]), location.weights);
        assert_eq!(bake.face(face).texel(x, y), Some(expected));
    }

    #[test]
    fn sampling_at_texel_centers_recovers_values_with_float_tolerance() {
        let mesh = mesh(32);
        let cells: Vec<_> = (0..mesh.cell_count())
            .map(|cell| channels(cell as f32 * 0.03125))
            .collect();
        let bake = bake(&mesh, &cells).unwrap();
        for face in CubeFace::ALL {
            for y in 0..bake.resolution() {
                for x in 0..bake.resolution() {
                    let direction = unit_direction(texel_center(face, x, y, bake.resolution()));
                    assert_channels_close(
                        bake.sample(direction).unwrap(),
                        bake.face(face).texel(x, y).unwrap(),
                        2.0e-5,
                    );
                }
            }
        }
    }

    #[test]
    fn filtered_derivatives_match_finite_differences_inside_one_texel_interval() {
        const STEP: f32 = 1.0e-4;
        const TOLERANCE: f32 = 2.0e-3;
        let resolution = 8;
        let faces = CubeFace::ALL.map(|face| {
            (0..resolution)
                .flat_map(|y| {
                    (0..resolution).map(move |x| {
                        let coordinates = texel_center(face, x, y, resolution);
                        [coordinates.u * 0.3 + coordinates.v * 0.7]
                    })
                })
                .collect()
        });
        let field = CubeField::from_face_texels(resolution, faces).unwrap();
        let direction = face_to_direction(FaceCoordinates {
            face: CubeFace::PositiveZ,
            u: 0.13,
            v: -0.27,
        })
        .unwrap();
        let sample = field.sample_with_derivatives(direction).unwrap();

        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            let before = field.sample(direction - axis * STEP).unwrap()[0];
            let after = field.sample(direction + axis * STEP).unwrap()[0];
            let finite_difference = (after - before) / (2.0 * STEP);
            assert!(
                (sample.derivatives[0].dot(axis) - finite_difference).abs() < TOLERANCE,
                "axis={axis:?}, analytic={}, finite={finite_difference}",
                sample.derivatives[0].dot(axis)
            );
        }
    }

    #[test]
    fn direction_filter_is_continuous_across_edges_and_corners() {
        let resolution = 64;
        let faces = CubeFace::ALL.map(|face| FaceField {
            resolution,
            texels: (0..resolution)
                .flat_map(|y| {
                    (0..resolution).map(move |x| {
                        let direction = unit_direction(texel_center(face, x, y, resolution));
                        channels(direction.x * 0.2 + direction.y * 0.3 + direction.z * 0.5)
                    })
                })
                .collect(),
        });
        let bake = CubeField { faces };

        for direction in [
            Vec3::new(1.0, 0.37, 1.0).normalized(),
            Vec3::new(-1.0, 1.0, -0.23).normalized(),
            Vec3::new(1.0, 1.0, 1.0).normalized(),
            Vec3::new(-1.0, -1.0, -1.0).normalized(),
        ] {
            let incident = CubeFace::ALL.into_iter().filter(|face| {
                direction.dot(face.frame().normal)
                    >= direction
                        .x
                        .abs()
                        .max(direction.y.abs())
                        .max(direction.z.abs())
                        - 1.0e-6
            });
            let samples: Vec<_> = incident
                .map(|face| {
                    bake.sample_face_coordinates(project_direction_onto_face(direction, face))
                })
                .collect();
            for sample in &samples[1..] {
                assert_channels_close(samples[0], *sample, 2.0e-4);
            }
        }
    }

    #[test]
    fn repeated_bakes_and_samples_are_bit_deterministic() {
        let mesh = mesh(64);
        let cells: Vec<_> = (0..mesh.cell_count())
            .map(|cell| channels((cell as f32 * 0.17).sin()))
            .collect();
        let bake_with_threads = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| bake(&mesh, &cells).unwrap())
        };
        let first = bake_with_threads(1);
        let second = bake_with_threads(4);
        assert_eq!(first, second);
        for direction in [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(1.0, 2.0, 3.0)] {
            assert_eq!(first.sample(direction), second.sample(direction));
        }
    }

    #[test]
    fn cached_face_reconstruction_validates_dimensions_and_data() {
        let faces = array::from_fn(|_| vec![channels(0.25); 16]);
        CubeField::from_face_texels(4, faces.clone()).unwrap();

        let mut invalid_dimensions = faces.clone();
        invalid_dimensions[2].pop();
        assert_eq!(
            CubeField::from_face_texels(4, invalid_dimensions),
            Err(BakeError::InvalidFaceDimensions { face: 2 })
        );

        let mut invalid_data = faces;
        invalid_data[3][7][1] = f32::NAN;
        assert_eq!(
            CubeField::from_face_texels(4, invalid_data),
            Err(BakeError::NonFiniteFaceData { face: 3 })
        );
    }

    #[test]
    fn rejects_invalid_inputs() {
        assert_eq!(
            control_face_resolution(0),
            Err(BakeError::UnsupportedCellCount { cell_count: 0 })
        );
        assert_eq!(
            control_face_resolution(usize::MAX),
            Err(BakeError::UnsupportedCellCount {
                cell_count: usize::MAX
            })
        );

        let mesh = mesh(16);
        assert_eq!(
            bake_cube_field::<CHANNELS>(&mesh, &[], 4),
            Err(BakeError::CellCountMismatch { mesh: 16, cells: 0 })
        );

        for resolution in [0, 3, MAX_CUBE_FIELD_RESOLUTION * 2] {
            assert_eq!(
                bake_cube_field(&mesh, &vec![channels(0.0); mesh.cell_count()], resolution),
                Err(BakeError::InvalidResolution)
            );
        }

        let mut non_finite = vec![channels(0.0); mesh.cell_count()];
        non_finite[4][2] = f32::NAN;
        assert_eq!(
            bake_cube_field(&mesh, &non_finite, 4),
            Err(BakeError::NonFiniteCell { cell: 4 })
        );

        let mut invalid_mesh = mesh.clone();
        invalid_mesh.cell_offsets.clear();
        assert!(matches!(
            bake_cube_field(&invalid_mesh, &vec![channels(0.0); mesh.cell_count()], 4),
            Err(BakeError::InvalidMesh(TopologyError::InvalidMesh))
        ));
        assert_eq!(
            bake_cube_field(&mesh, &vec![channels(0.0); mesh.cell_count()], 4)
                .unwrap()
                .sample(Vec3::ZERO),
            Err(MappingError::ZeroDirection)
        );
    }
}
