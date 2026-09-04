use bevy::prelude::*;
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, SphericalDelaunay};
use std::time::{Duration, Instant};

#[derive(Resource)]
pub struct ViewerSettings {
    pub count: usize,
    pub jitter: f32,
    pub seed: u64,
    pub show_points: bool,
    pub show_delaunay: bool,
    pub show_voronoi: bool,
    pub regenerate_requested: bool,
    pub last_error: Option<String>,
}

impl Default for ViewerSettings {
    fn default() -> Self {
        Self {
            count: 2_048,
            jitter: 0.5,
            seed: 7,
            show_points: false,
            show_delaunay: false,
            show_voronoi: true,
            regenerate_requested: false,
            last_error: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GenerationTimings {
    pub sampling: Duration,
    pub delaunay: Duration,
    pub voronoi: Duration,
}

impl GenerationTimings {
    pub fn total(self) -> Duration {
        self.sampling + self.delaunay + self.voronoi
    }
}

#[derive(Resource)]
pub struct GeneratedWorld {
    pub delaunay: SphericalDelaunay,
    pub voronoi: SphereMesh,
    pub timings: GenerationTimings,
    pub seed: u64,
    pub jitter: f32,
}

impl GeneratedWorld {
    pub fn generate(settings: &ViewerSettings) -> Result<Self, String> {
        let mut config = FibonacciConfig::new(settings.count);
        config.jitter = settings.jitter;
        config.seed = settings.seed;

        let started = Instant::now();
        let points = fibonacci_sphere(config).map_err(|error| error.to_string())?;
        let sampling = started.elapsed();

        let started = Instant::now();
        let delaunay = SphericalDelaunay::build(points).map_err(|error| error.to_string())?;
        let delaunay_time = started.elapsed();

        let started = Instant::now();
        let voronoi =
            SphereMesh::from_delaunay(&delaunay, 1.0).map_err(|error| error.to_string())?;
        let voronoi_time = started.elapsed();

        Ok(Self {
            delaunay,
            voronoi,
            timings: GenerationTimings {
                sampling,
                delaunay: delaunay_time,
                voronoi: voronoi_time,
            },
            seed: settings.seed,
            jitter: settings.jitter,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_consistent_viewer_counts() {
        let settings = ViewerSettings {
            count: 128,
            ..default()
        };
        let world = GeneratedWorld::generate(&settings).unwrap();

        assert_eq!(world.delaunay.triangle_count(), 252);
        assert_eq!(world.voronoi.cell_count(), 128);
        assert_eq!(world.voronoi.vertex_count(), 252);
        assert_eq!(world.voronoi.edge_count(), 378);
    }
}
