use bevy::prelude::*;
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, SphericalDelaunay};
use std::time::{Duration, Instant};

#[derive(Resource)]
pub struct GenerationSettings {
    pub fibonacci: FibonacciConfig,
}

impl Default for GenerationSettings {
    fn default() -> Self {
        let mut fibonacci = FibonacciConfig::new(2_048);
        fibonacci.jitter = 0.5;
        fibonacci.seed = 7;
        Self { fibonacci }
    }
}

#[derive(Resource)]
pub struct LayerSettings {
    pub show_points: bool,
    pub show_delaunay: bool,
    pub show_voronoi: bool,
}

impl Default for LayerSettings {
    fn default() -> Self {
        Self {
            show_points: false,
            show_delaunay: false,
            show_voronoi: true,
        }
    }
}

#[derive(Resource, Default)]
pub struct GenerationStatus {
    pub last_error: Option<String>,
}

#[derive(Message, Default)]
pub struct RegenerateWorld;

pub struct WorldModelPlugin;

impl Plugin for WorldModelPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<RegenerateWorld>()
            .init_resource::<GenerationStatus>()
            .add_systems(Update, regenerate_world);
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
    pub config: FibonacciConfig,
}

impl GeneratedWorld {
    pub fn generate(config: FibonacciConfig) -> Result<Self, String> {
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
            config,
        })
    }
}

fn regenerate_world(
    mut requests: MessageReader<RegenerateWorld>,
    settings: Res<GenerationSettings>,
    mut world: ResMut<GeneratedWorld>,
    mut status: ResMut<GenerationStatus>,
) {
    if requests.read().count() == 0 {
        return;
    }

    match GeneratedWorld::generate(settings.fibonacci) {
        Ok(generated) => {
            *world = generated;
            status.last_error = None;
        }
        Err(error) => status.last_error = Some(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_consistent_viewer_counts() {
        let world = GeneratedWorld::generate(FibonacciConfig::new(128)).unwrap();

        assert_eq!(world.delaunay.triangle_count(), 252);
        assert_eq!(world.voronoi.cell_count(), 128);
        assert_eq!(world.voronoi.vertex_count(), 252);
        assert_eq!(world.voronoi.edge_count(), 378);
    }

    #[test]
    fn regeneration_message_replaces_the_active_world() {
        let mut app = App::new();
        let current = GeneratedWorld::generate(FibonacciConfig::new(32)).unwrap();
        let requested = FibonacciConfig::new(64);
        app.insert_resource(current)
            .insert_resource(GenerationSettings {
                fibonacci: requested,
            })
            .add_plugins(WorldModelPlugin);

        app.world_mut().write_message(RegenerateWorld);
        app.update();

        let world = app.world().resource::<GeneratedWorld>();
        assert_eq!(world.config, requested);
        assert_eq!(world.voronoi.cell_count(), requested.count);
    }
}
