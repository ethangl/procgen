//! Coherent plate motion fitted to a smooth global flow field.
//!
//! The field stands in for mantle convection: three channels of the
//! fixed-polynomial gradient noise, sampled at a low lattice frequency,
//! assembled into a model-space vector and projected onto the tangent plane,
//! give a smooth surface flow `v(p)`. Each plate's Euler vector is the rigid
//! rotation that best fits that flow over the plate's own cells, weighted by
//! cell area: the least-squares problem `min Σ a |ω × p − v(p)|²`, whose
//! normal equations `(Σ a (I − p pᵀ)) ω = Σ a (p × v)` are one symmetric
//! 3-by-3 solve per plate. Fitting neighbouring plates to one field is what
//! gives boundary regimes a pattern larger than a plate.
//!
//! The fit sets direction only. Speed is the plate's hashed base speed scaled
//! by crust and by plate size, because where the field does make adjacent
//! plates move together those factors are the only contrast left for a
//! boundary to clear the migration threshold with. A `coherence` fraction
//! blends the fitted axis back toward the hashed random one, so zero
//! reproduces independent random motion apart from those speed factors.
//!
//! A plate with too few cells has no rotation to fit at all: its normal
//! equations are singular, and it keeps the hashed random rotation whole.

use crate::{CrustClass, CrustClassification, PlatePartition, StageInputError};
use procgen_core::{
    RandomStream, Vec3,
    random_streams::{PLATE_ANGULAR_SPEED, PLATE_FLOW_FIELD, PLATE_ROTATION_AXIS},
};
use procgen_noise::{fold_seed_u64_to_u32, gradient_noise_3d};
use procgen_sphere_mesh::SphereMesh;
use std::fmt;

/// Fraction of the plate's area cubed below which its normal equations count
/// as singular. The matrix entries scale with plate area, so the test has to
/// be relative; the fraction is a round number well under any conditioning a
/// plate of three or more spread cells produces, and one cell makes the
/// determinant exactly zero.
const SINGULAR_DETERMINANT_FRACTION: f64 = 1.0e-6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateKinematicsConfig {
    pub seed: u64,
    pub minimum_angular_speed: f32,
    pub maximum_angular_speed: f32,
    /// Lattice frequency of the flow field, in cycles per unit direction. A
    /// flow cell has to be much larger than a plate for adjacent plates to
    /// agree, so the useful range runs well below one cycle per radius; see the
    /// measurements in `docs/plate-movement.md`.
    pub flow_frequency: f32,
    /// Fraction of the way from the hashed random axis to the fitted axis.
    /// Zero reproduces independent random motion; one is fully field-driven.
    pub coherence: f32,
    /// Speed multiplier for plates classified oceanic.
    pub oceanic_speed_factor: f32,
    /// Speed multiplier for plates classified continental.
    pub continental_speed_factor: f32,
    /// Exponent on the mean plate area over the plate's own area, so larger
    /// plates move more slowly. Zero makes size irrelevant.
    pub size_exponent: f32,
}

impl PlateKinematicsConfig {
    pub const fn new(seed: u64) -> Self {
        Self {
            seed,
            minimum_angular_speed: 0.5,
            maximum_angular_speed: 1.0,
            flow_frequency: 1.5,
            coherence: 0.85,
            oceanic_speed_factor: 1.4,
            continental_speed_factor: 0.7,
            size_exponent: 0.25,
        }
    }

    /// Upper bound on relative normal closing speed for the configured motion
    /// at the given sphere radius.
    pub fn maximum_convergence(&self, radius: f32) -> f32 {
        self.maximum_angular_speed * radius * 2.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlateKinematics {
    /// Euler rotation vector per plate. Direction is the rotation axis and
    /// magnitude is angular speed in model radians per unit time.
    pub angular_velocities: Vec<Vec3>,
}

impl PlateKinematics {
    pub fn validate(&self, partition: &PlatePartition) -> Result<(), StageInputError> {
        if self.angular_velocities.len() != partition.plate_count {
            return Err(StageInputError::Plates);
        }
        Ok(())
    }

    /// Derives the instantaneous Cartesian velocity at a point fixed to a
    /// rigidly rotating plate. The result is tangent to the sphere at `position`.
    pub fn velocity_at(&self, plate: usize, position: Vec3) -> Vec3 {
        self.angular_velocities[plate].cross(position)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlateKinematicsError {
    InvalidAngularSpeedRange,
    InvalidFlowFrequency,
    InvalidCoherence,
    InvalidCrustSpeedFactor,
    InvalidSizeExponent,
    Input(StageInputError),
}

impl fmt::Display for PlateKinematicsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAngularSpeedRange => formatter.write_str(
                "angular speeds must be finite, non-negative, and ordered minimum to maximum",
            ),
            Self::InvalidFlowFrequency => {
                formatter.write_str("flow frequency must be finite and positive")
            }
            Self::InvalidCoherence => {
                formatter.write_str("coherence must be finite and between 0 and 1")
            }
            Self::InvalidCrustSpeedFactor => formatter
                .write_str("oceanic and continental speed factors must be finite and positive"),
            Self::InvalidSizeExponent => {
                formatter.write_str("size exponent must be finite and non-negative")
            }
            Self::Input(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PlateKinematicsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StageInputError> for PlateKinematicsError {
    fn from(error: StageInputError) -> Self {
        Self::Input(error)
    }
}

/// Generates each plate's rigid-rotation vector by fitting the flow field over
/// the plate's cells, then scaling the fitted direction by a speed that
/// depends on the plate's crust and size.
pub fn generate_plate_kinematics(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    config: PlateKinematicsConfig,
) -> Result<PlateKinematics, PlateKinematicsError> {
    validate_fitted_config(config)?;
    partition.validate(mesh)?;
    crust.validate(partition)?;

    let keys = flow_field_keys(config.seed);
    let fits = fit_plate_rotations(mesh, partition, |direction| {
        flow_velocity(keys, config.flow_frequency, direction)
    });
    let reference_area = mesh.total_area() / partition.plate_count as f64;

    let angular_velocities = fits
        .iter()
        .zip(&crust.plate_classes)
        .enumerate()
        .map(|(plate, (fit, &class))| match fit {
            Some(fit) => {
                let speed = plate_speed(plate, fit.area, reference_area, class, config);
                blend_axis(
                    random_axis(plate, config),
                    fit.rotation.normalized(),
                    config.coherence,
                ) * speed
            }
            // One cell makes the normal equations exactly singular and two
            // make them numerically so: there is no rotation to fit. Saying so
            // and keeping the hashed rotation whole is the honest answer, where
            // a fixed axis or a pseudo-inverse would return a direction the
            // field never supplied and hide which plates it happened to.
            None => random_rotation(plate, config),
        })
        .collect();

    Ok(PlateKinematics { angular_velocities })
}

/// Generates independent rigid-rotation vectors per plate: a hashed uniform
/// axis and a hashed speed within the configured bounds, with no field, crust,
/// or size involved.
///
/// This is the interim source of motion for the raster tectonics pilot, which
/// has no per-plate reduction on the GPU yet. The mesh path's
/// [`generate_plate_kinematics`] reads the same rotations as the coherence
/// blend's random end and as its fallback for plates too small to fit.
pub fn generate_random_plate_kinematics(
    plate_count: usize,
    config: PlateKinematicsConfig,
) -> Result<PlateKinematics, PlateKinematicsError> {
    validate_random_config(config)?;

    Ok(PlateKinematics {
        angular_velocities: (0..plate_count)
            .map(|plate| random_rotation(plate, config))
            .collect(),
    })
}

/// Validates the rules a hashed rotation depends on. The random generator
/// checks only these, because the field, the blend, and the speed factors do
/// not reach its output.
fn validate_random_config(config: PlateKinematicsConfig) -> Result<(), PlateKinematicsError> {
    if !config.minimum_angular_speed.is_finite()
        || !config.maximum_angular_speed.is_finite()
        || config.minimum_angular_speed < 0.0
        || config.minimum_angular_speed > config.maximum_angular_speed
    {
        return Err(PlateKinematicsError::InvalidAngularSpeedRange);
    }
    Ok(())
}

/// Validates every rule the fitted generator's output depends on.
fn validate_fitted_config(config: PlateKinematicsConfig) -> Result<(), PlateKinematicsError> {
    validate_random_config(config)?;
    if !config.flow_frequency.is_finite() || config.flow_frequency <= 0.0 {
        return Err(PlateKinematicsError::InvalidFlowFrequency);
    }
    if !config.coherence.is_finite() || !(0.0..=1.0).contains(&config.coherence) {
        return Err(PlateKinematicsError::InvalidCoherence);
    }
    if !config.oceanic_speed_factor.is_finite()
        || config.oceanic_speed_factor <= 0.0
        || !config.continental_speed_factor.is_finite()
        || config.continental_speed_factor <= 0.0
    {
        return Err(PlateKinematicsError::InvalidCrustSpeedFactor);
    }
    if !config.size_exponent.is_finite() || config.size_exponent < 0.0 {
        return Err(PlateKinematicsError::InvalidSizeExponent);
    }
    Ok(())
}

fn random_axis(plate: usize, config: PlateKinematicsConfig) -> Vec3 {
    RandomStream::new(config.seed, PLATE_ROTATION_AXIS).unit_vector(plate as u64)
}

fn random_speed(plate: usize, config: PlateKinematicsConfig) -> f32 {
    let speeds = RandomStream::new(config.seed, PLATE_ANGULAR_SPEED);
    config.minimum_angular_speed
        + speeds.unit_f32(plate as u64, 0)
            * (config.maximum_angular_speed - config.minimum_angular_speed)
}

fn random_rotation(plate: usize, config: PlateKinematicsConfig) -> Vec3 {
    random_axis(plate, config) * random_speed(plate, config)
}

/// Blends the hashed axis toward the fitted one. Both ends are unit vectors,
/// so normalizing the linear blend lands on the arc between them.
fn blend_axis(random: Vec3, fitted: Vec3, coherence: f32) -> Vec3 {
    (random + (fitted - random) * coherence).normalized()
}

fn plate_speed(
    plate: usize,
    plate_area: f64,
    reference_area: f64,
    class: CrustClass,
    config: PlateKinematicsConfig,
) -> f32 {
    let crust_factor = match class {
        CrustClass::Oceanic => config.oceanic_speed_factor,
        CrustClass::Continental => config.continental_speed_factor,
    };
    // The one transcendental on this path. Kinematics is a float output that is
    // never pinned, and the raster pipeline quantizes angular velocities before
    // upload, so a configurable exponent costs nothing a kernel can observe.
    let size_factor = (reference_area / plate_area).powf(f64::from(config.size_exponent)) as f32;
    (random_speed(plate, config) * crust_factor * size_factor)
        .clamp(config.minimum_angular_speed, config.maximum_angular_speed)
}

/// Noise keys for the flow field's three model-space channels. Folding a
/// per-channel draw from the field's own stream, rather than the seed plus an
/// offset, keeps the field independent of the crack walk's arc noise, which
/// folds its own seed the same way and shares this seed's value by default.
fn flow_field_keys(seed: u64) -> [u32; 3] {
    let stream = RandomStream::new(seed, PLATE_FLOW_FIELD);
    [0, 1, 2].map(|channel| fold_seed_u64_to_u32(stream.sample_u64(channel, 0)))
}

/// Samples the flow field at a unit direction. The three noise channels form a
/// model-space vector, projected onto the tangent plane so what a plate is
/// fitted to is a surface flow.
fn flow_velocity(keys: [u32; 3], frequency: f32, direction: Vec3) -> Vec3 {
    let position = direction * frequency;
    let sampled = Vec3::new(
        gradient_noise_3d(keys[0], position).value,
        gradient_noise_3d(keys[1], position).value,
        gradient_noise_3d(keys[2], position).value,
    );
    sampled - direction * sampled.dot(direction)
}

/// One plate's fitted rotation and the area it was fitted over.
struct PlateRotationFit {
    /// Least-squares Euler vector. Only its direction reaches the output.
    rotation: Vec3,
    area: f64,
}

/// Area-weighted normal equations `matrix * ω = vector` for one plate.
#[derive(Clone, Copy, Default)]
struct NormalEquations {
    matrix: [[f64; 3]; 3],
    vector: [f64; 3],
    area: f64,
}

impl NormalEquations {
    fn accumulate(&mut self, area: f64, direction: Vec3, velocity: Vec3) {
        let point = [direction.x, direction.y, direction.z].map(f64::from);
        for row in 0..3 {
            for column in 0..3 {
                let identity = f64::from(row == column);
                self.matrix[row][column] += area * (identity - point[row] * point[column]);
            }
        }
        let moment = direction.cross(velocity);
        for (entry, component) in self
            .vector
            .iter_mut()
            .zip([moment.x, moment.y, moment.z].map(f64::from))
        {
            *entry += area * component;
        }
        self.area += area;
    }

    fn solve(&self) -> Option<PlateRotationFit> {
        let rotation = solve_symmetric_3x3(self.matrix, self.vector, self.area)?;
        Some(PlateRotationFit {
            rotation: Vec3::new(rotation[0] as f32, rotation[1] as f32, rotation[2] as f32),
            area: self.area,
        })
    }
}

/// Fits one rigid rotation per plate to `field`, in one pass over the cells.
/// `None` marks a plate whose normal equations are singular.
fn fit_plate_rotations(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    field: impl Fn(Vec3) -> Vec3,
) -> Vec<Option<PlateRotationFit>> {
    let mut equations = vec![NormalEquations::default(); partition.plate_count];
    for (cell, &plate) in partition.cell_plates.iter().enumerate() {
        let direction = mesh.cell_centers[cell].normalized();
        equations[plate].accumulate(
            f64::from(mesh.cell_areas[cell]),
            direction,
            field(direction),
        );
    }
    equations.iter().map(NormalEquations::solve).collect()
}

/// Solves a symmetric 3-by-3 system by Cramer's rule, rejecting a determinant
/// too small for the system's scale to carry a direction.
fn solve_symmetric_3x3(matrix: [[f64; 3]; 3], vector: [f64; 3], scale: f64) -> Option<[f64; 3]> {
    let determinant = determinant_3x3(matrix);
    if determinant.abs() <= SINGULAR_DETERMINANT_FRACTION * scale * scale * scale {
        return None;
    }
    let mut solution = [0.0; 3];
    for (column, entry) in solution.iter_mut().enumerate() {
        let mut replaced = matrix;
        for (row, replacement) in vector.iter().enumerate() {
            replaced[row][column] = *replacement;
        }
        *entry = determinant_3x3(replaced) / determinant;
    }
    Some(solution)
}

fn determinant_3x3(m: [[f64; 3]; 3]) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CrustClassificationConfig, classify_crust, test_support};

    /// Normalizing the blended axis leaves the rotation vector's magnitude a
    /// few last bits off the clamped speed it was scaled by.
    const SPEED_TOLERANCE: f32 = 1.0e-6;

    fn reference_inputs() -> (SphereMesh, PlatePartition, CrustClassification) {
        let (mesh, partition) = test_support::reference_partition();
        let crust = classify_crust(&mesh, &partition, CrustClassificationConfig::new(17)).unwrap();
        (mesh, partition, crust)
    }

    /// Splits a mesh into two plates of nearly equal area by handing each cell
    /// to whichever plate is smaller so far, so comparing the two isolates the
    /// crust factor from the size factor.
    fn two_equal_plates() -> (SphereMesh, PlatePartition) {
        let mesh = test_support::mesh(64);
        let mut areas = [0.0_f64; 2];
        let cell_plates = mesh
            .cell_areas
            .iter()
            .map(|&area| {
                let plate = usize::from(areas[0] > areas[1]);
                areas[plate] += f64::from(area);
                plate
            })
            .collect();
        (
            mesh,
            PlatePartition {
                cell_plates,
                plate_count: 2,
            },
        )
    }

    fn plate_areas(mesh: &SphereMesh, partition: &PlatePartition) -> Vec<f64> {
        let mut areas = vec![0.0; partition.plate_count];
        for (&plate, &area) in partition.cell_plates.iter().zip(&mesh.cell_areas) {
            areas[plate] += f64::from(area);
        }
        areas
    }

    fn speeds(kinematics: &PlateKinematics) -> Vec<f32> {
        kinematics
            .angular_velocities
            .iter()
            .map(|velocity| velocity.length())
            .collect()
    }

    #[test]
    fn fitted_kinematics_are_deterministic_seeded_and_bounded() {
        let (mesh, partition, crust) = reference_inputs();
        let config = PlateKinematicsConfig::new(7);
        let first = generate_plate_kinematics(&mesh, &partition, &crust, config).unwrap();

        assert_eq!(
            first,
            generate_plate_kinematics(&mesh, &partition, &crust, config).unwrap()
        );
        assert_ne!(
            first,
            generate_plate_kinematics(&mesh, &partition, &crust, PlateKinematicsConfig::new(8))
                .unwrap()
        );
        assert_eq!(first.angular_velocities.len(), partition.plate_count);
        assert!(speeds(&first).iter().all(|&speed| {
            (config.minimum_angular_speed - SPEED_TOLERANCE
                ..=config.maximum_angular_speed + SPEED_TOLERANCE)
                .contains(&speed)
        }));
    }

    #[test]
    fn zero_coherence_keeps_every_hashed_axis() {
        let (mesh, partition, crust) = reference_inputs();
        let config = PlateKinematicsConfig {
            coherence: 0.0,
            ..PlateKinematicsConfig::new(7)
        };
        let kinematics = generate_plate_kinematics(&mesh, &partition, &crust, config).unwrap();

        for (plate, velocity) in kinematics.angular_velocities.iter().enumerate() {
            let expected = random_axis(plate, config);
            assert!(
                (velocity.normalized() - expected).length() < 1.0e-6,
                "plate {plate} left its hashed axis"
            );
        }
    }

    #[test]
    fn full_coherence_fits_one_global_rotation_and_passes_its_axis_through() {
        // A field that is itself a rigid rotation is the case where the fit has
        // an exact answer, so every plate recovers the same axis whichever
        // cells it owns.
        let (mesh, partition) = two_equal_plates();
        let rotation = Vec3::new(0.3, -0.7, 0.5);
        let fits = fit_plate_rotations(&mesh, &partition, |direction| rotation.cross(direction));
        let fitted: Vec<_> = fits
            .iter()
            .map(|fit| {
                fit.as_ref()
                    .expect("both plates span enough cells")
                    .rotation
            })
            .collect();

        for (plate, &fit) in fitted.iter().enumerate() {
            assert!(
                (fit - rotation).length() < 1.0e-4,
                "plate {plate} fitted {fit:?}"
            );
        }
        assert!(fitted[0].normalized().dot(fitted[1].normalized()) > 1.0 - 1.0e-6);

        // At full coherence the blend is the fitted axis itself, so the flow
        // field is the only thing setting direction.
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Oceanic; partition.plate_count],
        };
        let config = PlateKinematicsConfig {
            coherence: 1.0,
            ..PlateKinematicsConfig::new(7)
        };
        let kinematics = generate_plate_kinematics(&mesh, &partition, &crust, config).unwrap();
        let flow_fits = fit_plate_rotations(&mesh, &partition, |direction| {
            flow_velocity(
                flow_field_keys(config.seed),
                config.flow_frequency,
                direction,
            )
        });
        for (plate, velocity) in kinematics.angular_velocities.iter().enumerate() {
            let expected = flow_fits[plate].as_ref().unwrap().rotation.normalized();
            assert!(
                (velocity.normalized() - expected).length() < 1.0e-6,
                "plate {plate} left its fitted axis"
            );
        }
    }

    #[test]
    fn oceanic_plates_outrun_continental_plates_of_similar_size() {
        let (mesh, partition) = two_equal_plates();
        let areas = plate_areas(&mesh, &partition);
        assert!(
            (areas[0] - areas[1]).abs() / areas[0] < 0.01,
            "the halves must be similar in size: {areas:?}"
        );

        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Oceanic, CrustClass::Continental],
        };
        let kinematics =
            generate_plate_kinematics(&mesh, &partition, &crust, PlateKinematicsConfig::new(7))
                .unwrap();

        let speeds = speeds(&kinematics);
        assert!(speeds[0] > speeds[1], "{speeds:?}");
    }

    #[test]
    fn larger_plates_move_more_slowly_than_smaller_ones() {
        let mesh = test_support::mesh(64);
        // A minor plate of six cells beside one holding the rest. Both span
        // enough cells to fit, both are oceanic, and the speed range is wide
        // enough that nothing clamps, so size is the only factor that differs.
        let minor_cells = 6;
        let cell_plates = (0..mesh.cell_count())
            .map(|cell| usize::from(cell >= mesh.cell_count() - minor_cells))
            .collect();
        let partition = PlatePartition {
            cell_plates,
            plate_count: 2,
        };
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Oceanic; 2],
        };
        let config = PlateKinematicsConfig {
            minimum_angular_speed: 0.0,
            maximum_angular_speed: 100.0,
            ..PlateKinematicsConfig::new(7)
        };

        let areas = plate_areas(&mesh, &partition);
        assert!(areas[0] > areas[1] * 5.0, "{areas:?}");
        let kinematics = generate_plate_kinematics(&mesh, &partition, &crust, config).unwrap();
        let factors: Vec<_> = speeds(&kinematics)
            .iter()
            .enumerate()
            .map(|(plate, &speed)| {
                speed / (random_speed(plate, config) * config.oceanic_speed_factor)
            })
            .collect();

        assert!(
            factors[0] < 1.0,
            "the larger plate is held back: {factors:?}"
        );
        assert!(
            factors[1] > 1.0,
            "the smaller plate is pushed on: {factors:?}"
        );
    }

    #[test]
    fn single_cell_plates_fall_back_to_the_hashed_rotation() {
        let (mesh, _, partition) = test_support::two_plate_boundary_partition();
        let crust = CrustClassification {
            plate_classes: vec![CrustClass::Oceanic; 2],
        };
        let config = PlateKinematicsConfig::new(7);

        let fits = fit_plate_rotations(&mesh, &partition, |direction| {
            flow_velocity(
                flow_field_keys(config.seed),
                config.flow_frequency,
                direction,
            )
        });
        assert!(fits[1].is_none(), "a one-cell plate has no rotation to fit");

        let kinematics = generate_plate_kinematics(&mesh, &partition, &crust, config).unwrap();
        assert_eq!(kinematics.angular_velocities[1], random_rotation(1, config));
    }

    #[test]
    fn random_kinematics_stay_independent_and_bounded() {
        let config = PlateKinematicsConfig::new(17);
        let first = generate_random_plate_kinematics(12, config).unwrap();

        assert_eq!(first, generate_random_plate_kinematics(12, config).unwrap());
        assert_ne!(
            first,
            generate_random_plate_kinematics(12, PlateKinematicsConfig::new(18)).unwrap()
        );
        assert!(first.angular_velocities.iter().all(|velocity| {
            (config.minimum_angular_speed..=config.maximum_angular_speed)
                .contains(&velocity.length())
        }));
    }

    #[test]
    fn local_velocity_is_tangent_and_scales_with_radius() {
        let kinematics = PlateKinematics {
            angular_velocities: vec![Vec3::new(0.0, 2.0, 0.0)],
        };
        let unit_position = Vec3::new(1.0, 0.0, 0.0);
        let unit_velocity = kinematics.velocity_at(0, unit_position);
        let double_velocity = kinematics.velocity_at(0, unit_position * 2.0);

        assert_eq!(unit_velocity, Vec3::new(0.0, 0.0, -2.0));
        assert_eq!(double_velocity, unit_velocity * 2.0);
        assert_eq!(unit_velocity.dot(unit_position), 0.0);
    }

    #[test]
    fn maximum_convergence_accounts_for_both_sides() {
        let config = PlateKinematicsConfig {
            maximum_angular_speed: 2.0,
            ..PlateKinematicsConfig::new(17)
        };

        assert_eq!(config.maximum_convergence(3.0), 12.0);
    }

    #[test]
    fn flow_field_velocity_is_tangent_and_follows_the_seed() {
        let direction = Vec3::new(0.3, 0.5, -0.8).normalized();
        let keys = flow_field_keys(7);
        let velocity = flow_velocity(keys, 1.5, direction);

        assert!(velocity.dot(direction).abs() < 1.0e-6);
        assert!(velocity.length() > 0.0);
        assert_ne!(keys, flow_field_keys(8));
        assert_ne!(velocity, flow_velocity(flow_field_keys(8), 1.5, direction));
    }

    #[test]
    fn rejects_invalid_configuration() {
        let (mesh, partition, crust) = reference_inputs();
        let base = PlateKinematicsConfig::new(7);
        let cases = [
            (
                PlateKinematicsConfig {
                    minimum_angular_speed: 1.0,
                    maximum_angular_speed: 0.5,
                    ..base
                },
                PlateKinematicsError::InvalidAngularSpeedRange,
            ),
            (
                PlateKinematicsConfig {
                    minimum_angular_speed: -0.1,
                    ..base
                },
                PlateKinematicsError::InvalidAngularSpeedRange,
            ),
            (
                PlateKinematicsConfig {
                    maximum_angular_speed: f32::NAN,
                    ..base
                },
                PlateKinematicsError::InvalidAngularSpeedRange,
            ),
            (
                PlateKinematicsConfig {
                    flow_frequency: 0.0,
                    ..base
                },
                PlateKinematicsError::InvalidFlowFrequency,
            ),
            (
                PlateKinematicsConfig {
                    coherence: 1.5,
                    ..base
                },
                PlateKinematicsError::InvalidCoherence,
            ),
            (
                PlateKinematicsConfig {
                    oceanic_speed_factor: 0.0,
                    ..base
                },
                PlateKinematicsError::InvalidCrustSpeedFactor,
            ),
            (
                PlateKinematicsConfig {
                    continental_speed_factor: f32::INFINITY,
                    ..base
                },
                PlateKinematicsError::InvalidCrustSpeedFactor,
            ),
            (
                PlateKinematicsConfig {
                    size_exponent: -1.0,
                    ..base
                },
                PlateKinematicsError::InvalidSizeExponent,
            ),
        ];

        for (config, expected) in cases {
            assert_eq!(
                generate_plate_kinematics(&mesh, &partition, &crust, config),
                Err(expected),
                "{config:?}"
            );
        }
    }

    #[test]
    fn rejects_mismatched_inputs() {
        let (mesh, partition, crust) = reference_inputs();
        let config = PlateKinematicsConfig::new(7);

        assert_eq!(
            generate_plate_kinematics(
                &mesh,
                &PlatePartition {
                    cell_plates: partition.cell_plates[1..].to_vec(),
                    plate_count: partition.plate_count,
                },
                &crust,
                config,
            ),
            Err(PlateKinematicsError::Input(StageInputError::Cells))
        );
        assert_eq!(
            generate_plate_kinematics(
                &mesh,
                &partition,
                &CrustClassification {
                    plate_classes: crust.plate_classes[1..].to_vec(),
                },
                config,
            ),
            Err(PlateKinematicsError::Input(StageInputError::Plates))
        );
    }
}
