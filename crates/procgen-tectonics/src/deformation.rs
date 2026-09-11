//! Uplift and subsidence raised at the boundaries of one evolution step.
//!
//! A step's boundaries, the plates that currently own their cells, and the
//! crust those cells currently carry give every boundary cell one source
//! profile. Sources propagate a bounded number of hops inside their own plate
//! and resolve overlaps by maximum magnitude, exactly as the removed
//! post-evolution stage did over the final boundaries. The difference is that
//! the profile is now an increment: it is scaled by how much of
//! [`BoundaryDeformationConfig::full_deformation_time`] the step spent and
//! added into the field evolution carries with the crust. Belts therefore
//! widen where a boundary converged for many steps, a suture survives where a
//! boundary used to be, and a boundary that changed regime leaves both marks.

use crate::{
    BoundaryClass, BoundaryClassification, CellCrust, CrustClass, FieldSummary, PlatePartition,
    field::{DEFAULT_STEP_DURATION, summarize_field},
    stage::StageInputError,
};
use procgen_sphere_mesh::SphereMesh;
use std::{collections::VecDeque, fmt};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryEffect {
    /// Signed deformation at the boundary cell.
    pub offset: f32,
    /// Mesh hops the effect propagates within the current owning plate.
    pub depth: usize,
}

/// Graben and shoulders of a continental divergent boundary. A rift valley
/// sits below flanks that stand above the plateau behind them: the East
/// African floor lies about a kilometre under shoulders one to two kilometres
/// over their plateau. The flank may therefore be either sign, as long as it
/// is above the centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContinentalRiftProfile {
    /// Subsidence at a continental divergent boundary cell.
    pub center_offset: f32,
    /// Offset one mesh hop away from the boundary. Positive raises rift
    /// shoulders over the plateau; negative widens the depression.
    pub flank_offset: f32,
    /// Mesh hop at which the flank reaches zero within the owning plate.
    pub decay_depth: usize,
}

impl ContinentalRiftProfile {
    pub const MIN_DECAY_DEPTH: usize = 2;

    pub fn is_valid(&self) -> bool {
        self.center_offset.is_finite()
            && self.flank_offset.is_finite()
            && self.center_offset < 0.0
            && self.flank_offset > self.center_offset
            && self.decay_depth >= Self::MIN_DECAY_DEPTH
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryDeformationConfig {
    pub convergent: BoundaryEffect,
    /// Continental side of a divergent boundary.
    pub rift: ContinentalRiftProfile,
    /// Profile a transform boundary raises per unit of *residual convergence*,
    /// rather than per unit of shear: pure lateral slip builds no relief, and
    /// what a transform makes comes from the small normal component at a bend,
    /// positive where the bend is transpressive and negative where it pulls
    /// apart. The default depth is 1 because that relief is narrow — a
    /// restraining bend is a range one cell wide at this resolution, not the
    /// belt a convergent boundary spreads over six.
    pub transform: BoundaryEffect,
    /// Continental side of a mixed-crust convergent boundary.
    pub collision: BoundaryEffect,
    /// Oceanic side of a mixed-crust convergent boundary.
    pub trench: BoundaryEffect,
    /// Motion magnitude at which a boundary effect reaches its full offset.
    pub saturation_speed: f32,
    /// Model time over which a saturated boundary raises its full profile
    /// offset. Each step adds the profile scaled by the step's duration over
    /// this time, so the default of nine default steps
    /// (`9 * DEFAULT_STEP_DURATION`) is the time a boundary needs to hold one
    /// regime to reach the magnitudes the removed final-boundary stage
    /// produced. The viewer's run is longer than that, so a boundary that
    /// converges throughout raises more and [`Self::maximum_magnitude`]
    /// catches the few cells that saturate.
    pub full_deformation_time: f32,
    /// Magnitude the accumulated field is clamped to. The default is the
    /// largest offset the default profiles can raise — the collision centre at
    /// 0.5, against 0.4 for the convergent and transform centres and 0.2 for
    /// the trench and the rift centre — so it bites only where a boundary held
    /// one regime for longer than [`Self::full_deformation_time`]: 164 of the
    /// 65,536 cells at the viewer's defaults.
    pub maximum_magnitude: f32,
}

impl Default for BoundaryDeformationConfig {
    fn default() -> Self {
        Self {
            convergent: BoundaryEffect {
                offset: 0.4,
                depth: 6,
            },
            rift: ContinentalRiftProfile {
                center_offset: -0.2,
                flank_offset: 0.08,
                decay_depth: 3,
            },
            transform: BoundaryEffect {
                offset: 0.4,
                depth: 1,
            },
            collision: BoundaryEffect {
                offset: 0.5,
                depth: 5,
            },
            trench: BoundaryEffect {
                offset: -0.2,
                depth: 1,
            },
            saturation_speed: 2.0,
            full_deformation_time: 9.0 * DEFAULT_STEP_DURATION,
            maximum_magnitude: 0.5,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoundaryDeformationDiagnostics {
    pub summary: FieldSummary,
    /// Boundary cells that sourced a nonzero profile across all steps. A cell
    /// can contribute once per step.
    pub source_cell_count: usize,
    pub uplifted_cell_count: usize,
    pub subsided_cell_count: usize,
}

impl BoundaryDeformationDiagnostics {
    pub(crate) fn summarize(deformation: &[f32], source_cell_count: usize) -> Self {
        let mut uplifted_cell_count = 0;
        let mut subsided_cell_count = 0;
        let summary = summarize_field(deformation, |value| {
            uplifted_cell_count += usize::from(value > 0.0);
            subsided_cell_count += usize::from(value < 0.0);
        });
        Self {
            summary,
            source_cell_count,
            uplifted_cell_count,
            subsided_cell_count,
        }
    }

    pub const fn affected_cell_count(&self) -> usize {
        self.uplifted_cell_count + self.subsided_cell_count
    }
}

/// Signed per-cell deformation accumulated over every evolution step, carried
/// with the crust that it deformed.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryDeformation {
    pub cell_deformation: Vec<f32>,
    pub diagnostics: BoundaryDeformationDiagnostics,
}

impl BoundaryDeformation {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), StageInputError> {
        if self.cell_deformation.len() != mesh.cell_count() {
            return Err(StageInputError::Deformation);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryDeformationError {
    InvalidConfig,
    InvalidRiftProfile,
}

impl fmt::Display for BoundaryDeformationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str(
                "deformation offsets must be finite, and saturation speed, full deformation time, and maximum magnitude must be finite and positive",
            ),
            Self::InvalidRiftProfile => write!(
                formatter,
                "rift profile must satisfy center < 0 and center < flank, and decay depth >= {}",
                ContinentalRiftProfile::MIN_DECAY_DEPTH
            ),
        }
    }
}

impl std::error::Error for BoundaryDeformationError {}

/// Adds one step's boundary deformation into `accumulated` and returns how
/// many boundary cells carried a source.
///
/// Each boundary cell retains the strongest local source by absolute
/// magnitude. Sources then propagate for a bounded number of mesh hops without
/// crossing the plate that currently owns them, and overlaps within the step
/// also use maximum absolute magnitude; stable cell iteration makes
/// equal-magnitude ties deterministic. `scale` is the fraction of
/// [`BoundaryDeformationConfig::full_deformation_time`] the step spent, and
/// multiplying the propagated field by it is the same as scaling every source,
/// because the maxima that resolve overlaps are taken on magnitudes a positive
/// scale preserves.
///
/// `config` must have passed [`validate_config`]; evolution runs that once
/// rather than once per step.
impl BoundaryDeformationConfig {
    /// Adds one step's increment to what a parcel of crust already carries.
    /// The clamp is the whole of the accumulation rule, so it is written
    /// once here rather than at each caller.
    pub(crate) fn accumulate(&self, carried: f32, increment: f32) -> f32 {
        (carried + increment).clamp(-self.maximum_magnitude, self.maximum_magnitude)
    }
}

pub(crate) fn boundary_deformation_increment(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    crust: CellCrust<'_>,
    boundaries: &BoundaryClassification,
    config: &BoundaryDeformationConfig,
    scale: f32,
) -> (Vec<f32>, usize) {
    let sources = collect_boundary_sources(mesh, crust, boundaries, config);
    let mut increment = propagate_boundary_effects(mesh, partition, &sources);
    for offset in &mut increment {
        *offset *= scale;
    }
    (increment, sources.iter().flatten().count())
}

fn collect_boundary_sources(
    mesh: &SphereMesh,
    crust: CellCrust<'_>,
    boundaries: &BoundaryClassification,
    config: &BoundaryDeformationConfig,
) -> Vec<Option<PropagationProfile>> {
    let mut sources = vec![None; mesh.cell_count()];
    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        let class = boundaries.edge_classes[edge_index];
        let Some(scale) = source_scale(boundaries, edge_index, config) else {
            continue;
        };

        let classes = edge.cells.map(|cell| crust.class(cell));
        for side in 0..2 {
            let Some(source) = boundary_source(config, class, classes[side], classes[1 - side])
            else {
                continue;
            };
            let source_cell = edge.cells[side];
            let source = source.scaled(scale);
            retain_stronger_source(&mut sources[source_cell], source);
        }
    }
    sources
}

/// Signed fraction of its profile the edge's motion raises, or `None` where
/// the edge is not a boundary.
///
/// A convergent or divergent edge scales by its own strength, which is a
/// magnitude. A transform edge scales by its *signed residual convergence*
/// instead: lateral slip alone makes no relief, so what is left of the normal
/// component after classification decides both how much a transform raises and
/// which way. A negative scale turns the profile upside down, which is the
/// pull-apart basin a transtensional bend opens.
fn source_scale(
    boundaries: &BoundaryClassification,
    edge: usize,
    config: &BoundaryDeformationConfig,
) -> Option<f32> {
    let speed = match boundaries.edge_classes[edge] {
        BoundaryClass::Transform => boundaries.convergence(edge),
        _ => boundaries.strength(edge)?,
    };
    Some((speed / config.saturation_speed).clamp(-1.0, 1.0))
}

fn boundary_source(
    config: &BoundaryDeformationConfig,
    class: BoundaryClass,
    own: CrustClass,
    other: CrustClass,
) -> Option<PropagationProfile> {
    match (class, own, other) {
        (BoundaryClass::Convergent, CrustClass::Continental, CrustClass::Oceanic) => {
            Some(config.collision.into())
        }
        (BoundaryClass::Convergent, CrustClass::Oceanic, CrustClass::Continental) => {
            Some(config.trench.into())
        }
        (BoundaryClass::Convergent, _, _) => Some(config.convergent.into()),
        (BoundaryClass::Divergent, CrustClass::Oceanic, _) => None,
        (BoundaryClass::Divergent, CrustClass::Continental, _) => Some(config.rift.into()),
        (BoundaryClass::Transform, _, _) => Some(config.transform.into()),
        (BoundaryClass::Interior, _, _) => unreachable!("interior edges are skipped"),
    }
}

pub(crate) fn validate_config(
    config: BoundaryDeformationConfig,
) -> Result<(), BoundaryDeformationError> {
    if !config.rift.is_valid() {
        return Err(BoundaryDeformationError::InvalidRiftProfile);
    }
    let effects = [
        config.convergent,
        config.transform,
        config.collision,
        config.trench,
    ];
    let positives = [
        config.saturation_speed,
        config.full_deformation_time,
        config.maximum_magnitude,
    ];
    if effects.iter().any(|effect| !effect.offset.is_finite())
        || positives
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(BoundaryDeformationError::InvalidConfig);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PropagationProfile {
    center_offset: f32,
    flank_offset: f32,
    decay_depth: usize,
}

impl PropagationProfile {
    fn depth(self) -> usize {
        self.decay_depth.saturating_sub(1)
    }

    fn offset_at(self, depth: usize) -> f32 {
        if depth == 0 {
            return self.center_offset;
        }
        if self.decay_depth == 1 {
            return 0.0;
        }
        self.flank_offset * (1.0 - (depth - 1) as f32 / (self.decay_depth - 1) as f32)
    }

    fn scaled(self, scale: f32) -> Self {
        Self {
            center_offset: self.center_offset * scale,
            flank_offset: self.flank_offset * scale,
            ..self
        }
    }
}

impl From<ContinentalRiftProfile> for PropagationProfile {
    fn from(profile: ContinentalRiftProfile) -> Self {
        Self {
            center_offset: profile.center_offset,
            flank_offset: profile.flank_offset,
            decay_depth: profile.decay_depth,
        }
    }
}

impl From<BoundaryEffect> for PropagationProfile {
    fn from(effect: BoundaryEffect) -> Self {
        let decay_depth = effect.depth.saturating_add(1);
        Self {
            center_offset: effect.offset,
            flank_offset: effect.offset * effect.depth as f32 / decay_depth as f32,
            decay_depth,
        }
    }
}

fn retain_stronger_source(slot: &mut Option<PropagationProfile>, candidate: PropagationProfile) {
    let candidate_offset = candidate.offset_at(0);
    if candidate_offset != 0.0
        && slot.is_none_or(|current| candidate_offset.abs() > current.offset_at(0).abs())
    {
        *slot = Some(candidate);
    }
}

fn propagate_boundary_effects(
    mesh: &SphereMesh,
    partition: &PlatePartition,
    sources: &[Option<PropagationProfile>],
) -> Vec<f32> {
    let mut effects = vec![0.0_f32; mesh.cell_count()];
    let mut seen_at = vec![usize::MAX; mesh.cell_count()];
    let mut queue = VecDeque::new();

    // Cell order provides a stable tie break when equal-magnitude sources overlap.
    for (source_cell, source) in sources.iter().enumerate() {
        let Some(source) = source else { continue };
        let source_plate = partition.cell_plates[source_cell];
        queue.clear();
        queue.push_back((source_cell, 0_usize));
        seen_at[source_cell] = source_cell;

        while let Some((cell, depth)) = queue.pop_front() {
            let effect = source.offset_at(depth);
            if effect.abs() > effects[cell].abs() {
                effects[cell] = effect;
            }
            if depth == source.depth() {
                continue;
            }

            for corner in mesh.cell_corners(cell) {
                let neighbor = corner.neighbor;
                if seen_at[neighbor] == source_cell
                    || partition.cell_plates[neighbor] != source_plate
                {
                    continue;
                }
                seen_at[neighbor] = source_cell;
                queue.push_back((neighbor, depth + 1));
            }
        }
    }
    effects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        EvolutionFixture, NO_LIFECYCLE, NO_POLE_DRIFT, convergent_fixture, empty_boundaries,
        final_state_fixture, mesh as test_mesh, plate_cell_birth, two_plate_boundary_partition,
        two_plate_fixture,
    };
    use crate::{BoundaryClass, PlateEvolutionConfig};

    /// One step's whole profile, over crust nothing has deformed yet.
    fn deform_once(
        mesh: &SphereMesh,
        partition: &PlatePartition,
        cell_birth: &[Option<i32>],
        boundaries: &BoundaryClassification,
        config: BoundaryDeformationConfig,
    ) -> Vec<f32> {
        let mut field = vec![0.0; mesh.cell_count()];
        accumulate(
            mesh, partition, cell_birth, boundaries, &config, 1.0, &mut field,
        );
        field
    }

    /// What [`EvolvingWorld::deform`] does to one particle, driven over a
    /// per-cell field so that a test can run several steps of one static
    /// boundary without a whole world around it.
    fn accumulate(
        mesh: &SphereMesh,
        partition: &PlatePartition,
        cell_birth: &[Option<i32>],
        boundaries: &BoundaryClassification,
        config: &BoundaryDeformationConfig,
        scale: f32,
        accumulated: &mut [f32],
    ) -> usize {
        let (increment, source_cell_count) = boundary_deformation_increment(
            mesh,
            partition,
            CellCrust { cell_birth },
            boundaries,
            config,
            scale,
        );
        for (total, offset) in accumulated.iter_mut().zip(increment) {
            *total = config.accumulate(*total, offset);
        }
        source_cell_count
    }

    #[test]
    fn an_evolved_field_is_signed_and_summarizes_its_own_cells() {
        let (mesh, _, evolution) = final_state_fixture();
        let deformation = &evolution.deformation;

        deformation.validate(&mesh).unwrap();
        assert!(deformation.diagnostics.summary.minimum < 0.0);
        assert!(deformation.diagnostics.summary.maximum > 0.0);
        assert_eq!(
            deformation.diagnostics.affected_cell_count(),
            deformation.diagnostics.uplifted_cell_count
                + deformation.diagnostics.subsided_cell_count
        );
        assert_eq!(
            deformation.diagnostics.affected_cell_count(),
            deformation
                .cell_deformation
                .iter()
                .filter(|value| **value != 0.0)
                .count()
        );
    }

    #[test]
    fn a_step_adds_its_scaled_profile_and_clamps_the_running_total() {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let cell_birth = plate_cell_birth(&partition, &[CrustClass::Continental; 2]);
        let mut boundaries = empty_boundaries(&mesh);
        boundaries.edge_classes[edge_index] = BoundaryClass::Convergent;
        boundaries.edge_normal_speeds[edge_index] = [1.0, 1.0];
        let config = BoundaryDeformationConfig {
            maximum_magnitude: 0.5,
            ..Default::default()
        };
        let whole = deform_once(&mesh, &partition, &cell_birth, &boundaries, config);
        assert_eq!(whole[edge.cells[0]], config.convergent.offset);

        let mut accumulated = vec![0.0; mesh.cell_count()];
        for expected in [0.1, 0.2, 0.3, 0.4, 0.5, 0.5] {
            let source_cell_count = accumulate(
                &mesh,
                &partition,
                &cell_birth,
                &boundaries,
                &config,
                0.25,
                &mut accumulated,
            );
            assert_eq!(source_cell_count, 2);
            assert!((accumulated[edge.cells[0]] - expected).abs() < 1.0e-6);
        }
        assert!(
            accumulated
                .iter()
                .all(|value| value.abs() <= config.maximum_magnitude)
        );
    }

    #[test]
    fn mixed_convergence_uses_per_cell_crust_for_uplift_and_trench() {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let mut cell_birth =
            plate_cell_birth(&partition, &[CrustClass::Continental, CrustClass::Oceanic]);
        let mut boundaries = empty_boundaries(&mesh);
        boundaries.edge_classes[edge_index] = BoundaryClass::Convergent;
        boundaries.edge_normal_speeds[edge_index] = [1.0, 1.0];
        let config = BoundaryDeformationConfig {
            collision: BoundaryEffect {
                depth: 0,
                ..BoundaryDeformationConfig::default().collision
            },
            trench: BoundaryEffect {
                depth: 0,
                ..BoundaryDeformationConfig::default().trench
            },
            ..Default::default()
        };

        let original = deform_once(&mesh, &partition, &cell_birth, &boundaries, config);
        assert!(original[edge.cells[0]] > 0.0);
        assert!(original[edge.cells[1]] < 0.0);

        cell_birth.swap(edge.cells[0], edge.cells[1]);
        let changed = deform_once(&mesh, &partition, &cell_birth, &boundaries, config);
        assert!(changed[edge.cells[0]] < 0.0);
        assert!(changed[edge.cells[1]] > 0.0);
    }

    #[test]
    fn continental_rift_scales_with_its_normal_strength() {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let cell_birth = plate_cell_birth(&partition, &[CrustClass::Continental; 2]);
        let config = BoundaryDeformationConfig {
            rift: ContinentalRiftProfile {
                center_offset: -0.4,
                flank_offset: 0.1,
                decay_depth: 3,
            },
            saturation_speed: 4.0,
            ..Default::default()
        };
        let mut boundaries = empty_boundaries(&mesh);
        boundaries.edge_classes[edge_index] = BoundaryClass::Divergent;
        boundaries.edge_normal_speeds[edge_index] = [-0.5, -0.5];
        // Shear a transform would read, which a divergent edge must not.
        boundaries.edge_shear[edge_index] = 3.0;
        let divergent = deform_once(&mesh, &partition, &cell_birth, &boundaries, config);
        for cell in edge.cells {
            assert_eq!(divergent[cell], config.rift.center_offset / 4.0);
        }
        let expected_flank = config.rift.flank_offset * 0.5 / config.saturation_speed;
        assert!(
            divergent
                .iter()
                .enumerate()
                .any(|(cell, &value)| partition.cell_plates[cell] == 0 && value == expected_flank)
        );
    }

    #[test]
    fn transform_relief_follows_the_sign_of_its_residual_convergence() {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let cell_birth = plate_cell_birth(&partition, &[CrustClass::Continental; 2]);
        let config = BoundaryDeformationConfig {
            transform: BoundaryEffect {
                offset: 0.4,
                depth: 1,
            },
            saturation_speed: 4.0,
            ..Default::default()
        };
        let mut boundaries = empty_boundaries(&mesh);
        boundaries.edge_classes[edge_index] = BoundaryClass::Transform;
        boundaries.edge_shear[edge_index] = 3.0;

        // Lateral slip alone, however fast, makes no relief.
        let slipping = deform_once(&mesh, &partition, &cell_birth, &boundaries, config);
        assert!(slipping.iter().all(|&value| value == 0.0));

        // A restraining bend raises, a releasing bend subsides, and both take
        // the same fraction of the profile as their residual is of saturation.
        boundaries.edge_normal_speeds[edge_index] = [0.5, 0.5];
        let restraining = deform_once(&mesh, &partition, &cell_birth, &boundaries, config);
        boundaries.edge_normal_speeds[edge_index] = [-0.5, -0.5];
        let releasing = deform_once(&mesh, &partition, &cell_birth, &boundaries, config);
        for cell in edge.cells {
            assert_eq!(restraining[cell], config.transform.offset / 4.0);
        }
        // A pull-apart basin is the same shape upside down, flank included.
        assert!(
            releasing
                .iter()
                .zip(&restraining)
                .all(|(released, raised)| *released == -raised)
        );
    }

    #[test]
    fn a_boundary_that_only_slips_accumulates_no_deformation() {
        let (mesh, _, partition) = two_plate_boundary_partition();
        let cell_birth = plate_cell_birth(&partition, &[CrustClass::Continental; 2]);
        let mut boundaries = empty_boundaries(&mesh);
        for (edge_index, edge) in mesh.edges.iter().enumerate() {
            let [first, second] = edge.cells.map(|cell| partition.cell_plates[cell]);
            if first != second {
                boundaries.edge_classes[edge_index] = BoundaryClass::Transform;
                boundaries.edge_shear[edge_index] = 3.0;
            }
        }

        let mut accumulated = vec![0.0; mesh.cell_count()];
        for _ in 0..4 {
            let source_cell_count = accumulate(
                &mesh,
                &partition,
                &cell_birth,
                &boundaries,
                &BoundaryDeformationConfig::default(),
                0.25,
                &mut accumulated,
            );
            assert_eq!(source_cell_count, 0);
        }
        assert!(accumulated.iter().all(|&value| value == 0.0));
    }

    #[test]
    fn divergent_deformation_uses_per_cell_crust_and_leaves_oceanic_ridges_to_bathymetry() {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let mut cell_birth =
            plate_cell_birth(&partition, &[CrustClass::Continental, CrustClass::Oceanic]);
        let mut boundaries = empty_boundaries(&mesh);
        boundaries.edge_classes[edge_index] = BoundaryClass::Divergent;
        boundaries.edge_normal_speeds[edge_index] = [-1.0, -1.0];
        let config = BoundaryDeformationConfig {
            rift: ContinentalRiftProfile {
                center_offset: -0.4,
                flank_offset: -0.1,
                decay_depth: 3,
            },
            saturation_speed: 2.0,
            ..Default::default()
        };
        let crust = CellCrust {
            cell_birth: &cell_birth,
        };
        let deformation = deform_once(&mesh, &partition, &cell_birth, &boundaries, config);
        for cell in edge.cells {
            let expected = match crust.class(cell) {
                CrustClass::Continental => -0.4,
                CrustClass::Oceanic => 0.0,
            };
            assert_eq!(deformation[cell], expected);
        }

        let rift_flanks = deformation
            .iter()
            .enumerate()
            .filter(|(cell, value)| {
                partition.cell_plates[*cell] == 0 && **value == config.rift.flank_offset
            })
            .count();
        assert_eq!(rift_flanks, 4);
        assert!(
            deformation
                .iter()
                .enumerate()
                .all(|(cell, &value)| partition.cell_plates[cell] == 0 || value == 0.0)
        );

        cell_birth.swap(edge.cells[0], edge.cells[1]);
        let changed = deform_once(&mesh, &partition, &cell_birth, &boundaries, config);
        assert_eq!(changed[edge.cells[0]], 0.0);
        assert_eq!(changed[edge.cells[1]], -0.4);
    }

    #[test]
    fn continental_rift_has_a_deep_center_raised_shoulders_and_bounded_decay_to_zero() {
        let source = PropagationProfile::from(ContinentalRiftProfile {
            center_offset: -0.8,
            flank_offset: 0.2,
            decay_depth: 4,
        });

        assert_eq!(source.offset_at(0), -0.8);
        assert_eq!(source.offset_at(1), 0.2);
        assert!((source.offset_at(2) - 0.2 * (2.0 / 3.0)).abs() < f32::EPSILON);
        assert!((source.offset_at(3) - 0.2 * (1.0 / 3.0)).abs() < f32::EPSILON);
        assert_eq!(source.offset_at(4), 0.0);
        assert!(
            (1..=source.depth()).all(|depth| source.offset_at(depth) > 0.0),
            "a shoulder stands above the plateau until it decays to zero"
        );
    }

    #[test]
    fn linear_effect_conversion_preserves_its_profile_and_zero_depth_edge_case() {
        let effect = BoundaryEffect {
            offset: 0.6,
            depth: 3,
        };
        let profile = PropagationProfile::from(effect);
        for depth in 0..=effect.depth {
            let expected = effect.offset * (1.0 - depth as f32 / (effect.depth + 1) as f32);
            assert!((profile.offset_at(depth) - expected).abs() < f32::EPSILON);
        }

        let point = PropagationProfile::from(BoundaryEffect {
            offset: -0.2,
            depth: 0,
        });
        assert_eq!(point.depth(), 0);
        assert_eq!(point.offset_at(0), -0.2);
        assert_eq!(point.offset_at(1), 0.0);
    }

    #[test]
    fn propagation_is_bounded_to_the_current_plate() {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let cell_birth = plate_cell_birth(&partition, &[CrustClass::Continental; 2]);
        let mut boundaries = empty_boundaries(&mesh);
        boundaries.edge_classes[edge_index] = BoundaryClass::Convergent;
        boundaries.edge_normal_speeds[edge_index] = [1.0, 1.0];
        let config = BoundaryDeformationConfig {
            convergent: BoundaryEffect {
                offset: 0.4,
                depth: 1,
            },
            ..Default::default()
        };

        let deformation = deform_once(&mesh, &partition, &cell_birth, &boundaries, config);
        assert_eq!(deformation[edge.cells[1]], 0.4);
        assert_eq!(
            deformation
                .iter()
                .enumerate()
                .filter(|(cell, value)| partition.cell_plates[*cell] == 1 && **value != 0.0)
                .count(),
            1
        );
        assert!(
            deformation
                .iter()
                .enumerate()
                .any(|(cell, &value)| partition.cell_plates[cell] == 0 && value == 0.2)
        );
    }

    #[test]
    fn maximum_magnitude_wins_and_equal_ties_keep_the_first_source() {
        let mut source = None;
        let first = PropagationProfile::from(BoundaryEffect {
            offset: -0.4,
            depth: 1,
        });
        retain_stronger_source(&mut source, first);
        retain_stronger_source(
            &mut source,
            PropagationProfile::from(BoundaryEffect {
                offset: 0.4,
                depth: 7,
            }),
        );
        assert_eq!(source, Some(first));

        let stronger = PropagationProfile::from(BoundaryEffect {
            offset: 0.5,
            depth: 2,
        });
        retain_stronger_source(&mut source, stronger);
        assert_eq!(source, Some(stronger));

        let mesh = test_mesh(32);
        let mut source_cells = mesh.edges[0].cells;
        source_cells.sort();
        let overlap = mesh
            .cell_corners(source_cells[0])
            .iter()
            .map(|corner| corner.neighbor)
            .find(|&cell| {
                cell != source_cells[1]
                    && mesh
                        .cell_corners(source_cells[1])
                        .iter()
                        .any(|corner| corner.neighbor == cell)
            })
            .unwrap();
        let partition = PlatePartition {
            cell_plates: vec![0; mesh.cell_count()],
            plate_count: 1,
        };
        let mut sources = vec![None; mesh.cell_count()];
        sources[source_cells[0]] = Some(PropagationProfile::from(BoundaryEffect {
            offset: -0.4,
            depth: 1,
        }));
        sources[source_cells[1]] = Some(PropagationProfile::from(BoundaryEffect {
            offset: 0.4,
            depth: 1,
        }));
        let propagated = propagate_boundary_effects(&mesh, &partition, &sources);
        assert_eq!(propagated[overlap], -0.2);
    }

    #[test]
    fn rejects_invalid_configuration() {
        assert_eq!(
            validate_config(BoundaryDeformationConfig::default()),
            Ok(())
        );
        for config in [
            BoundaryDeformationConfig {
                saturation_speed: 0.0,
                ..Default::default()
            },
            BoundaryDeformationConfig {
                full_deformation_time: 0.0,
                ..Default::default()
            },
            BoundaryDeformationConfig {
                full_deformation_time: f32::NAN,
                ..Default::default()
            },
            BoundaryDeformationConfig {
                maximum_magnitude: -1.0,
                ..Default::default()
            },
            BoundaryDeformationConfig {
                maximum_magnitude: f32::INFINITY,
                ..Default::default()
            },
            BoundaryDeformationConfig {
                convergent: BoundaryEffect {
                    offset: f32::NAN,
                    ..BoundaryDeformationConfig::default().convergent
                },
                ..Default::default()
            },
        ] {
            assert_eq!(
                validate_config(config),
                Err(BoundaryDeformationError::InvalidConfig)
            );
        }
        // A negative flank is the pre-shoulder graben and stays legal.
        assert_eq!(
            validate_config(BoundaryDeformationConfig {
                rift: ContinentalRiftProfile {
                    flank_offset: -0.1,
                    ..BoundaryDeformationConfig::default().rift
                },
                ..Default::default()
            }),
            Ok(())
        );
        for rift in [
            ContinentalRiftProfile {
                center_offset: 0.1,
                ..BoundaryDeformationConfig::default().rift
            },
            ContinentalRiftProfile {
                center_offset: -0.1,
                flank_offset: -0.2,
                ..BoundaryDeformationConfig::default().rift
            },
            ContinentalRiftProfile {
                decay_depth: 1,
                ..BoundaryDeformationConfig::default().rift
            },
        ] {
            assert_eq!(
                validate_config(BoundaryDeformationConfig {
                    rift,
                    ..Default::default()
                }),
                Err(BoundaryDeformationError::InvalidRiftProfile)
            );
        }
    }
    /// A convergent two-plate run whose boundary never moves: the step is far
    /// too short for any material to leave its own cell, so every step
    /// classifies the same boundaries and raises the same profile.
    fn static_boundary_fixture(
        step_count: usize,
        maximum_magnitude: f32,
    ) -> (EvolutionFixture, PlateEvolutionConfig) {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental, CrustClass::Oceanic]);
        let config = PlateEvolutionConfig {
            step_count,
            // Short enough that no particle leaves its own cell: two cells of
            // this coarse mesh have centres 0.0034 apart, and a cell that
            // sampled its neighbour would raise a different increment on the
            // next step. Two to the minus fourteen, with a full deformation
            // time of ten times it, so the tenth of a profile a step raises
            // is exactly a tenth and the assertions below can be exact.
            step_duration: 0.000_061_035_156,
            deformation: BoundaryDeformationConfig {
                // A tenth of the profile per step, and every boundary here
                // closes far faster than this, so every source saturates.
                full_deformation_time: 0.000_610_351_56,
                saturation_speed: 0.1,
                maximum_magnitude,
                ..BoundaryDeformationConfig::default()
            },
            // Every step must classify the same boundaries, which drifting
            // motion and a splitting plate are precisely what stop happening.
            pole_drift: NO_POLE_DRIFT,
            lifecycle: NO_LIFECYCLE,
            ..PlateEvolutionConfig::default()
        };
        (fixture, config)
    }

    #[test]
    fn a_boundary_that_stays_put_adds_the_same_increment_every_step_until_the_clamp() {
        let limit = 0.12;
        let (fixture, config) = static_boundary_fixture(1, limit);
        let single = fixture.evolve(config).deformation.cell_deformation;
        assert_eq!(single[fixture.mesh.edges[0].cells[0]], 0.05);

        for step_count in 1..=6 {
            let run = fixture.evolve(PlateEvolutionConfig {
                step_count,
                ..config
            });
            assert_eq!(
                run.partition, fixture.partition,
                "nothing may move, or the increments would differ between steps"
            );
            assert_eq!(run.cell_birth, fixture.birth_prior.cell_birth);
            for (cell, &value) in run.deformation.cell_deformation.iter().enumerate() {
                let expected = (step_count as f32 * single[cell]).clamp(-limit, limit);
                // Summing an increment n times and multiplying it by n round
                // differently in the last bits.
                assert!(
                    (value - expected).abs() < 1.0e-6,
                    "cell {cell} after {step_count} steps: {value} against {expected}"
                );
            }
        }
    }

    #[test]
    fn deformation_reaches_no_further_than_the_profiles_propagate() {
        let depth = 2;
        let (fixture, config) = static_boundary_fixture(4, 1.0);
        let effect = BoundaryEffect { offset: 0.5, depth };
        // Every profile reaches exactly `depth` hops: a linear effect decays to
        // zero one hop past its own, and a rift one hop past its decay depth.
        let run = fixture.evolve(PlateEvolutionConfig {
            deformation: BoundaryDeformationConfig {
                convergent: effect,
                transform: effect,
                collision: effect,
                trench: BoundaryEffect {
                    offset: -0.2,
                    depth,
                },
                rift: ContinentalRiftProfile {
                    decay_depth: depth + 1,
                    ..config.deformation.rift
                },
                ..config.deformation
            },
            ..config
        });

        let mut reached: Vec<_> = (0..fixture.mesh.cell_count())
            .map(|cell| {
                fixture.mesh.cell_corners(cell).iter().any(|corner| {
                    fixture.partition.cell_plates[corner.neighbor]
                        != fixture.partition.cell_plates[cell]
                })
            })
            .collect();
        for _ in 0..depth {
            reached = (0..fixture.mesh.cell_count())
                .map(|cell| {
                    reached[cell]
                        || fixture
                            .mesh
                            .cell_corners(cell)
                            .iter()
                            .any(|corner| reached[corner.neighbor])
                })
                .collect();
        }

        assert!(
            reached.iter().any(|within| !within),
            "the test needs cells the profiles cannot reach"
        );
        for (cell, &value) in run.deformation.cell_deformation.iter().enumerate() {
            if !reached[cell] {
                assert_eq!(
                    value, 0.0,
                    "cell {cell} is further than {depth} hops from the boundary"
                );
            }
        }
    }

    #[test]
    fn a_boundary_that_has_moved_on_leaves_its_deformation_behind() {
        let (fixture, config, steps) = convergent_fixture();
        let config = PlateEvolutionConfig {
            step_count: steps,
            deformation: BoundaryDeformationConfig {
                full_deformation_time: 1.0,
                ..BoundaryDeformationConfig::default()
            },
            ..config
        };
        let run = fixture.evolve(config);

        // The overridden cell was the whole of its plate, so the boundary that
        // deformed these cells no longer exists anywhere.
        assert_eq!(run.partition.plate_count, 1);
        assert!(
            run.boundaries
                .edge_classes
                .iter()
                .all(|class| *class == BoundaryClass::Interior)
        );
        let (current, _) = boundary_deformation_increment(
            &fixture.mesh,
            &run.partition,
            run.cell_crust(),
            &run.boundaries,
            &config.deformation,
            1.0,
        );
        assert!(current.iter().all(|&value| value == 0.0));
        assert!(
            run.deformation
                .cell_deformation
                .iter()
                .any(|&value| value != 0.0),
            "the suture the vanished boundary left must survive it"
        );
    }
}
