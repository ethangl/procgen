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
//! The fit sets direction only. What sets speed is [`crate::speed`], and this
//! stage can apply only half of that rule: [`crate::plate_speed`] reads the
//! share of a plate's perimeter that is subducting slab, and a trench has a
//! side only once the ocean floor has ages, which the crust-birth prior gives
//! it after this stage runs. So the fitted axes are scaled by the crust factor
//! alone, and evolution applies the slab factor before its first step moves
//! anything. A `coherence` fraction blends the fitted axis back toward the
//! hashed random one, so zero reproduces independent random motion apart from
//! that factor.
//!
//! A plate with too few cells has no rotation to fit at all: its normal
//! equations are singular, and it keeps the hashed axis. Only direction falls
//! back; the speed rule is the same for every plate.
//!
//! Everything here stays on add, multiply, divide, and square root, so no libm
//! call sits between the field and the integer boundary classes the angular
//! velocities decide.
//!
//! What this stage produces is one classification, and through it the
//! crust-birth prior and the subducting fractions a run's first respeed reads.
//! Evolution drifts and respeeds its own copy of the motion step by step and
//! returns the motion it ended on, which is what every consumer downstream of
//! a run reads.

use crate::{CrustClassification, PlatePartition, StageInputError, speed::crust_scaled_speed};
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
    /// Bottom of the range each plate's hashed base speed is drawn from. It
    /// bounds the draw alone and not the speed that comes out: a plate with a
    /// continent on it and no slab to pull it is slower than this, which is
    /// the point of [`plate_speed`].
    pub minimum_angular_speed: f32,
    /// Top of that range, and the ceiling [`plate_speed`] clamps its answer
    /// to, so it is the fastest plate a world can hold and not merely the top
    /// of the hashed draw. Must be positive: at zero every plate is fitted at
    /// rest, every boundary is interior, and the stages that measure a length
    /// against a plate speed have nothing to divide by.
    /// [`crate::derive_crust_birth_prior`] reads it as the speed one hop of
    /// its walk is crossed at, and [`crate::maximum_step_duration`] as the
    /// fastest plate a step has to stay inside the transport reach for.
    pub maximum_angular_speed: f32,
    /// Lattice frequency of the flow field, in cycles per unit direction. A
    /// flow cell has to be much larger than a plate for adjacent plates to
    /// agree, so raising this past the default costs coherence quickly; see the
    /// measurements in `docs/plate-movement.md`.
    pub flow_frequency: f32,
    /// Fraction of the way from the hashed random axis to the fitted axis.
    /// Zero reproduces independent random motion; one is fully field-driven.
    pub coherence: f32,
    /// Speed multiplier for a plate that carries no continental crust.
    pub oceanic_speed_factor: f32,
    /// Speed multiplier for a plate that is continental throughout. A plate
    /// that carries both crusts interpolates between the two factors by its
    /// continental area fraction.
    pub continental_speed_factor: f32,
    /// Share of its base speed a plate with no subducting slab keeps. Slab
    /// pull is what spreads plate speeds over a factor of ten on Earth, so
    /// this is the bottom of that spread rather than a correction to it.
    /// Must lie in `(0, 1]`; at one the slab factor is gone.
    pub trenchless_speed_factor: f32,
    /// Share of a plate's boundary edges that must be subducting slab for the
    /// pull to be felt in full. The Pacific's trench share is about the
    /// default. Must lie in `(0, 1]`.
    pub slab_saturation_fraction: f32,
}

impl PlateKinematicsConfig {
    pub const fn new(seed: u64) -> Self {
        Self {
            seed,
            minimum_angular_speed: 0.5,
            maximum_angular_speed: 1.0,
            flow_frequency: 1.0,
            coherence: 0.85,
            oceanic_speed_factor: 1.5,
            continental_speed_factor: 1.0,
            trenchless_speed_factor: 0.25,
            slab_saturation_fraction: 0.4,
        }
    }

    /// Upper bound on relative normal closing speed for the configured motion
    /// at the given sphere radius.
    pub fn maximum_convergence(&self, radius: f32) -> f32 {
        self.maximum_angular_speed * radius * 2.0
    }
}

/// The smooth global surface flow the kinematics fit reads, and the same field
/// base elevation takes its dynamic topography from.
///
/// Three channels of the fixed-polynomial gradient noise at
/// [`PlateKinematicsConfig::flow_frequency`], assembled into a model-space
/// vector and projected onto the tangent plane, stand in for mantle
/// convection. Building it from the config once and handing it to both
/// consumers is what keeps them reading one field: the fit turns it into plate
/// motion and base elevation turns its divergence into broad sag and swell,
/// and neither may sample a field the other does not see.
///
/// Sampling stays on add and multiply alone, so no libm call sits between the
/// field and the integer boundary classes the fitted velocities decide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlowField {
    /// Noise keys for the three model-space channels. Folding a per-channel
    /// draw from the field's own stream, rather than the seed plus an offset,
    /// keeps the field independent of the crack walk's arc noise, which folds
    /// its own seed the same way and shares this seed's value by default.
    keys: [u32; 3],
    frequency: f32,
}

impl FlowField {
    pub fn new(config: &PlateKinematicsConfig) -> Self {
        let stream = RandomStream::new(config.seed, PLATE_FLOW_FIELD);
        Self {
            keys: [0, 1, 2].map(|channel| fold_seed_u64_to_u32(stream.sample_u64(channel, 0))),
            frequency: config.flow_frequency,
        }
    }

    /// Samples the field at a unit direction. The three noise channels form a
    /// model-space vector, projected onto the tangent plane so what a plate is
    /// fitted to is a surface flow.
    pub fn velocity_at(&self, direction: Vec3) -> Vec3 {
        let position = direction * self.frequency;
        let sampled = Vec3::new(
            gradient_noise_3d(self.keys[0], position).value,
            gradient_noise_3d(self.keys[1], position).value,
            gradient_noise_3d(self.keys[2], position).value,
        );
        sampled - direction * sampled.dot(direction)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlateKinematics {
    /// Euler rotation vector per plate. Direction is the rotation axis and
    /// magnitude is angular speed in model radians per unit time.
    pub angular_velocities: Vec<Vec3>,
    /// The hashed base speed each plate's own speed is a multiple of, carried
    /// beside the motion because [`plate_speed`] is recomputed from it every
    /// evolution step. It is the draw itself, so a plate keeps it whatever its
    /// trenches and its crust then do to the speed it turns at.
    pub base_speeds: Vec<f32>,
}

impl PlateKinematics {
    pub fn validate(&self, partition: &PlatePartition) -> Result<(), StageInputError> {
        if self.angular_velocities.len() != partition.plate_count
            || self.base_speeds.len() != partition.plate_count
        {
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
    InvalidTrenchlessSpeedFactor,
    InvalidSlabSaturationFraction,
    Input(StageInputError),
}

impl fmt::Display for PlateKinematicsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAngularSpeedRange => formatter.write_str(
                "angular speeds must be finite and ordered minimum to maximum, \
                 with a non-negative minimum and a positive maximum",
            ),
            Self::InvalidFlowFrequency => {
                formatter.write_str("flow frequency must be finite and positive")
            }
            Self::InvalidCoherence => {
                formatter.write_str("coherence must be finite and between 0 and 1")
            }
            Self::InvalidCrustSpeedFactor => formatter
                .write_str("oceanic and continental speed factors must be finite and positive"),
            Self::InvalidTrenchlessSpeedFactor => {
                formatter.write_str("trenchless speed factor must lie in (0, 1]")
            }
            Self::InvalidSlabSaturationFraction => {
                formatter.write_str("slab saturation fraction must lie in (0, 1]")
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
/// the plate's cells, then scaling the fitted direction by
/// [`crust_scaled_speed`].
///
/// This is not the speed a plate turns at. [`plate_speed`] is, and it reads a
/// subducting fraction that nothing can supply yet: a trench has a side only
/// once the ocean floor has ages, and the crust-birth prior that dates it runs
/// after this stage and reads the boundaries this motion implies. So the stage
/// stops at the crust factor, and evolution applies the slab factor before its
/// first step moves anything.
///
/// What this motion is for, therefore, is one classification: the one its
/// caller takes from it, which the prior reads to find its ridges and which
/// [`crate::evolve_plate_ownership`] reads once to reach the fractions its own
/// first respeed needs.
pub fn generate_plate_kinematics(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: &CrustClassification,
    config: PlateKinematicsConfig,
) -> Result<PlateKinematics, PlateKinematicsError> {
    validate_config(config)?;
    partition.validate(mesh)?;
    crust.validate(mesh)?;

    let continental = crust.plate_continental_fraction(mesh, partition);
    let areas = partition.plate_areas(mesh);
    let flow = FlowField::new(&config);
    let fitted = fit_plate_rotations(mesh, partition, &areas, |direction| {
        flow.velocity_at(direction)
    });

    let base_speeds: Vec<f32> = (0..partition.plate_count)
        .map(|plate| random_speed(plate, config))
        .collect();
    let angular_velocities = fitted
        .iter()
        .zip(&base_speeds)
        .zip(&continental)
        .enumerate()
        .map(|(plate, ((fitted, &base), &continental))| {
            let random = random_axis(plate, config);
            let axis = match fitted {
                Some(rotation) => blend_axis(random, rotation.normalized(), config.coherence),
                // One cell makes the normal equations exactly singular and two
                // make them numerically so: there is no rotation to fit. Saying
                // so and keeping the hashed axis is the honest answer, where a
                // fixed axis or a pseudo-inverse would return a direction the
                // field never supplied and hide which plates it happened to.
                // Only direction falls back; speed follows the same rule for
                // every plate.
                None => random,
            };
            axis * crust_scaled_speed(base, continental, config)
        })
        .collect();

    Ok(PlateKinematics {
        angular_velocities,
        base_speeds,
    })
}

/// The one place the motion config's rules live. It is `pub(crate)` because
/// the crust-birth prior scales a hop by `maximum_angular_speed` and so has to
/// hold the config to the same rules this stage does, rather than keep a
/// second, weaker copy of them.
pub(crate) fn validate_config(config: PlateKinematicsConfig) -> Result<(), PlateKinematicsError> {
    // The maximum has to be positive, not merely non-negative: a maximum of
    // zero fits every plate at rest, which leaves every boundary interior and
    // every stage that scales a length against a plate speed with nothing to
    // divide by. The crust-birth prior is one of those.
    if !config.minimum_angular_speed.is_finite()
        || !config.maximum_angular_speed.is_finite()
        || config.minimum_angular_speed < 0.0
        || config.maximum_angular_speed <= 0.0
        || config.minimum_angular_speed > config.maximum_angular_speed
    {
        return Err(PlateKinematicsError::InvalidAngularSpeedRange);
    }
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
    // Both are shares of something, so both stop at one: a trenchless plate
    // faster than its base, or a slab that saturates past a whole perimeter,
    // would each be a factor whose name no longer describes it.
    // Written as one comparison each rather than a range test, because a NaN
    // must fail both and a range contains neither it nor zero.
    if !(config.trenchless_speed_factor > 0.0 && config.trenchless_speed_factor <= 1.0) {
        return Err(PlateKinematicsError::InvalidTrenchlessSpeedFactor);
    }
    if !(config.slab_saturation_fraction > 0.0 && config.slab_saturation_fraction <= 1.0) {
        return Err(PlateKinematicsError::InvalidSlabSaturationFraction);
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

/// Blends the hashed axis toward the fitted one. Both ends are unit vectors,
/// so normalizing the linear blend lands on the arc between them.
fn blend_axis(random: Vec3, fitted: Vec3, coherence: f32) -> Vec3 {
    (random + (fitted - random) * coherence).normalized()
}

/// Area-weighted normal equations `matrix * ω = vector` for one plate.
#[derive(Clone, Copy, Default)]
struct NormalEquations {
    matrix: [[f64; 3]; 3],
    vector: [f64; 3],
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
    }

    /// Least-squares Euler vector, or `None` when the system is singular.
    /// `area` is the plate's own area, which the entries scale with and the
    /// determinant test needs as its reference.
    fn solve(&self, area: f64) -> Option<Vec3> {
        let rotation = solve_3x3(self.matrix, self.vector, area)?;
        Some(Vec3::new(
            rotation[0] as f32,
            rotation[1] as f32,
            rotation[2] as f32,
        ))
    }
}

/// Fits one rigid rotation per plate to `field`, in one pass over the cells.
/// `None` marks a plate whose normal equations are singular.
fn fit_plate_rotations(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    areas: &[f64],
    field: impl Fn(Vec3) -> Vec3,
) -> Vec<Option<Vec3>> {
    let mut equations = vec![NormalEquations::default(); partition.plate_count];
    for (cell, &plate) in partition.cell_plates.iter().enumerate() {
        let direction = mesh.cell_centers[cell].normalized();
        equations[plate].accumulate(
            f64::from(mesh.cell_areas[cell]),
            direction,
            field(direction),
        );
    }
    equations
        .iter()
        .zip(areas)
        .map(|(equations, &area)| equations.solve(area))
        .collect()
}

/// Solves a 3-by-3 system by Cramer's rule, rejecting a determinant too small
/// for the system's scale to carry a direction.
fn solve_3x3(matrix: [[f64; 3]; 3], vector: [f64; 3], scale: f64) -> Option<[f64; 3]> {
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
    use crate::test_support::plate_crust;
    use crate::{CrustClass, classify_crust, test_support};

    /// Normalizing the blended axis leaves the rotation vector's magnitude a
    /// few last bits off the clamped speed it was scaled by.
    const SPEED_TOLERANCE: f32 = 1.0e-6;

    fn reference_inputs() -> (SphereMesh, PlatePartition, CrustClassification) {
        let (mesh, partition) = test_support::reference_partition();
        let crust = classify_crust(&mesh, test_support::reference_crust_config()).unwrap();
        (mesh, partition, crust)
    }

    /// Splits a mesh into two plates of nearly equal area by handing each cell
    /// to whichever plate is smaller so far, so the two differ in crust and in
    /// nothing else that a speed reads.
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
        assert_eq!(first.base_speeds.len(), partition.plate_count);
        assert!(
            speeds(&first)
                .iter()
                .all(|&speed| speed <= config.maximum_angular_speed + SPEED_TOLERANCE)
        );
    }

    /// The stage scales its fitted axes by the crust factor and by nothing
    /// else. It cannot read a trench — the ocean has no ages until the
    /// crust-birth prior runs — so a plate carrying no continent turns at
    /// exactly its hashed base times the oceanic factor, and one carrying
    /// some continent is slower than that.
    #[test]
    fn every_fitted_speed_is_the_hashed_base_scaled_by_crust_alone() {
        let (mesh, partition, crust) = reference_inputs();
        let config = PlateKinematicsConfig::new(7);
        let kinematics = generate_plate_kinematics(&mesh, &partition, &crust, config).unwrap();
        let continental = crust.plate_continental_fraction(&mesh, &partition);

        let factors: Vec<f32> = speeds(&kinematics)
            .iter()
            .zip(&kinematics.base_speeds)
            .map(|(&speed, &base)| speed / base)
            .collect();
        let all_ocean: Vec<usize> = (0..partition.plate_count)
            .filter(|&plate| continental[plate] == 0.0)
            .collect();
        assert!(!all_ocean.is_empty(), "the world must hold an ocean plate");
        let mut clamped = 0;
        for plate in all_ocean {
            let speed = kinematics.angular_velocities[plate].length();
            // The ceiling is the one thing besides the crust factor that may
            // touch a fitted speed, and the oceanic factor of 1.5 carries the
            // faster half of the hashed range into it.
            if speed >= config.maximum_angular_speed - SPEED_TOLERANCE {
                clamped += 1;
                continue;
            }
            assert!(
                (factors[plate] - config.oceanic_speed_factor).abs() < SPEED_TOLERANCE,
                "plate {plate}: {}",
                factors[plate]
            );
        }
        assert!(clamped > 0, "the ceiling must bite somewhere");
        for (plate, &factor) in factors.iter().enumerate() {
            if continental[plate] > 0.0 {
                assert!(
                    factor < config.oceanic_speed_factor,
                    "plate {plate} carries continent and was not slowed: {factor}"
                );
            }
        }
        // A size factor would have moved plates apart by area; a slab factor
        // would have moved them apart by trench. Neither is applied, so every
        // all-ocean plate under the ceiling shares one factor.
        assert!(
            factors
                .iter()
                .all(|&factor| factor <= config.oceanic_speed_factor)
        );
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
        // an exact answer, so every plate recovers the same rotation whichever
        // cells it owns.
        let (mesh, partition) = two_equal_plates();
        let areas = partition.plate_areas(&mesh);
        let rotation = Vec3::new(0.3, -0.7, 0.5);
        let fitted: Vec<_> = fit_plate_rotations(&mesh, &partition, &areas, |direction| {
            rotation.cross(direction)
        })
        .into_iter()
        .map(|fit| fit.expect("both plates span enough cells"))
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
        let crust = plate_crust(
            &partition,
            &vec![CrustClass::Oceanic; partition.plate_count],
        );
        let config = PlateKinematicsConfig {
            coherence: 1.0,
            ..PlateKinematicsConfig::new(7)
        };
        let kinematics = generate_plate_kinematics(&mesh, &partition, &crust, config).unwrap();
        let flow = FlowField::new(&config);
        let flow_fits = fit_plate_rotations(&mesh, &partition, &areas, |direction| {
            flow.velocity_at(direction)
        });
        for (plate, velocity) in kinematics.angular_velocities.iter().enumerate() {
            let expected = flow_fits[plate].unwrap().normalized();
            assert!(
                (velocity.normalized() - expected).length() < 1.0e-6,
                "plate {plate} left its fitted axis"
            );
        }
    }

    #[test]
    fn oceanic_plates_outrun_continental_plates_of_similar_size() {
        let (mesh, partition) = two_equal_plates();
        let areas = partition.plate_areas(&mesh);
        assert!(
            (areas[0] - areas[1]).abs() / areas[0] < 0.01,
            "the halves must be similar in size: {areas:?}"
        );

        let crust = plate_crust(&partition, &[CrustClass::Oceanic, CrustClass::Continental]);
        let kinematics =
            generate_plate_kinematics(&mesh, &partition, &crust, PlateKinematicsConfig::new(7))
                .unwrap();

        let speeds = speeds(&kinematics);
        assert!(speeds[0] > speeds[1], "{speeds:?}");
    }

    #[test]
    fn single_cell_plates_fall_back_to_the_hashed_axis() {
        let (mesh, _, partition) = test_support::two_plate_boundary_partition();
        let crust = plate_crust(&partition, &[CrustClass::Oceanic; 2]);
        let config = PlateKinematicsConfig::new(7);
        let areas = partition.plate_areas(&mesh);

        let flow = FlowField::new(&config);
        let fits = fit_plate_rotations(&mesh, &partition, &areas, |direction| {
            flow.velocity_at(direction)
        });
        assert!(fits[1].is_none(), "a one-cell plate has no rotation to fit");

        // Only the axis falls back. The speed rule still reads the plate's
        // crust and its trenches, exactly as it does for a plate that fitted.
        let kinematics = generate_plate_kinematics(&mesh, &partition, &crust, config).unwrap();
        assert!(
            (kinematics.angular_velocities[1].normalized() - random_axis(1, config)).length()
                < 1.0e-6
        );
        assert_eq!(kinematics.base_speeds[1], random_speed(1, config));
    }

    #[test]
    fn local_velocity_is_tangent_and_scales_with_radius() {
        let kinematics = PlateKinematics {
            angular_velocities: vec![Vec3::new(0.0, 2.0, 0.0)],
            base_speeds: vec![2.0],
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
        let config = PlateKinematicsConfig {
            flow_frequency: 1.5,
            ..PlateKinematicsConfig::new(7)
        };
        let field = FlowField::new(&config);
        let velocity = field.velocity_at(direction);
        let other = FlowField::new(&PlateKinematicsConfig { seed: 8, ..config });

        assert!(velocity.dot(direction).abs() < 1.0e-6);
        assert!(velocity.length() > 0.0);
        assert_ne!(field, other);
        assert_ne!(velocity, other.velocity_at(direction));
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
            // A world in which no plate can move is not a slow world; it is one
            // that leaves every consumer of a plate speed without one.
            (
                PlateKinematicsConfig {
                    minimum_angular_speed: 0.0,
                    maximum_angular_speed: 0.0,
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
            // At zero a trenchless plate is frozen rather than slow, and past
            // one the factor no longer names a share of the base speed.
            (
                PlateKinematicsConfig {
                    trenchless_speed_factor: 0.0,
                    ..base
                },
                PlateKinematicsError::InvalidTrenchlessSpeedFactor,
            ),
            (
                PlateKinematicsConfig {
                    trenchless_speed_factor: 1.5,
                    ..base
                },
                PlateKinematicsError::InvalidTrenchlessSpeedFactor,
            ),
            (
                PlateKinematicsConfig {
                    slab_saturation_fraction: 0.0,
                    ..base
                },
                PlateKinematicsError::InvalidSlabSaturationFraction,
            ),
            (
                PlateKinematicsConfig {
                    slab_saturation_fraction: f32::NAN,
                    ..base
                },
                PlateKinematicsError::InvalidSlabSaturationFraction,
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
                    cell_classes: crust.cell_classes[1..].to_vec(),
                    ..crust.clone()
                },
                config,
            ),
            Err(PlateKinematicsError::Input(StageInputError::CrustClasses))
        );
    }
}
