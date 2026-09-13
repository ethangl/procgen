//! Fixed pilot preparation shared by the inspector and headless validation.
use crate::{ContactError, DetailSource, PlanetError, Population, TerrainQueries};
use std::sync::Arc;
pub struct UsableTerrain {
    pub queries: TerrainQueries,
    pub population: Population,
}
#[derive(Debug)]
pub enum UsableError {
    Planet(PlanetError),
    Contact(ContactError),
}
impl From<PlanetError> for UsableError {
    fn from(e: PlanetError) -> Self {
        Self::Planet(e)
    }
}
impl From<ContactError> for UsableError {
    fn from(e: ContactError) -> Self {
        Self::Contact(e)
    }
}
impl std::fmt::Display for UsableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Planet(e) => e.fmt(f),
            Self::Contact(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for UsableError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::Planet(e) => e,
            Self::Contact(e) => e,
        })
    }
}
impl UsableTerrain {
    pub fn prepare(source: Arc<DetailSource>, seed: u64) -> Result<Self, ContactError> {
        let queries = TerrainQueries::new(Arc::clone(&source))?;
        let population = Population::prepare(seed, &queries);
        Ok(Self {
            queries,
            population,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CONTACT_SKIN, DetailLevel, WALK_RADIUS, Walker, build_region};
    use crate::{PILOT_PLANET, prepare_detail};
    use procgen_core::Vec3;
    use procgen_cubesphere::CubeFace;
    use std::sync::atomic::AtomicBool;
    #[test]
    fn contact_survives_render_replacement_and_placement_recreation() {
        let cancel = AtomicBool::new(false);
        let source = Arc::new(
            prepare_detail(&PILOT_PLANET.validate(42).unwrap(), &cancel)
                .unwrap()
                .unwrap(),
        );
        let terrain = UsableTerrain::prepare(Arc::clone(&source), 42).unwrap();
        let mut walker = crate::test_support::positions()
            .find_map(|p| Walker::land(&terrain.queries, p).ok())
            .expect("walkable sample");
        for _ in 0..120 {
            walker
                .advance(&terrain.queries, Vec3::ZERO, 1.0 / 120.0)
                .unwrap();
        }
        let resting = walker.center();
        assert!(walker.grounded());
        for level in DetailLevel::ALL {
            let region = build_region(&source, CubeFace::PositiveZ, level, &cancel).unwrap();
            assert!(!region.surface().triangles().is_empty());
            for _ in 0..120 {
                walker
                    .advance(&terrain.queries, Vec3::ZERO, 1.0 / 120.0)
                    .unwrap();
            }
            assert!(
                walker.center().distance_squared(resting) < 1e-8,
                "support moved with render replacement"
            );
        }
        let mut min_clearance = f32::MAX;
        for _ in 0..360 {
            walker
                .advance(&terrain.queries, Vec3::X, 1.0 / 120.0)
                .unwrap();
            let patch = terrain.queries.patch(walker.center()).unwrap();
            let clearance = patch
                .clearance(&terrain.queries, walker.center())
                .unwrap()
                .0;
            min_clearance = min_clearance.min(clearance);
        }
        assert!(
            min_clearance >= WALK_RADIUS - CONTACT_SKIN,
            "{min_clearance}"
        );
        assert!(
            walker.center().distance_squared(resting) > 0.01,
            "walker did not move"
        );
        let recreated = Population::prepare(42, &terrain.queries);
        assert_eq!(terrain.population.placements(), recreated.placements());
        assert!(!recreated.placements().is_empty());
        let fingerprint = recreated.placements().iter().fold(0_u32, |hash, p| {
            procgen_core::hash_u32(
                hash,
                p.id.face.index() as u32,
                p.id.x,
                p.id.y
                    + match p.id.kind {
                        crate::PlacementKind::Rock => 0,
                        crate::PlacementKind::Landmark => 256,
                    },
            )
        });
        // Separate contour sheets change accepted support triangles and candidate IDs.
        assert_eq!(fingerprint, 3_736_725_638);
        for p in recreated.placements() {
            let nearby: Vec<_> = recreated.nearby(p.position).copied().collect();
            assert!(nearby.contains(p));
            assert_eq!(recreated.nearby(p.position * 100.0).count(), 0);
            assert_eq!(
                nearby,
                recreated.nearby(p.position).copied().collect::<Vec<_>>()
            );
            if p.id.kind == crate::PlacementKind::Landmark {
                assert_eq!(recreated.landmark(p.position), Some(p));
            }
        }
        assert!(recreated.landmark(Vec3::X * 100.0).is_none());
        assert_ne!(
            recreated.placements(),
            Population::prepare(42 + (1_u64 << 32), &terrain.queries).placements()
        );
    }
}
