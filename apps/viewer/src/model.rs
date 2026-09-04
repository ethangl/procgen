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
        Self {
            fibonacci: FibonacciConfig {
                jitter: 0.5,
                seed: 7,
                ..FibonacciConfig::new(2_048)
            },
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
        let (points, sampling) = timed(|| fibonacci_sphere(config));
        let points = points.map_err(|error| error.to_string())?;

        let (delaunay, delaunay_time) = timed(|| SphericalDelaunay::build(points));
        let delaunay = delaunay.map_err(|error| error.to_string())?;

        let (voronoi, voronoi_time) = timed(|| SphereMesh::from_delaunay(&delaunay, 1.0));
        let voronoi = voronoi.map_err(|error| error.to_string())?;

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

fn timed<T>(operation: impl FnOnce() -> T) -> (T, Duration) {
    let started = Instant::now();
    let result = operation();
    (result, started.elapsed())
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
