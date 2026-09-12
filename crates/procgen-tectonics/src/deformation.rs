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
    BoundaryClassification, BoundaryDeformationConfig, CellCrust, FieldSummary, PlatePartition,
    boundary_profiles::PropagationProfile, boundary_sources::collect_boundary_sources,
    field::summarize_field, stage::StageInputError,
};
use procgen_sphere_mesh::SphereMesh;
use std::collections::VecDeque;

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

impl BoundaryDeformationConfig {
    /// Decays what a parcel of crust carries, adds one step's increment, and
    /// clamps. The whole of the accumulation rule is written once here rather
    /// than at each caller.
    ///
    /// Decay comes first, so a fresh source raises the whole of its increment
    /// and a boundary that holds its regime rises toward
    /// `increment * erosion_time / step_duration` before the clamp. Both signs
    /// decay: a trench whose subduction stops fills, and so does a rift whose
    /// spreading stops, which is what sediment does to both.
    ///
    /// The kept fraction is the linear `1 - step_duration / erosion_time`
    /// rather than `exp(-step_duration / erosion_time)`, because the linear
    /// form costs a multiply and a divide where the exponential costs libm on
    /// a path a kernel would have to mirror. The two differ to second order in
    /// `step_duration / erosion_time`, about one part in a thousand at the
    /// defaults, so halving the step does not decay to the same bits. An
    /// infinite `erosion_time` keeps the whole of `carried`, which is the
    /// world before the sink existed.
    pub(crate) fn accumulate(&self, carried: f32, increment: f32, step_duration: f32) -> f32 {
        let kept = carried * (1.0 - step_duration / self.erosion_time);
        (kept + increment).clamp(-self.maximum_magnitude, self.maximum_magnitude)
    }
}

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
        EvolutionFixture, NO_EROSION, NO_LIFECYCLE, NO_POLE_DRIFT, convergent_fixture,
        empty_boundaries, final_state_fixture, mesh as test_mesh, plate_cell_birth,
        two_plate_boundary_partition, two_plate_fixture,
    };
    use crate::{
        BoundaryClass, BoundaryEffect, ContinentalRiftProfile, CrustClass, PlateEvolutionConfig,
        step::EvolvingWorld,
    };

    use procgen_sphere_mesh::hop_length;

    /// One step's whole profile, over crust nothing has deformed yet.
    fn deform_once(
        mesh: &SphereMesh,
        partition: &PlatePartition,
        cell_birth: &[Option<f32>],
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
        cell_birth: &[Option<f32>],
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
        // `deform` derives its scale as the step over the full deformation
        // time, so the step this scale stands for is the one that decays what
        // the field already carries.
        let step_duration = scale * config.full_deformation_time;
        for (total, offset) in accumulated.iter_mut().zip(increment) {
            *total = config.accumulate(*total, offset, step_duration);
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
        // A clamp the quarter-profile steps below reach on the fifth of them,
        // so the loop sees both the increments and the bound. It is stated
        // against the profile rather than as a number, so that retuning the
        // profile cannot quietly stop this reaching the clamp at all.
        const SCALE: f32 = 0.25;
        const STEPS_TO_CLAMP: f32 = 5.0;
        let config = BoundaryDeformationConfig {
            maximum_magnitude: BoundaryDeformationConfig::default().convergent.offset
                * SCALE
                * STEPS_TO_CLAMP,
            // The increments and the clamp are what this pins, so the running
            // total is their sum and nothing takes anything away from it.
            erosion_time: NO_EROSION,
            ..Default::default()
        };
        let whole = deform_once(&mesh, &partition, &cell_birth, &boundaries, config);

        assert_eq!(whole[edge.cells[0]], config.convergent.offset);

        let increment = config.convergent.offset * SCALE;
        let mut accumulated = vec![0.0; mesh.cell_count()];
        for step in 1..=6 {
            let expected = (step as f32 * increment).min(config.maximum_magnitude);

            let source_cell_count = accumulate(
                &mesh,
                &partition,
                &cell_birth,
                &boundaries,
                &config,
                SCALE,
                &mut accumulated,
            );
            assert_eq!(source_cell_count, 2);
            assert!(
                (accumulated[edge.cells[0]] - expected).abs() < 1.0e-6,
                "step {step}: {} against {expected}",
                accumulated[edge.cells[0]]
            );
        }
        assert!(
            accumulated
                .iter()
                .all(|value| value.abs() <= config.maximum_magnitude)
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
                depth: hop_length(mesh.cell_count(), 1.0),
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

    /// A convergent two-plate run whose boundary never moves: the step is far
    /// too short for any material to leave its own cell, so every step
    /// classifies the same boundaries and raises the same profile.
    ///
    /// The sink is off, so what a cell carries is the sum of the increments
    /// and nothing else. The one test below that is about the sink turns it
    /// back on.
    fn static_boundary_fixture(
        step_count: usize,
        maximum_magnitude: f32,
    ) -> (EvolutionFixture, PlateEvolutionConfig) {
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental, CrustClass::Oceanic]);
        // Short enough that no particle leaves its own cell: two cells of
        // this coarse mesh have centres 0.0034 apart, and a cell that sampled
        // its neighbour would raise a different increment on the next step.
        // Two to the minus fourteen, with a full deformation time of ten times
        // it, so the tenth of a profile a step raises is exactly a tenth and
        // the assertions below can be exact.
        const STEP_DURATION: f32 = 0.000_061_035_156;
        let config = PlateEvolutionConfig {
            deformation: BoundaryDeformationConfig {
                // A tenth of the profile per step, and every boundary here
                // closes far faster than this, so every source saturates.
                full_deformation_time: 0.000_610_351_56,
                saturation_speed: 0.1,
                maximum_magnitude,
                erosion_time: NO_EROSION,
                ..BoundaryDeformationConfig::default()
            },

            // Every step must classify the same boundaries, which drifting
            // motion and a splitting plate are precisely what stop happening.
            pole_drift: NO_POLE_DRIFT,
            lifecycle: NO_LIFECYCLE,
            ..PlateEvolutionConfig::default()
        }
        .with_steps(step_count, STEP_DURATION);
        (fixture, config)
    }

    #[test]
    fn a_boundary_that_stays_put_adds_the_same_increment_every_step_until_the_clamp() {
        let limit = 0.12;
        let (fixture, config) = static_boundary_fixture(1, limit);
        let single = fixture.evolve(config).deformation.cell_deformation;
        assert_eq!(single[fixture.mesh.edges[0].cells[0]], 0.05);

        for step_count in 1..=6 {
            let run = fixture.evolve(config.with_steps(step_count, config.step_duration));
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

    /// Relief with no boundary under it decays by the same factor every step,
    /// and the assertion is float equality because the expectation is built
    /// from the same multiply the rule uses.
    #[test]
    fn relief_with_no_source_under_it_decays_geometrically() {
        const CARRIED: f32 = 0.4;
        let fixture = two_plate_fixture(-1.0, vec![CrustClass::Continental; 2]);
        let config = PlateEvolutionConfig {
            deformation: BoundaryDeformationConfig {
                // Three steps of the run below, so a step keeps two thirds of
                // what it carries and the decay is large enough to read.
                erosion_time: 0.3,
                ..BoundaryDeformationConfig::default()
            },
            pole_drift: NO_POLE_DRIFT,
            lifecycle: NO_LIFECYCLE,
            ..PlateEvolutionConfig::default()
        }
        .with_steps(0, 0.1);
        let kept = 1.0 - config.step_duration / config.deformation.erosion_time;

        let mut world = EvolvingWorld::new(&fixture.mesh, fixture.inputs(), config);
        for particle in &mut world.particles {
            particle.deformation = CARRIED;
        }
        // No boundary anywhere, so `deform` raises nothing and the decay is
        // the whole of what a step does to the field.
        let boundaries = empty_boundaries(&fixture.mesh);
        let mut expected = CARRIED;
        for step in 1..=6 {
            assert_eq!(world.deform(&boundaries), 0);
            expected *= kept;
            for particle in &world.particles {
                assert_eq!(
                    particle.deformation, expected,
                    "particle after {step} steps"
                );
            }
        }
        assert!(expected < 0.05, "six steps must decay most of the relief");
    }

    /// A negative field is the same rule: a trench whose subduction stopped
    /// fills at the rate a range wears down.
    #[test]
    fn a_negative_field_decays_by_the_same_factor() {
        let config = BoundaryDeformationConfig {
            erosion_time: 0.3,
            ..BoundaryDeformationConfig::default()
        };
        assert_eq!(
            config.accumulate(-0.4, 0.0, 0.1),
            -config.accumulate(0.4, 0.0, 0.1)
        );
    }

    /// With the sink under it, a boundary that keeps its regime stops short of
    /// the clamp: it rises toward the relief at which the decay of one step
    /// takes exactly what that step raises.
    #[test]
    fn an_active_boundary_settles_where_uplift_and_decay_balance() {
        // Out of reach, so the clamp cannot be what the run settles at.
        let (fixture, config) = static_boundary_fixture(1, 10.0);
        let config = PlateEvolutionConfig {
            deformation: BoundaryDeformationConfig {
                erosion_time: 2.0 * config.deformation.full_deformation_time,
                ..config.deformation
            },
            ..config
        };
        let cell = fixture.mesh.edges[0].cells[0];
        let increment = fixture.evolve(config).deformation.cell_deformation[cell];
        // The decay takes `carried * step / erosion_time` a step and the
        // boundary raises `increment`, so the two balance where the carried
        // relief is the increment times the erosion time over the step. That
        // is `offset * erosion_time / full_deformation_time`, two of the 0.5
        // collision offset here, and the clamp above is out of its reach.
        let settled = increment * config.deformation.erosion_time / config.step_duration;

        let mut previous = 0.0;
        let mut previous_rise = f32::INFINITY;
        for step_count in 1..=8 {
            let value = fixture
                .evolve(config.with_steps(step_count, config.step_duration))
                .deformation
                .cell_deformation[cell];
            let rise = value - previous;
            assert!(value > previous, "step {step_count} must raise the cell");
            assert!(rise < previous_rise, "step {step_count} must raise less");
            previous = value;
            previous_rise = rise;
        }

        let long = fixture
            .evolve(config.with_steps(120, config.step_duration))
            .deformation
            .cell_deformation[cell];
        assert!(
            long < settled && settled - long < increment,
            "120 steps reached {long}, not within one increment of {settled}"
        );
    }

    #[test]
    fn deformation_reaches_no_further_than_the_profiles_propagate() {
        let depth = 2;
        let (fixture, config) = static_boundary_fixture(4, 1.0);
        let cell_count = fixture.mesh.cell_count();
        let effect = BoundaryEffect {
            offset: 0.5,
            depth: hop_length(cell_count, depth as f32),
        };
        // Every profile reaches exactly `depth` hops: a linear effect decays to
        // zero one hop past its own, and a rift one hop past its decay depth.
        let run = fixture.evolve(PlateEvolutionConfig {
            deformation: BoundaryDeformationConfig {
                convergent: effect,
                transform: effect,
                collision: effect,
                trench: BoundaryEffect {
                    offset: -0.2,
                    depth: effect.depth,
                },
                island_arc: effect,
                rift: ContinentalRiftProfile {
                    decay_depth: hop_length(cell_count, depth as f32 + 1.0),
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
            deformation: BoundaryDeformationConfig {
                full_deformation_time: 1.0,
                ..BoundaryDeformationConfig::default()
            },
            ..config
        }
        .with_steps(steps, config.step_duration);
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

    /// Two sources whose profiles meet: the overlap takes the stronger by
    /// magnitude, which is the same rule a single cell resolves its own edges
    /// with.
    #[test]
    fn overlapping_profiles_resolve_by_magnitude() {
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
        sources[source_cells[0]] = Some(PropagationProfile::linear(-0.4, 1));
        sources[source_cells[1]] = Some(PropagationProfile::linear(0.4, 1));
        let propagated = propagate_boundary_effects(&mesh, &partition, &sources);
        assert_eq!(propagated[overlap], -0.2);
    }
}
