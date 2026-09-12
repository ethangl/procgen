//! Which profile each boundary cell raises, before anything propagates it.
//!
//! One cell can sit on several boundary edges, and the classes those edges
//! carry decide between them: the crust on each side, the polarity a trench
//! has, and how hard the two plates are closing. This module is that choice
//! and nothing else. What happens to the profile afterwards — how far into
//! its own plate it reaches, and what the carried field does with it — is
//! `deformation`'s.
//!
//! Splitting the two keeps the physical table in one place. Adding a boundary
//! class means adding a row here, not touching a propagation that knows
//! nothing about crust.

use crate::{
    BoundaryClass, BoundaryClassification, BoundaryDeformationConfig, CellCrust, CrustClass,
    boundary_profiles::{PropagationProfile, PropagationProfiles},
};
use procgen_sphere_mesh::SphereMesh;
use std::cmp::Ordering;

pub(crate) fn collect_boundary_sources(
    mesh: &SphereMesh,
    crust: CellCrust<'_>,
    boundaries: &BoundaryClassification,
    config: &BoundaryDeformationConfig,
) -> Vec<Option<PropagationProfile>> {
    // Every configured depth is a model length; this is where the mesh turns
    // them into the hop counts the propagation counts down.
    let profiles = config.profiles(mesh.cell_count());
    let mut sources = vec![None; mesh.cell_count()];
    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        let class = boundaries.edge_classes[edge_index];
        let Some(scale) = source_scale(boundaries, edge_index, config) else {
            continue;
        };

        for side in 0..2 {
            let source_cell = edge.cells[side];
            let Some(source) = boundary_source(
                &profiles,
                class,
                crust.class(source_cell),
                crust.order(source_cell, edge.cells[1 - side]),
            ) else {
                continue;
            };
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

/// The profile one side of one boundary edge raises, from its own crust class
/// and from [`crate::material_order`] over the two sides' crust.
///
/// A convergent edge is read by its polarity, which is the one rule
/// [`crate::material_order`] states: the side that covers the other is the
/// overriding plate and the side it covers is the slab going down. Continental
/// over oceanic is the Andean pair, the collision belt against a trench.
/// Oceanic over oceanic is the Marianas pair, a narrow island arc against a
/// trench. `Equal` has no polarity — two continents, or two floors of one age
/// — and both sides take the symmetric `convergent` belt.
///
/// Divergent and transform edges do not read the order: a rift is a property
/// of the crust on the side, and a transform's relief is its residual normal
/// component whatever lies across it.
fn boundary_source(
    profiles: &PropagationProfiles,
    class: BoundaryClass,
    own: CrustClass,
    order: Ordering,
) -> Option<PropagationProfile> {
    match (class, own, order) {
        (BoundaryClass::Convergent, _, Ordering::Equal) => Some(profiles.convergent),
        (BoundaryClass::Convergent, _, Ordering::Less) => Some(profiles.trench),
        (BoundaryClass::Convergent, CrustClass::Continental, Ordering::Greater) => {
            Some(profiles.collision)
        }
        (BoundaryClass::Convergent, CrustClass::Oceanic, Ordering::Greater) => {
            Some(profiles.island_arc)
        }
        (BoundaryClass::Divergent, CrustClass::Oceanic, _) => None,
        (BoundaryClass::Divergent, CrustClass::Continental, _) => Some(profiles.rift),
        (BoundaryClass::Transform, _, _) => Some(profiles.transform),
        (BoundaryClass::Interior, _, _) => unreachable!("interior edges are skipped"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        empty_boundaries, plate_cell_birth, plate_cell_birth_times, scaled_deformation,
        two_plate_boundary_partition,
    };
    use crate::{BoundaryEffect, ContinentalRiftProfile};
    use procgen_sphere_mesh::{hop_length, hops};

    /// One step's whole profile over crust nothing has deformed yet, which is
    /// what every source rule below is read through.
    fn deform_once(
        mesh: &SphereMesh,
        partition: &crate::PlatePartition,
        cell_birth: &[Option<f32>],
        boundaries: &BoundaryClassification,
        config: BoundaryDeformationConfig,
    ) -> Vec<f32> {
        let (increment, _) = crate::deformation::boundary_deformation_increment(
            mesh,
            partition,
            CellCrust { cell_birth },
            boundaries,
            &config,
            1.0,
        );
        increment
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
                depth: 0.0,
                ..BoundaryDeformationConfig::default().collision
            },
            trench: BoundaryEffect {
                depth: 0.0,
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

    /// Ocean-ocean convergence is the Marianas pair: the younger floor
    /// overrides, so it carries the arc and the older floor carries the
    /// trench. Plate 0 is every cell but one, so the arc has room to
    /// propagate its whole depth inside it.
    #[test]
    fn ocean_ocean_convergence_arcs_the_younger_side_and_trenches_the_older() {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let cell_birth = plate_cell_birth_times(&partition, &[Some(0.5), Some(0.1)]);
        let mut boundaries = empty_boundaries(&mesh);
        boundaries.edge_classes[edge_index] = BoundaryClass::Convergent;
        boundaries.edge_normal_speeds[edge_index] = [1.0, 1.0];
        let config = scaled_deformation(mesh.cell_count());

        let deformation = deform_once(&mesh, &partition, &cell_birth, &boundaries, config);
        assert_eq!(deformation[edge.cells[0]], config.island_arc.offset);
        assert_eq!(deformation[edge.cells[1]], config.trench.offset);

        let mut within = vec![false; mesh.cell_count()];
        within[edge.cells[0]] = true;
        for _ in 0..hops(mesh.cell_count(), config.island_arc.depth) {
            within = (0..mesh.cell_count())
                .map(|cell| {
                    within[cell]
                        || mesh
                            .cell_corners(cell)
                            .iter()
                            .any(|corner| within[corner.neighbor])
                })
                .collect();
        }
        assert!(
            deformation
                .iter()
                .enumerate()
                .all(|(cell, &value)| within[cell] || value == 0.0),
            "the arc reached further than its own depth"
        );
        assert!(
            deformation
                .iter()
                .enumerate()
                .any(|(cell, &value)| cell != edge.cells[0]
                    && partition.cell_plates[cell] == 0
                    && value > 0.0),
            "the arc is a belt behind the boundary, not one cell"
        );
    }

    /// Two floors of one age have no polarity, so neither side is the
    /// overriding plate and both take the symmetric belt.
    #[test]
    fn ocean_ocean_convergence_of_one_age_stays_symmetric() {
        let (mesh, edge_index, partition) = two_plate_boundary_partition();
        let edge = mesh.edges[edge_index];
        let cell_birth = plate_cell_birth_times(&partition, &[Some(0.5); 2]);
        let mut boundaries = empty_boundaries(&mesh);
        boundaries.edge_classes[edge_index] = BoundaryClass::Convergent;
        boundaries.edge_normal_speeds[edge_index] = [1.0, 1.0];
        let config = BoundaryDeformationConfig::default();

        let deformation = deform_once(&mesh, &partition, &cell_birth, &boundaries, config);
        for cell in edge.cells {
            assert_eq!(deformation[cell], config.convergent.offset);
        }
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
                decay_depth: hop_length(mesh.cell_count(), 3.0),
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
                depth: hop_length(mesh.cell_count(), 1.0),
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
                decay_depth: hop_length(mesh.cell_count(), 3.0),
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

    /// The strongest source by magnitude wins a cell, and an equal tie keeps
    /// the one already there, which is what makes a cell on several boundary
    /// edges answer the same way every run.
    #[test]
    fn the_strongest_source_wins_and_an_equal_tie_keeps_the_first() {
        let mut source = None;
        let first = PropagationProfile::linear(-0.4, 1);
        retain_stronger_source(&mut source, first);
        retain_stronger_source(&mut source, PropagationProfile::linear(0.4, 7));
        assert_eq!(source, Some(first));

        let stronger = PropagationProfile::linear(0.5, 2);
        retain_stronger_source(&mut source, stronger);
        assert_eq!(source, Some(stronger));
    }
}
