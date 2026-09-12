//! Flood basalt provinces: the broad plateau a plume floods under a continent.
//!
//! A hotspot's trail is a point feature a few cells long. A plume under
//! continental crust does something else as well: it floods a flat-topped
//! plateau hundreds of kilometres across in a few million years, as the Deccan
//! Traps and the Columbia River basalts did. That plateau is one of the few
//! sources of relief a continental interior has away from any boundary.
//!
//! [`hotspots`] decides which plumes erupt one and aggregates the weights;
//! this module owns the shape.
//!
//! [`hotspots`]: crate::hotspots

use procgen_sphere_mesh::{SphereMesh, hops, multi_source_distances};
use procgen_tectonics::{CellCrust, CrustClass, PlatePartition};

/// One cell of a flood basalt province and the plateau weight it stands at.
pub(crate) struct ProvinceCell {
    pub(crate) cell: usize,
    pub(crate) weight: f32,
}

/// A province's configured radius and rim, resolved onto one mesh. The
/// configured pair are model lengths; this is where they become the hops the
/// flood counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProvinceProfile {
    /// Hop at which the rim would reach zero. That ring is already outside the
    /// province.
    pub(crate) radius: usize,
    /// Hops of the outer province over which the plateau slopes back down.
    /// Zero is a hard edge.
    pub(crate) rim: usize,
}

impl ProvinceProfile {
    pub(crate) fn new(cell_count: usize, radius: f32, rim: f32) -> Self {
        Self {
            radius: hops(cell_count, radius),
            rim: hops(cell_count, rim),
        }
    }

    /// Weight one across the flat top, then falling linearly toward zero at
    /// the radius: the shape a flood basalt pile has at this scale. The caller
    /// only asks about cells inside the radius, so the result is always
    /// positive.
    pub(crate) fn weight_at(self, hops: usize) -> f32 {
        if hops + self.rim <= self.radius {
            1.0
        } else {
            (self.radius - hops) as f32 / self.rim as f32
        }
    }
}

/// Floods a plateau outward from `source_cell` over the continental cells of
/// its own plate, so a province stops at a coast and at a plate boundary.
///
/// The radius is where the rim would reach zero, so it lies just outside: a
/// province is the cells strictly inside it, every one of which carries a
/// positive weight. Hop distances and the two-integer weights are exact, so
/// the cells and the profile are bit-identical wherever this runs.
pub(crate) fn trace_province(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    cell_crust: CellCrust<'_>,
    source_cell: usize,
    profile: ProvinceProfile,
) -> Vec<ProvinceCell> {
    let plate = plates.cell_plates[source_cell];
    let hops = multi_source_distances(mesh, &[source_cell], |_, neighbor| {
        plates.cell_plates[neighbor] == plate
            && cell_crust.class(neighbor) == CrustClass::Continental
    });
    hops.iter()
        .enumerate()
        .filter_map(|(cell, hops)| {
            let hops = (*hops)?;
            (hops < profile.radius).then(|| ProvinceCell {
                cell,
                weight: profile.weight_at(hops),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        HotspotFieldConfig, generate_hotspot_field,
        test_support::{
            HOTSPOT_CELL_COUNT, PlateFixture, hotspot_fixture, hotspot_province_config,
        },
    };
    use procgen_core::fingerprint;
    use procgen_sphere_mesh::{DEFAULT_CELL_COUNT, connected_components, hop_length};

    fn generate(fixture: &PlateFixture, config: HotspotFieldConfig) -> crate::HotspotField {
        generate_hotspot_field(
            &fixture.mesh,
            &fixture.plates,
            fixture.crust(),
            &fixture.kinematics,
            config,
        )
        .unwrap()
    }

    /// The hop profile one config resolves to on the reference mesh, which is
    /// what the assertions below are stated in.
    fn reference_profile(config: HotspotFieldConfig) -> ProvinceProfile {
        ProvinceProfile::new(
            HOTSPOT_CELL_COUNT,
            config.province_radius,
            config.province_rim,
        )
    }

    /// The radius and rim are model lengths now, and this is the assertion
    /// that they still mean the five and two cells each was written as.
    #[test]
    fn the_default_profile_is_the_cell_counts_it_replaced() {
        let config = HotspotFieldConfig::new(17);
        let profile = ProvinceProfile::new(
            DEFAULT_CELL_COUNT,
            config.province_radius,
            config.province_rim,
        );
        assert_eq!(profile.radius, 5);
        assert_eq!(profile.rim, 2);
    }

    #[test]
    fn provinces_are_bounded_continental_plate_local_and_connected() {
        let fixture = hotspot_fixture(HOTSPOT_CELL_COUNT);
        let config = hotspot_province_config();
        let profile = reference_profile(config);
        let field = generate(&fixture, config);
        let crust = fixture.crust();
        assert!(field.diagnostics.province_count > 0);

        for hotspot in &field.hotspots {
            if hotspot.province_cell_count == 0 {
                continue;
            }
            let province = trace_province(
                &fixture.mesh,
                &fixture.plates,
                crust,
                hotspot.source_cell,
                profile,
            );
            assert_eq!(province.len(), hotspot.province_cell_count);

            let in_province: Vec<_> = (0..fixture.mesh.cell_count())
                .map(|cell| province.iter().any(|point| point.cell == cell))
                .collect();
            // Hops through the province's own passable cells, which is what
            // the profile reads, and hops through the whole mesh, which a
            // detour around ocean or another plate can only lengthen.
            let hops = multi_source_distances(&fixture.mesh, &[hotspot.source_cell], |_, cell| {
                fixture.plates.cell_plates[cell] == hotspot.plate
                    && crust.class(cell) == CrustClass::Continental
            });
            let free_hops =
                multi_source_distances(&fixture.mesh, &[hotspot.source_cell], |_, _| true);
            for point in &province {
                let hops = hops[point.cell].expect("province cells are reachable");
                assert!(hops < profile.radius);
                assert!(free_hops[point.cell].expect("the mesh graph is connected") <= hops);
                assert_eq!(crust.class(point.cell), CrustClass::Continental);
                assert_eq!(
                    fixture.plates.cell_plates[point.cell], hotspot.plate,
                    "a province stops at a plate boundary"
                );
                // The flat top stands at one and the rim falls with distance,
                // but the weight reaches zero only at the radius, which is
                // outside the province, so no cell of one carries nothing.
                if hops + profile.rim <= profile.radius {
                    assert_eq!(point.weight, 1.0);
                } else {
                    assert!(point.weight > 0.0 && point.weight < 1.0);
                }
            }
            for pair in province.windows(2) {
                let closer = hops[pair[0].cell].unwrap();
                let further = hops[pair[1].cell].unwrap();
                if closer < further {
                    assert!(pair[0].weight >= pair[1].weight);
                }
            }

            let components =
                connected_components(&fixture.mesh, |cell| in_province[cell], |_, _| true);
            assert_eq!(components.len(), 1);
            assert_eq!(components[0].len(), province.len());
        }
    }

    #[test]
    fn an_oceanic_source_erupts_no_province() {
        let fixture = hotspot_fixture(HOTSPOT_CELL_COUNT);
        let field = generate(&fixture, hotspot_province_config());
        let crust = fixture.crust();
        let oceanic_sources = field
            .hotspots
            .iter()
            .filter(|hotspot| crust.class(hotspot.source_cell) == CrustClass::Oceanic)
            .count();
        assert!(oceanic_sources > 0);

        // The whole fraction erupts, so a missing province can only be an
        // ineligible source.
        for hotspot in &field.hotspots {
            assert_eq!(
                hotspot.province_cell_count > 0,
                crust.class(hotspot.source_cell) == CrustClass::Continental
            );
        }
    }

    #[test]
    fn overlapping_provinces_resolve_to_the_maximum_weight() {
        let fixture = hotspot_fixture(HOTSPOT_CELL_COUNT);
        // A wide radius on a coarse mesh puts several plateaus on top of one
        // another.
        let config = HotspotFieldConfig {
            hotspot_count: 64,
            province_radius: hop_length(HOTSPOT_CELL_COUNT, 6.0),
            province_rim: hop_length(HOTSPOT_CELL_COUNT, 3.0),
            ..hotspot_province_config()
        };
        let profile = reference_profile(config);
        let field = generate(&fixture, config);

        let mut expected = vec![0.0_f32; fixture.mesh.cell_count()];
        let mut contributions = vec![0_usize; fixture.mesh.cell_count()];
        for hotspot in &field.hotspots {
            if hotspot.province_cell_count == 0 {
                continue;
            }
            for point in trace_province(
                &fixture.mesh,
                &fixture.plates,
                fixture.crust(),
                hotspot.source_cell,
                profile,
            ) {
                expected[point.cell] = expected[point.cell].max(point.weight);
                contributions[point.cell] += 1;
            }
        }

        assert!(contributions.iter().any(|&count| count > 1));
        assert_eq!(field.cell_plateau, expected);
        assert_eq!(
            field.diagnostics.province_cell_count,
            contributions.iter().filter(|&&count| count > 0).count()
        );
    }

    #[test]
    fn reference_provinces_have_a_stable_cell_fingerprint() {
        let fixture = hotspot_fixture(HOTSPOT_CELL_COUNT);
        let field = generate(&fixture, hotspot_province_config());
        let covered: Vec<_> = field
            .cell_plateau
            .iter()
            .enumerate()
            .filter(|&(_, &weight)| weight > 0.0)
            .map(|(cell, _)| cell as u64)
            .collect();
        // Every cell a province covers carries weight, so the positive cells
        // of the field are exactly the cells the diagnostics counted.
        assert_eq!(covered.len(), field.diagnostics.province_cell_count);

        let values = field
            .hotspots
            .iter()
            .flat_map(|hotspot| {
                [
                    hotspot.source_cell as u64,
                    hotspot.province_cell_count as u64,
                ]
            })
            .chain(covered);

        assert_eq!(fingerprint(values), 17_700_700_707_790_126_781);
        assert_eq!(field.diagnostics.province_count, 3);
        assert_eq!(field.diagnostics.province_cell_count, 22);
    }
}
