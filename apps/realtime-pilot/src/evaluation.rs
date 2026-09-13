//! Bounded, deterministic CPU checks; file formats and rendering stay in consumers.
use crate::{
    CONTACT_SKIN, ContactError, DetailLevel, MeshTopology, PlanetError, Scenario, UsableTerrain,
    WALK_RADIUS, build_region, prepare_detail, route_walker, walking_input,
};
use procgen_core::{Vec3, quantized_fingerprint};
use procgen_cubesphere::CubeFace;
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
pub const PREPARATION_LIMIT_MS: f64 = 1000.0;
pub const WALK_STEP_LIMIT_MS: f64 = 4.0;
#[derive(Debug)]
pub enum EvaluationError {
    Planet(PlanetError),
    Contact(ContactError),
    Invariant(&'static str),
}
impl From<PlanetError> for EvaluationError {
    fn from(e: PlanetError) -> Self {
        Self::Planet(e)
    }
}
impl From<ContactError> for EvaluationError {
    fn from(e: ContactError) -> Self {
        Self::Contact(e)
    }
}
impl std::fmt::Display for EvaluationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Planet(e) => e.fmt(f),
            Self::Contact(e) => e.fmt(f),
            Self::Invariant(e) => f.write_str(e),
        }
    }
}
impl std::error::Error for EvaluationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Planet(e) => Some(e),
            Self::Contact(e) => Some(e),
            Self::Invariant(_) => None,
        }
    }
}
#[derive(Debug)]
pub struct EvaluationFailure {
    pub stage: &'static str,
    pub error: EvaluationError,
    pub location: Option<Vec3>,
    pub tick: Option<u32>,
}
impl std::fmt::Display for EvaluationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.stage, self.error)
    }
}
impl std::error::Error for EvaluationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}
fn failure(stage: &'static str, error: impl Into<EvaluationError>) -> EvaluationFailure {
    EvaluationFailure {
        stage,
        error: error.into(),
        location: None,
        tick: None,
    }
}
pub struct Evaluation {
    pub preparation_ms: f64,
    pub mesh_checks_ms: f64,
    pub walk_ms: f64,
    pub walk_step_max_ms: f64,
    pub triangles: usize,
    pub topology: MeshTopology,
    pub radius_range: [f32; 2],
    pub broad_height: crate::HeightDistribution,
    pub mesh_fingerprint: u64,
    pub route_fingerprint: u64,
    pub placements: usize,
    pub min_clearance: f32,
    pub walk_displacement: f32,
    pub rest_drift: f32,
    pub grounded_rest_steps: usize,
    pub source_bytes: usize,
    pub collision_index_bytes: usize,
}
pub fn evaluate(scenario: Scenario) -> Result<Evaluation, EvaluationFailure> {
    let start = Instant::now();
    let field = scenario.validate().map_err(|e| failure("parameters", e))?;
    let cancel = AtomicBool::new(false);
    let source = Arc::new(
        prepare_detail(&field, &cancel)
            .map_err(|e| failure("source", e))?
            .expect("not cancelled"),
    );
    let usable = UsableTerrain::prepare(Arc::clone(&source), scenario.seed)
        .map_err(|e| failure("collision", e))?;
    let preparation_ms = start.elapsed().as_secs_f64() * 1000.0;
    let checks = Instant::now();
    let topology = source.mesh.topology();
    check_topology(topology).map_err(|e| failure("fine topology", e))?;
    let mut radius_range = [f32::INFINITY, f32::NEG_INFINITY];
    for p in source.mesh.positions() {
        let r = p.length();
        radius_range[0] = radius_range[0].min(r);
        radius_range[1] = radius_range[1].max(r);
    }
    let mesh_fingerprint =
        quantized_fingerprint(source.mesh.positions().iter().flat_map(|p| [p.x, p.y, p.z]));
    // Only six region products live at once; rotate the level assignment so
    // every face is tested at every level without retaining eighteen products.
    for offset in 0..3 {
        let products: Vec<_> = CubeFace::ALL
            .iter()
            .enumerate()
            .map(|(i, &face)| {
                build_region(&source, face, DetailLevel::ALL[(i + offset) % 3], &cancel)
                    .expect("not cancelled")
            })
            .collect();
        let mixed = crate::detail::join_regions(products.iter());
        check_topology(mixed.topology()).map_err(|e| failure("mixed topology", e))?;
    }
    let recreated = crate::Population::prepare(scenario.seed, &usable.queries);
    if recreated.placements() != usable.population.placements() {
        return Err(failure(
            "population",
            EvaluationError::Invariant("placement recreation changed"),
        ));
    }
    let mesh_checks_ms = checks.elapsed().as_secs_f64() * 1000.0;
    let broad_height = crate::height_distribution::sample(&field);
    // Prove stationary support independently of where the arbitrary route ends.
    // An idle phase can end on an unwalkable slope and continue sliding.
    let mut probe = route_walker(&usable).map_err(|e| failure("landing", e))?;
    for _ in 0..120 {
        probe
            .advance(&usable.queries, Vec3::ZERO, 1.0 / 120.0)
            .map_err(|e| failure("rest", e))?;
    }
    let anchor = probe.center();
    let mut rest_drift = 0.0_f32;
    let mut grounded_rest_steps = 0;
    for _ in 0..120 {
        probe
            .advance(&usable.queries, Vec3::ZERO, 1.0 / 120.0)
            .map_err(|e| failure("rest", e))?;
        rest_drift = rest_drift.max(probe.center().distance_squared(anchor).sqrt());
        if !probe.grounded() || rest_drift > 0.001 {
            return Err(EvaluationFailure {
                stage: "rest",
                error: EvaluationError::Invariant("unstable walkable support at landing site"),
                location: Some(probe.eye()),
                tick: None,
            });
        }
        grounded_rest_steps += 1;
    }
    let mut walker = route_walker(&usable).map_err(|e| failure("landing", e))?;
    let origin = walker.center();
    let mut outward = origin;
    let mut rest = None;
    let mut path = Vec::with_capacity(4320 * 3);
    let mut min_clearance = f32::INFINITY;
    let mut step_max = 0.0_f64;
    let walk_start = Instant::now();
    for tick in 0..4320_u32 {
        let now = tick as f32 / 120.0;
        let was_grounded = walker.grounded();
        let previous = walker.center();
        let before = Instant::now();
        if let Err(error) = walker.advance(&usable.queries, walking_input(now), 1.0 / 120.0) {
            return Err(EvaluationFailure {
                stage: "walk",
                error: error.into(),
                location: Some(walker.eye()),
                tick: Some(tick),
            });
        }
        step_max = step_max.max(before.elapsed().as_secs_f64() * 1000.0);
        let p = walker.center();
        path.extend([p.x, p.y, p.z]);
        let clearance = walker.clearance(&usable.queries).ok_or(EvaluationFailure {
            stage: "walk",
            error: EvaluationError::Invariant("missing support triangles"),
            location: Some(walker.eye()),
            tick: Some(tick),
        })?;
        if clearance < WALK_RADIUS - CONTACT_SKIN {
            return Err(EvaluationFailure {
                stage: "contact",
                error: EvaluationError::Invariant("sphere penetrated fine terrain"),
                location: Some(walker.eye()),
                tick: Some(tick),
            });
        }
        min_clearance = min_clearance.min(clearance);
        if tick == 1439 {
            outward = p;
        }
        if (1440..2160).contains(&tick) || tick >= 3600 {
            // Stopping input in midair must not freeze gravity. Measure static
            // friction only during continuous walkable support, including its
            // first step. Landing-site support is checked separately above.
            if was_grounded && walker.grounded() {
                let anchor = *rest.get_or_insert(previous);
                rest_drift = rest_drift.max(p.distance_squared(anchor).sqrt());
                grounded_rest_steps += 1;
            } else {
                rest = None;
            }
            if rest_drift > 0.001 {
                return Err(EvaluationFailure {
                    stage: "rest",
                    error: EvaluationError::Invariant("support moved while resting"),
                    location: Some(walker.eye()),
                    tick: Some(tick),
                });
            }
        } else {
            rest = None;
        }
    }
    Ok(Evaluation {
        preparation_ms,
        mesh_checks_ms,
        walk_ms: walk_start.elapsed().as_secs_f64() * 1000.0,
        walk_step_max_ms: step_max,
        triangles: source.triangle_count(),
        topology,
        radius_range,
        broad_height,
        mesh_fingerprint,
        route_fingerprint: quantized_fingerprint(path),
        placements: usable.population.placements().len(),
        min_clearance,
        walk_displacement: outward.distance_squared(origin).sqrt(),
        rest_drift,
        grounded_rest_steps,
        source_bytes: source.allocated_bytes(),
        collision_index_bytes: usable.queries.allocated_bytes(),
    })
}
fn check_topology(t: MeshTopology) -> Result<(), EvaluationError> {
    if !t.is_closed_manifold() {
        return Err(EvaluationError::Invariant(
            "open, nonmanifold, unbalanced, or degenerate surface",
        ));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PLANET_PRESETS, RouteKind};
    #[test]
    fn resolved_presets_repeat_and_invalid_cases_have_stage_context() {
        let scenario = Scenario {
            // This route is airborne when input stops at 12 seconds, then lands.
            // Falling is valid; drift while grounded must still be rejected.
            seed: 0,
            planet: PLANET_PRESETS[0].planet,
            route: RouteKind::Walk,
        };
        let a = evaluate(scenario).unwrap();
        let b = evaluate(scenario).unwrap();
        assert_eq!(a.mesh_fingerprint, b.mesh_fingerprint);
        assert_eq!(a.route_fingerprint, b.route_fingerprint);
        assert!(a.grounded_rest_steps > 0);
        let bad = Scenario {
            planet: crate::PlanetConfig {
                radius: 8.0,
                ..scenario.planet
            },
            ..scenario
        };
        assert_eq!(evaluate(bad).err().unwrap().stage, "parameters");
    }
}
