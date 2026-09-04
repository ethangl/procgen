use bevy::prelude::*;
use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
use procgen_sphere_mesh::{SphereMesh, SphericalDelaunay};
use std::{
    error::Error,
    time::{Duration, Instant},
};

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
            .init_resource::<GenerationSettings>()
            .init_resource::<GenerationStatus>()
            .init_resource::<GeneratedWorld>()
            .add_systems(
                Update,
                regenerate_world.run_if(on_message::<RegenerateWorld>),
            );
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
    pub voronoi: SphereMesh,
    pub timings: GenerationTimings,
    pub config: FibonacciConfig,
}

impl GeneratedWorld {
    pub fn generate(config: FibonacciConfig) -> Result<Self, Box<dyn Error>> {
        let (points, sampling) = timed(|| fibonacci_sphere(config))?;
        let (delaunay, delaunay_time) = timed(|| SphericalDelaunay::build(points))?;
        let (voronoi, voronoi_time) = timed(|| SphereMesh::from_delaunay(&delaunay, 1.0))?;

        Ok(Self {
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

impl FromWorld for GeneratedWorld {
    fn from_world(world: &mut World) -> Self {
        let config = world.resource::<GenerationSettings>().fibonacci;
        Self::generate(config).expect("default world generation must succeed")
    }
}

fn timed<T, E>(operation: impl FnOnce() -> Result<T, E>) -> Result<(T, Duration), E> {
    let started = Instant::now();
    let result = operation()?;
    Ok((result, started.elapsed()))
}

fn regenerate_world(
    settings: Res<GenerationSettings>,
    mut world: ResMut<GeneratedWorld>,
    mut status: ResMut<GenerationStatus>,
) {
    match GeneratedWorld::generate(settings.fibonacci) {
        Ok(generated) => {
            *world = generated;
            status.last_error = None;
        }
        Err(error) => status.last_error = Some(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_consistent_viewer_counts() {
        let world = GeneratedWorld::generate(FibonacciConfig::new(128)).unwrap();

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
