//! Upwind advection of a per-cell field along the wind, on the sphere mesh.
//!
//! Moisture is what this crate advects today, but nothing here knows that. A
//! route is built from the mesh, the wind, and how long a step lasts; what
//! rides along it is a mass per cell and could be anything conserved.
//!
//! The scheme is upwind and explicit. A cell exports one fraction of what it
//! holds per step — the distance the wind covers in a step over the distance
//! to its nearest downwind neighbour, capped — and splits that export among
//! the neighbours the wind actually points at, weighted by the square of the
//! alignment so a cell blowing between two neighbours favours the one it
//! faces. A cell whose wind points at no neighbour exports nothing.
//!
//! The cap is what keeps the scheme stable: without it a step longer than the
//! wind's crossing time would export more than a cell holds and drive the
//! field negative. It is a Courant limit stated as a fraction.
//!
//! Routes are built once per run, because the wind is fixed over a run. The
//! move itself is a Jacobi update: every cell reads the field the step began
//! with, and the new field is written only once every cell has been read.

use procgen_core::Vec3;
use procgen_sphere_mesh::SphereMesh;

/// What one cell does with its content each step: the share that leaves, and
/// where it goes. `destinations` weights sum to one, and an empty one means
/// the wind found no neighbour downwind of this cell.
#[derive(Clone, Debug)]
pub(crate) struct Route {
    pub(crate) export_fraction: f64,
    pub(crate) destinations: Vec<(usize, f64)>,
}

/// Builds one route per cell from the wind blowing over it.
///
/// `winds` are per-cell velocities in metres per second, `planet_radius_meters`
/// turns the mesh's angles into distances, `step_seconds` is how long one step
/// lasts, and `maximum_export_fraction` is the Courant cap on what may leave a
/// cell in that step.
pub(crate) fn build_routes(
    mesh: &SphereMesh,
    winds: &[Vec3],
    planet_radius_meters: f64,
    step_seconds: f64,
    maximum_export_fraction: f64,
) -> Vec<Route> {
    (0..mesh.cell_count())
        .map(|cell| {
            let normal = mesh.cell_centers[cell].normalized();
            let speed = f64::from(winds[cell].length());
            let mut destinations = Vec::new();
            let mut total_weight = 0.0_f64;
            let mut minimum_distance = f64::INFINITY;
            for corner in mesh.cell_corners(cell) {
                let neighbor_normal = mesh.cell_centers[corner.neighbor].normalized();
                let direction =
                    (neighbor_normal - normal * normal.dot(neighbor_normal)).normalized();
                let alignment = f64::from(winds[cell].dot(direction));
                if alignment <= 0.0 {
                    continue;
                }
                let weight = alignment * alignment;
                total_weight += weight;
                let angle = f64::from(normal.dot(neighbor_normal))
                    .clamp(-1.0, 1.0)
                    .acos();
                minimum_distance = minimum_distance.min(angle * planet_radius_meters);
                destinations.push((corner.neighbor, weight));
            }
            if total_weight == 0.0 {
                return Route {
                    export_fraction: 0.0,
                    destinations: Vec::new(),
                };
            }
            for (_, weight) in &mut destinations {
                *weight /= total_weight;
            }
            Route {
                export_fraction: (speed * step_seconds / minimum_distance)
                    .min(maximum_export_fraction),
                destinations,
            }
        })
        .collect()
}

/// Moves one step of `field` along `routes`, leaving the result in `field`.
///
/// `scratch` is the caller's buffer of the same length, kept across steps so a
/// run allocates nothing per step; what it holds on entry is overwritten and
/// what it holds on return is the field the step began with.
pub(crate) fn advect(routes: &[Route], field: &mut Vec<f64>, scratch: &mut Vec<f64>) {
    scratch.clone_from_slice(field);
    for (cell, route) in routes.iter().enumerate() {
        let exported = field[cell] * route.export_fraction;
        scratch[cell] -= exported;
        for &(neighbor, share) in &route.destinations {
            scratch[neighbor] += exported * share;
        }
    }
    std::mem::swap(field, scratch);
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::build_sphere_mesh;

    const RADIUS_METERS: f64 = 6.371e6;
    const STEP_SECONDS: f64 = 3_600.0;
    const MAXIMUM_EXPORT: f64 = 0.25;

    fn mesh(count: usize) -> SphereMesh {
        build_sphere_mesh(
            fibonacci_sphere(FibonacciConfig {
                count,
                jitter: 0.0,
                seed: 7,
            })
            .unwrap(),
            1.0,
        )
        .unwrap()
    }

    /// Eastward at every cell, which is the flow the moisture stage's own
    /// tests use, so every cell has some neighbour downwind of it.
    fn eastward(mesh: &SphereMesh, speed: f32) -> Vec<Vec3> {
        mesh.cell_centers
            .iter()
            .map(|&center| {
                let east = Vec3::new(-center.y, center.x, 0.0);
                if east.length() > 1.0e-6 {
                    east.normalized() * speed
                } else {
                    Vec3::ZERO
                }
            })
            .collect()
    }

    fn routes(winds: &[Vec3], mesh: &SphereMesh) -> Vec<Route> {
        build_routes(mesh, winds, RADIUS_METERS, STEP_SECONDS, MAXIMUM_EXPORT)
    }

    #[test]
    fn a_route_sends_its_export_only_downwind_and_splits_it_whole() {
        let mesh = mesh(256);
        let winds = eastward(&mesh, 20.0);
        let routes = routes(&winds, &mesh);

        assert_eq!(routes.len(), mesh.cell_count());
        for (cell, route) in routes.iter().enumerate() {
            assert!(
                (0.0..=MAXIMUM_EXPORT).contains(&route.export_fraction),
                "cell {cell} exports {}",
                route.export_fraction
            );
            if route.destinations.is_empty() {
                assert_eq!(route.export_fraction, 0.0);
                continue;
            }
            let total: f64 = route.destinations.iter().map(|&(_, share)| share).sum();
            assert!((total - 1.0).abs() < 1.0e-12, "cell {cell} splits {total}");
            // Every destination is a neighbour the wind points at, so the flow
            // never carries anything upwind.
            for &(neighbor, share) in &route.destinations {
                assert!(share > 0.0);
                let normal = mesh.cell_centers[cell].normalized();
                let neighbor_normal = mesh.cell_centers[neighbor].normalized();
                let direction =
                    (neighbor_normal - normal * normal.dot(neighbor_normal)).normalized();
                assert!(winds[cell].dot(direction) > 0.0, "cell {cell} sent upwind");
            }
        }
    }

    #[test]
    fn advection_conserves_the_field_it_moves() {
        let mesh = mesh(256);
        let routes = routes(&eastward(&mesh, 20.0), &mesh);
        let mut field: Vec<f64> = (0..mesh.cell_count()).map(|cell| cell as f64).collect();
        let mut scratch = vec![0.0; mesh.cell_count()];
        let before: f64 = field.iter().sum();

        for _ in 0..32 {
            advect(&routes, &mut field, &mut scratch);
        }

        // The cap keeps every export inside what a cell holds, so a field that
        // starts non-negative stays so and nothing is created or destroyed.
        assert!(field.iter().all(|&value| value >= 0.0));
        let after: f64 = field.iter().sum();
        assert!(
            (after - before).abs() / before < 1.0e-12,
            "{before} to {after}"
        );
    }

    #[test]
    fn still_air_moves_nothing() {
        let mesh = mesh(128);
        let routes = routes(&vec![Vec3::ZERO; mesh.cell_count()], &mesh);
        assert!(
            routes
                .iter()
                .all(|route| route.export_fraction == 0.0 && route.destinations.is_empty())
        );

        let mut field: Vec<f64> = (0..mesh.cell_count()).map(|cell| cell as f64).collect();
        let mut scratch = vec![0.0; mesh.cell_count()];
        let before = field.clone();

        advect(&routes, &mut field, &mut scratch);

        assert_eq!(field, before);
    }

    /// A faster wind exports more, up to the cap and no further.
    #[test]
    fn the_export_fraction_follows_the_wind_and_stops_at_the_cap() {
        let mesh = mesh(256);
        let slow = routes(&eastward(&mesh, 1.0), &mesh);
        let fast = routes(&eastward(&mesh, 20.0), &mesh);
        let gale = routes(&eastward(&mesh, 10_000.0), &mesh);

        let moving = |routes: &[Route]| {
            routes
                .iter()
                .filter(|route| !route.destinations.is_empty())
                .map(|route| route.export_fraction)
                .collect::<Vec<_>>()
        };
        for (slow, fast) in moving(&slow).iter().zip(moving(&fast)) {
            assert!(*slow < fast, "{slow} against {fast}");
        }
        assert!(
            moving(&gale)
                .iter()
                .all(|&fraction| fraction == MAXIMUM_EXPORT)
        );
    }
}
