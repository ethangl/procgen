//! Typed CPU control-face baking and seamless direction-based sampling.

use crate::mapping::{CubeFace, FaceCoordinates, MappingError, direction_to_face, unit_direction};
use procgen_core::Vec3;
use procgen_sphere_mesh::{SphereMesh, TopologyError};
use procgen_terrain::{TerrainCellControls, TerrainControls, TerrainStampInput};
use rayon::prelude::*;
use std::f64::consts::PI;
use std::fmt;

/// One typed CPU face of interpolated terrain controls.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlFace {
    resolution: u32,
    texels: Vec<TerrainCellControls>,
}

impl ControlFace {
    pub const fn resolution(&self) -> u32 {
        self.resolution
    }

    pub fn texels(&self) -> &[TerrainCellControls] {
        &self.texels
    }

    pub fn texel(&self, x: u32, y: u32) -> Option<TerrainCellControls> {
        if x >= self.resolution || y >= self.resolution {
            return None;
        }
        Some(self.texels[(y * self.resolution + x) as usize])
    }
}

/// Six CPU control faces plus sparse stamps retained in their source form.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlBake {
    resolution: u32,
    faces: [ControlFace; 6],
    stamps: Vec<TerrainStampInput>,
}

impl ControlBake {
    pub const fn resolution(&self) -> u32 {
        self.resolution
    }

    pub fn face(&self, face: CubeFace) -> &ControlFace {
        &self.faces[face_index(face)]
    }

    pub fn stamps(&self) -> &[TerrainStampInput] {
        &self.stamps
    }

    /// Bilinearly samples a direction, remapping taps outside the selected face
    /// through their directions onto adjacent faces. This is the CPU analogue
    /// of seamless cube filtering and applies the same rule at corners.
    pub fn sample(&self, direction: Vec3) -> Result<TerrainCellControls, MappingError> {
        let coordinates = direction_to_face(direction)?;
        Ok(self.sample_face_coordinates(coordinates))
    }

    fn sample_face_coordinates(&self, coordinates: FaceCoordinates) -> TerrainCellControls {
        let resolution = self.resolution as f32;
        let x = snap_integer((coordinates.u + 1.0) * 0.5 * resolution - 0.5);
        let y = snap_integer((coordinates.v + 1.0) * 0.5 * resolution - 0.5);
        let x0 = x.floor() as i64;
        let y0 = y.floor() as i64;
        let tx = x - x.floor();
        let ty = y - y.floor();

        let lower_left = self.tap(coordinates.face, x0, y0);
        let lower_right = self.tap(coordinates.face, x0 + 1, y0);
        let upper_left = self.tap(coordinates.face, x0, y0 + 1);
        let upper_right = self.tap(coordinates.face, x0 + 1, y0 + 1);
        lerp_controls(
            lerp_controls(lower_left, lower_right, tx),
            lerp_controls(upper_left, upper_right, tx),
            ty,
        )
    }

    fn tap(&self, source_face: CubeFace, x: i64, y: i64) -> TerrainCellControls {
        let edge = i64::from(self.resolution);
        if (0..edge).contains(&x) && (0..edge).contains(&y) {
            return self.face(source_face).texels[(y * edge + x) as usize];
        }

        let outside_x = !(0..edge).contains(&x);
        let outside_y = !(0..edge).contains(&y);
        if outside_x && outside_y {
            return self.corner_tap(source_face, x, y);
        }

        let resolution = self.resolution as f32;
        let coordinates = FaceCoordinates {
            face: source_face,
            u: 2.0 * (x as f32 + 0.5) / resolution - 1.0,
            v: 2.0 * (y as f32 + 0.5) / resolution - 1.0,
        };
        let adjacent = direction_to_face(unit_direction(coordinates))
            .expect("an extended face coordinate always produces a finite direction");
        let adjacent_x = nearest_texel(adjacent.u, self.resolution);
        let adjacent_y = nearest_texel(adjacent.v, self.resolution);
        self.face(adjacent.face).texels[(adjacent_y * edge + adjacent_x) as usize]
    }

    fn corner_tap(&self, source_face: CubeFace, x: i64, y: i64) -> TerrainCellControls {
        let corner = unit_direction(FaceCoordinates {
            face: source_face,
            u: if x < 0 { -1.0 } else { 1.0 },
            v: if y < 0 { -1.0 } else { 1.0 },
        });
        let mut incident = CubeFace::ALL
            .into_iter()
            .filter(|face| corner.dot(face.frame().normal) > 0.0)
            .map(|face| {
                let coordinates = coordinates_on_face(corner, face);
                let x = nearest_texel(coordinates.u, self.resolution);
                let y = nearest_texel(coordinates.v, self.resolution);
                let edge = i64::from(self.resolution);
                self.face(face).texels[(y * edge + x) as usize]
            });
        let first = incident.next().expect("a cube corner meets three faces");
        let second = incident.next().expect("a cube corner meets three faces");
        let third = incident.next().expect("a cube corner meets three faces");
        debug_assert!(incident.next().is_none());
        weighted_controls([first, second, third], [1.0 / 3.0; 3])
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ControlBakeError {
    InvalidCellCount,
    ResolutionOverflow,
    InvalidMesh(TopologyError),
    CellCountMismatch { mesh: usize, controls: usize },
    NonFiniteControl { cell: usize },
    InvalidStamp { stamp: usize },
}

impl fmt::Display for ControlBakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCellCount => formatter.write_str("cell count must be at least four"),
            Self::ResolutionOverflow => formatter.write_str("control-face resolution exceeds u32"),
            Self::InvalidMesh(error) => write!(formatter, "invalid sphere mesh: {error}"),
            Self::CellCountMismatch { mesh, controls } => write!(
                formatter,
                "mesh has {mesh} cells but terrain controls have {controls}"
            ),
            Self::NonFiniteControl { cell } => {
                write!(formatter, "terrain controls for cell {cell} are not finite")
            }
            Self::InvalidStamp { stamp } => write!(formatter, "terrain stamp {stamp} is invalid"),
        }
    }
}

impl std::error::Error for ControlBakeError {
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
pub fn control_face_resolution(cell_count: usize) -> Result<u32, ControlBakeError> {
    if cell_count < 4 {
        return Err(ControlBakeError::InvalidCellCount);
    }
    let required = (PI * cell_count as f64).sqrt().ceil();
    if required > f64::from(u32::MAX) {
        return Err(ControlBakeError::ResolutionOverflow);
    }
    (required as u32)
        .checked_next_power_of_two()
        .ok_or(ControlBakeError::ResolutionOverflow)
}

/// Bakes all five terrain-control channels at equi-angular face texel centers.
pub fn bake_control_faces(
    mesh: &SphereMesh,
    controls: &TerrainControls,
) -> Result<ControlBake, ControlBakeError> {
    mesh.validate().map_err(ControlBakeError::InvalidMesh)?;
    if controls.cells.len() != mesh.cell_count() {
        return Err(ControlBakeError::CellCountMismatch {
            mesh: mesh.cell_count(),
            controls: controls.cells.len(),
        });
    }
    if let Some(cell) = controls.cells.iter().position(|control| !finite(*control)) {
        return Err(ControlBakeError::NonFiniteControl { cell });
    }
    if let Some(stamp) = controls.stamps.iter().position(|stamp| {
        stamp.cell >= mesh.cell_count()
            || !stamp.position.is_finite()
            || !stamp.strength.is_finite()
            || !(0.0..=1.0).contains(&stamp.strength)
    }) {
        return Err(ControlBakeError::InvalidStamp { stamp });
    }

    let resolution = control_face_resolution(mesh.cell_count())?;
    let faces = CubeFace::ALL
        .into_par_iter()
        .map(|face| bake_face(mesh, &controls.cells, face, resolution))
        .collect::<Vec<_>>()
        .try_into()
        .expect("there are exactly six cube faces");
    Ok(ControlBake {
        resolution,
        faces,
        stamps: controls.stamps.clone(),
    })
}

fn bake_face(
    mesh: &SphereMesh,
    controls: &[TerrainCellControls],
    face: CubeFace,
    resolution: u32,
) -> ControlFace {
    let mut hint = 0;
    let mut texels = Vec::with_capacity((resolution * resolution) as usize);
    for y in 0..resolution {
        for x in 0..resolution {
            let direction = unit_direction(texel_center(face, x, y, resolution));
            let location = mesh.locate_delaunay(direction, hint);
            hint = location.triangle;
            texels.push(weighted_controls(
                location.cells.map(|cell| controls[cell]),
                location.weights,
            ));
        }
    }
    ControlFace { resolution, texels }
}

fn texel_center(face: CubeFace, x: u32, y: u32, resolution: u32) -> FaceCoordinates {
    let resolution = resolution as f32;
    FaceCoordinates {
        face,
        u: 2.0 * (x as f32 + 0.5) / resolution - 1.0,
        v: 2.0 * (y as f32 + 0.5) / resolution - 1.0,
    }
}

fn finite(control: TerrainCellControls) -> bool {
    control.base_elevation.is_finite()
        && control.detail_amplitude.is_finite()
        && control.ridge_weight.is_finite()
        && control.octave_gain.is_finite()
        && control.abyssal_amplitude.is_finite()
}

fn weighted_controls(controls: [TerrainCellControls; 3], weights: [f32; 3]) -> TerrainCellControls {
    let channel = |read: fn(TerrainCellControls) -> f32| {
        let first = read(controls[0]);
        first + (read(controls[1]) - first) * weights[1] + (read(controls[2]) - first) * weights[2]
    };
    TerrainCellControls {
        base_elevation: channel(|control| control.base_elevation),
        detail_amplitude: channel(|control| control.detail_amplitude),
        ridge_weight: channel(|control| control.ridge_weight),
        octave_gain: channel(|control| control.octave_gain),
        abyssal_amplitude: channel(|control| control.abyssal_amplitude),
    }
}

fn lerp_controls(
    left: TerrainCellControls,
    right: TerrainCellControls,
    amount: f32,
) -> TerrainCellControls {
    let weights = [1.0 - amount, amount, 0.0];
    weighted_controls([left, right, TerrainCellControls::default()], weights)
}

fn nearest_texel(coordinate: f32, resolution: u32) -> i64 {
    let index = ((coordinate + 1.0) * 0.5 * resolution as f32 - 0.5).round() as i64;
    index.clamp(0, i64::from(resolution) - 1)
}

fn coordinates_on_face(direction: Vec3, face: CubeFace) -> FaceCoordinates {
    let frame = face.frame();
    let depth = direction.dot(frame.normal);
    FaceCoordinates {
        face,
        u: (direction.dot(frame.u_axis) / depth).atan() / std::f32::consts::FRAC_PI_4,
        v: (direction.dot(frame.v_axis) / depth).atan() / std::f32::consts::FRAC_PI_4,
    }
}

fn snap_integer(value: f32) -> f32 {
    let rounded = value.round();
    if (value - rounded).abs() <= 8.0 * f32::EPSILON * value.abs().max(1.0) {
        rounded
    } else {
        value
    }
}

const fn face_index(face: CubeFace) -> usize {
    match face {
        CubeFace::PositiveX => 0,
        CubeFace::NegativeX => 1,
        CubeFace::PositiveY => 2,
        CubeFace::NegativeY => 3,
        CubeFace::PositiveZ => 4,
        CubeFace::NegativeZ => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::build_sphere_mesh;
    use procgen_terrain::{TerrainStampInput, TerrainStampKind};

    fn mesh(cell_count: usize) -> SphereMesh {
        let points = fibonacci_sphere(FibonacciConfig {
            jitter: 0.5,
            seed: 42,
            ..FibonacciConfig::new(cell_count)
        })
        .unwrap();
        build_sphere_mesh(points, 6_371.0).unwrap()
    }

    fn control(value: f32) -> TerrainCellControls {
        TerrainCellControls {
            base_elevation: value,
            detail_amplitude: value + 1.0,
            ridge_weight: value + 2.0,
            octave_gain: value + 3.0,
            abyssal_amplitude: value + 4.0,
        }
    }

    fn controls(mesh: &SphereMesh, make: impl Fn(usize) -> TerrainCellControls) -> TerrainControls {
        TerrainControls {
            cells: (0..mesh.cell_count()).map(make).collect(),
            stamps: Vec::new(),
        }
    }

    fn assert_control_close(left: TerrainCellControls, right: TerrainCellControls, epsilon: f32) {
        for (left, right) in [
            (left.base_elevation, right.base_elevation),
            (left.detail_amplitude, right.detail_amplitude),
            (left.ridge_weight, right.ridge_weight),
            (left.octave_gain, right.octave_gain),
            (left.abyssal_amplitude, right.abyssal_amplitude),
        ] {
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
    fn constant_fields_bake_all_channels_and_keep_stamps_sparse() {
        let mesh = mesh(32);
        let expected = control(0.25);
        let stamp = TerrainStampInput {
            cell: 3,
            kind: TerrainStampKind::Hotspot,
            source_index: 7,
            position: mesh.cell_centers[3],
            strength: 0.75,
        };
        let mut controls = controls(&mesh, |_| expected);
        controls.stamps.push(stamp);

        let bake = bake_control_faces(&mesh, &controls).unwrap();
        assert_eq!(bake.stamps(), &[stamp]);
        for face in CubeFace::ALL {
            assert!(
                bake.face(face)
                    .texels()
                    .iter()
                    .all(|texel| *texel == expected)
            );
        }
    }

    #[test]
    fn bake_uses_delaunay_barycentric_interpolation() {
        let mesh = mesh(32);
        let controls = controls(&mesh, |cell| control(cell as f32 * 0.125));
        let bake = bake_control_faces(&mesh, &controls).unwrap();
        let (face, x, y) = (CubeFace::PositiveZ, 3, 5);
        let direction = unit_direction(texel_center(face, x, y, bake.resolution()));
        let location = mesh.locate_delaunay(direction, 0);
        let expected = weighted_controls(
            location.cells.map(|cell| controls.cells[cell]),
            location.weights,
        );
        assert_eq!(bake.face(face).texel(x, y), Some(expected));
    }

    #[test]
    fn sampling_at_every_texel_center_returns_that_texel() {
        let mesh = mesh(32);
        let controls = controls(&mesh, |cell| control(cell as f32 * 0.03125));
        let bake = bake_control_faces(&mesh, &controls).unwrap();
        for face in CubeFace::ALL {
            for y in 0..bake.resolution() {
                for x in 0..bake.resolution() {
                    let direction = unit_direction(texel_center(face, x, y, bake.resolution()));
                    assert_eq!(
                        bake.sample(direction).unwrap(),
                        bake.face(face).texel(x, y).unwrap()
                    );
                }
            }
        }
    }

    #[test]
    fn direction_filter_is_continuous_across_edges_and_corners() {
        let resolution = 64;
        let faces = CubeFace::ALL.map(|face| ControlFace {
            resolution,
            texels: (0..resolution)
                .flat_map(|y| {
                    (0..resolution).map(move |x| {
                        let direction = unit_direction(texel_center(face, x, y, resolution));
                        control(direction.x * 0.2 + direction.y * 0.3 + direction.z * 0.5)
                    })
                })
                .collect(),
        });
        let bake = ControlBake {
            resolution,
            faces,
            stamps: Vec::new(),
        };

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
                    bake.sample_face_coordinates(super::coordinates_on_face(direction, face))
                })
                .collect();
            for sample in &samples[1..] {
                assert_control_close(samples[0], *sample, 2.0e-4);
            }
        }
    }

    #[test]
    fn repeated_bakes_and_samples_are_bit_deterministic() {
        let mesh = mesh(64);
        let controls = controls(&mesh, |cell| control((cell as f32 * 0.17).sin()));
        let bake_with_threads = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| bake_control_faces(&mesh, &controls).unwrap())
        };
        let first = bake_with_threads(1);
        let second = bake_with_threads(4);
        assert_eq!(first, second);
        for direction in [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(1.0, 2.0, 3.0)] {
            assert_eq!(first.sample(direction), second.sample(direction));
        }
    }

    #[test]
    fn rejects_invalid_inputs() {
        assert_eq!(
            control_face_resolution(0),
            Err(ControlBakeError::InvalidCellCount)
        );
        assert_eq!(
            control_face_resolution(usize::MAX),
            Err(ControlBakeError::ResolutionOverflow)
        );

        let mesh = mesh(16);
        let too_few = TerrainControls::default();
        assert_eq!(
            bake_control_faces(&mesh, &too_few),
            Err(ControlBakeError::CellCountMismatch {
                mesh: 16,
                controls: 0
            })
        );

        let mut non_finite = controls(&mesh, |_| control(0.0));
        non_finite.cells[4].ridge_weight = f32::NAN;
        assert_eq!(
            bake_control_faces(&mesh, &non_finite),
            Err(ControlBakeError::NonFiniteControl { cell: 4 })
        );

        let mut invalid_stamp = controls(&mesh, |_| control(0.0));
        invalid_stamp.stamps.push(TerrainStampInput {
            cell: mesh.cell_count(),
            kind: TerrainStampKind::Hotspot,
            source_index: 0,
            position: Vec3::X,
            strength: 1.0,
        });
        assert_eq!(
            bake_control_faces(&mesh, &invalid_stamp),
            Err(ControlBakeError::InvalidStamp { stamp: 0 })
        );

        let mut invalid_mesh = mesh.clone();
        invalid_mesh.cell_offsets.clear();
        assert!(matches!(
            bake_control_faces(&invalid_mesh, &controls(&mesh, |_| control(0.0))),
            Err(ControlBakeError::InvalidMesh(TopologyError::InvalidMesh))
        ));
        assert_eq!(
            bake_control_faces(&mesh, &controls(&mesh, |_| control(0.0)))
                .unwrap()
                .sample(Vec3::ZERO),
            Err(MappingError::ZeroDirection)
        );
    }
}
