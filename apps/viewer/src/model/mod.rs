mod climate;
mod geology;
mod tectonics;

pub use climate::{ClimateSettings, ClimateWorld};
pub use geology::{GeologySettings, GeologyWorld};
pub use tectonics::{TectonicsSettings, TectonicsWorld};

use crate::cache::WorldCache;
use bevy::prelude::*;
use std::{
    error::Error,
    time::{Duration, Instant},
};

pub const WORLD_RADIUS: f32 = 1.0;

/// The generation phases in dependency order, which is also their order: each
/// phase consumes the results of the phases before it and can be regenerated
/// on its own.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Phase {
    Tectonics,
    Geology,
    Climate,
}

impl Phase {
    pub const ALL: &[Self] = &[Self::Tectonics, Self::Geology, Self::Climate];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Tectonics => "Tectonics",
            Self::Geology => "Geology",
            Self::Climate => "Climate",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Resource)]
pub struct GenerationSettings {
    pub tectonics: TectonicsSettings,
    pub geology: GeologySettings,
    pub climate: ClimateSettings,
}

/// The phase results currently in memory. A phase is present only while the
/// results it was built from are still current: regenerating a phase drops
/// every downstream result.
#[derive(Default, Resource)]
pub struct GeneratedWorld {
    tectonics: Option<TectonicsWorld>,
    geology: Option<GeologyWorld>,
    climate: Option<ClimateWorld>,
}

/// A world holding every phase, which is the only shape the cache stores.
#[derive(Clone, Copy)]
pub struct CompleteWorld<'a> {
    pub tectonics: &'a TectonicsWorld,
    pub geology: &'a GeologyWorld,
    pub climate: &'a ClimateWorld,
}

impl GeneratedWorld {
    pub fn from_phases(
        tectonics: TectonicsWorld,
        geology: GeologyWorld,
        climate: ClimateWorld,
    ) -> Self {
        Self {
            tectonics: Some(tectonics),
            geology: Some(geology),
            climate: Some(climate),
        }
    }

    pub fn tectonics(&self) -> Option<&TectonicsWorld> {
        self.tectonics.as_ref()
    }

    pub fn geology(&self) -> Option<&GeologyWorld> {
        self.geology.as_ref()
    }

    pub fn climate(&self) -> Option<&ClimateWorld> {
        self.climate.as_ref()
    }

    pub fn holds(&self, phase: Phase) -> bool {
        match phase {
            Phase::Tectonics => self.tectonics.is_some(),
            Phase::Geology => self.geology.is_some(),
            Phase::Climate => self.climate.is_some(),
        }
    }

    pub fn complete(&self) -> Option<CompleteWorld<'_>> {
        Some(CompleteWorld {
            tectonics: self.tectonics.as_ref()?,
            geology: self.geology.as_ref()?,
            climate: self.climate.as_ref()?,
        })
    }

    /// The most refined elevation the generated phases offer, with the phase it
    /// comes from: the isostatically adjusted elevation once geology has run,
    /// and tectonic elevation before that.
    pub fn surface_elevations(&self) -> Option<(Phase, &[f32])> {
        match (&self.geology, &self.tectonics) {
            (Some(geology), _) => Some((Phase::Geology, &geology.isostasy.cell_elevations)),
            (None, Some(tectonics)) => {
                Some((Phase::Tectonics, &tectonics.elevation.cell_elevations))
            }
            (None, None) => None,
        }
    }

    /// Generates one phase over the results already in memory and drops the
    /// results it invalidates. Phases run in order, so the upstream results a
    /// phase reads are present.
    fn generate_phase(
        &mut self,
        phase: Phase,
        settings: &GenerationSettings,
    ) -> Result<(), Box<dyn Error>> {
        const UPSTREAM: &str = "a phase runs only after the phases it consumes";
        match phase {
            Phase::Tectonics => {
                self.tectonics = Some(TectonicsWorld::generate(settings.tectonics)?);
                self.geology = None;
                self.climate = None;
            }
            Phase::Geology => {
                let tectonics = self.tectonics.as_ref().ok_or(UPSTREAM)?;
                let geology = GeologyWorld::generate(tectonics, settings.geology)?;
                self.geology = Some(geology);
                self.climate = None;
            }
            Phase::Climate => {
                let tectonics = self.tectonics.as_ref().ok_or(UPSTREAM)?;
                let geology = self.geology.as_ref().ok_or(UPSTREAM)?;
                let climate = ClimateWorld::generate(tectonics, geology, settings.climate)?;
                self.climate = Some(climate);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn from_tectonics(tectonics: TectonicsWorld) -> Self {
        Self {
            tectonics: Some(tectonics),
            geology: None,
            climate: None,
        }
    }
}

impl CompleteWorld<'_> {
    pub fn settings(&self) -> GenerationSettings {
        GenerationSettings {
            tectonics: self.tectonics.config,
            geology: self.geology.config,
            climate: self.climate.config,
        }
    }

    pub fn validate(&self) -> Result<(), Box<dyn Error>> {
        self.tectonics.validate()?;
        self.geology.validate(self.tectonics)?;
        self.climate.validate(self.tectonics)?;
        Ok(())
    }
}

/// A request to generate one phase, or every phase from scratch.
#[derive(Clone, Copy, Debug, Eq, Message, PartialEq)]
pub enum GenerateRequest {
    Phase(Phase),
    AllPhases,
}

#[derive(Message, Default)]
pub struct ClearWorldCache;

#[derive(Debug, Resource)]
pub enum GenerationStatus {
    Empty {
        notice: Option<String>,
    },
    StartupLoaded {
        duration: Duration,
    },
    Generated {
        phases: Vec<Phase>,
        cache_notices: Vec<String>,
    },
    GenerationFailed {
        error: String,
    },
    CacheCleared {
        existed: bool,
    },
    CacheClearFailed {
        error: String,
    },
}

#[derive(Default)]
pub struct WorldModelPlugin {
    pub cache: WorldCache,
    pub settings: GenerationSettings,
}

impl Plugin for WorldModelPlugin {
    fn build(&self, app: &mut App) {
        let (world, status) = load_cached_world(&self.cache);
        let settings = world
            .complete()
            .map_or(self.settings, |complete| complete.settings());

        app.insert_resource(self.cache.clone())
            .insert_resource(settings)
            .insert_resource(status)
            .insert_resource(world);
        app.add_message::<GenerateRequest>()
            .add_message::<ClearWorldCache>()
            .add_systems(
                Update,
                (
                    generate_world.run_if(on_message::<GenerateRequest>),
                    clear_world_cache.run_if(on_message::<ClearWorldCache>),
                ),
            );
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StageTiming {
    pub label: &'static str,
    pub duration: Duration,
}

#[derive(Clone, Debug, Default)]
pub struct GenerationTimings {
    stages: Vec<StageTiming>,
}

impl GenerationTimings {
    fn record<T, E>(
        &mut self,
        label: &'static str,
        operation: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        let started = Instant::now();
        let result = operation();
        self.stages.push(StageTiming {
            label,
            duration: started.elapsed(),
        });
        result
    }

    pub fn stages(&self) -> &[StageTiming] {
        &self.stages
    }

    pub fn total(&self) -> Duration {
        self.stages.iter().map(|stage| stage.duration).sum()
    }
}

fn load_cached_world(cache: &WorldCache) -> (GeneratedWorld, GenerationStatus) {
    let started = Instant::now();
    match cache.load() {
        Ok(world) => (
            world,
            GenerationStatus::StartupLoaded {
                duration: started.elapsed(),
            },
        ),
        Err(error) => (
            GeneratedWorld::default(),
            GenerationStatus::Empty {
                notice: (!WorldCache::is_missing(&error))
                    .then(|| format!("Ignored world cache: {error}")),
            },
        ),
    }
}

fn generate_world(
    mut requests: MessageReader<GenerateRequest>,
    settings: Res<GenerationSettings>,
    cache: Res<WorldCache>,
    mut world: ResMut<GeneratedWorld>,
    mut status: ResMut<GenerationStatus>,
) {
    for &request in requests.read() {
        *status = match run_request(request, *settings, &mut world) {
            Ok(phases) => GenerationStatus::Generated {
                phases,
                cache_notices: store_complete_world(&world, &cache),
            },
            Err(error) => GenerationStatus::GenerationFailed {
                error: error.to_string(),
            },
        };
    }
}

/// Generates the requested phase, reusing the upstream results already in
/// memory and generating the ones that are missing. Returns the phases it ran.
fn run_request(
    request: GenerateRequest,
    settings: GenerationSettings,
    world: &mut GeneratedWorld,
) -> Result<Vec<Phase>, Box<dyn Error>> {
    let (first, target) = match request {
        GenerateRequest::AllPhases => (Phase::Tectonics, Phase::Climate),
        // A single phase starts at the earliest upstream phase with no result
        // yet, or at itself when everything it consumes is already in memory.
        GenerateRequest::Phase(target) => (
            Phase::ALL
                .iter()
                .copied()
                .take_while(|&phase| phase < target)
                .find(|&phase| !world.holds(phase))
                .unwrap_or(target),
            target,
        ),
    };

    let phases: Vec<_> = Phase::ALL
        .iter()
        .copied()
        .filter(|&phase| (first..=target).contains(&phase))
        .collect();
    for &phase in &phases {
        world.generate_phase(phase, &settings)?;
    }
    Ok(phases)
}

fn store_complete_world(world: &GeneratedWorld, cache: &WorldCache) -> Vec<String> {
    world
        .complete()
        .and_then(|complete| cache.store(complete).err())
        .map(|error| format!("Could not save world cache: {error}"))
        .into_iter()
        .collect()
}

fn clear_world_cache(cache: Res<WorldCache>, mut status: ResMut<GenerationStatus>) {
    *status = match cache.clear() {
        Ok(existed) => GenerationStatus::CacheCleared { existed },
        Err(error) => GenerationStatus::CacheClearFailed {
            error: error.to_string(),
        },
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Fixture, cache as test_cache, settings as test_settings};
    use std::fs;

    fn app_with(cache: WorldCache, settings: GenerationSettings) -> App {
        let mut app = App::new();
        app.add_plugins(WorldModelPlugin { cache, settings });
        app
    }

    fn generate(app: &mut App, request: GenerateRequest) {
        app.world_mut().write_message(request);
        app.update();
    }

    #[test]
    fn default_settings_generate_every_phase() {
        let settings = GenerationSettings::default();
        let tectonics = TectonicsWorld::generate(settings.tectonics).unwrap();
        let geology = GeologyWorld::generate(&tectonics, settings.geology).unwrap();
        ClimateWorld::generate(&tectonics, &geology, settings.climate).unwrap();
    }

    #[test]
    fn an_empty_cache_leaves_the_viewer_without_a_world() {
        let (cache_dir, cache) = test_cache("empty-start");
        let app = app_with(cache, test_settings(32, 41));

        let world = app.world().resource::<GeneratedWorld>();
        assert!(Phase::ALL.iter().all(|&phase| !world.holds(phase)));
        assert!(world.surface_elevations().is_none());
        assert!(matches!(
            app.world().resource::<GenerationStatus>(),
            GenerationStatus::Empty { notice: None }
        ));

        assert!(!cache_dir.exists());
    }

    #[test]
    fn corrupt_startup_cache_is_reported_and_ignored() {
        let (cache_dir, cache) = test_cache("startup-corrupt");
        fs::create_dir_all(&cache_dir).unwrap();
        fs::write(cache_dir.join("world.bin"), b"not a snapshot").unwrap();

        let app = app_with(cache, test_settings(32, 43));

        assert!(matches!(
            app.world().resource::<GenerationStatus>(),
            GenerationStatus::Empty { notice: Some(notice) } if notice.contains("Ignored")
        ));

        fs::remove_dir_all(cache_dir).unwrap();
    }

    #[test]
    fn startup_loads_cached_world_and_restores_its_settings() {
        let (cache_dir, cache) = test_cache("startup-load");
        let cached_settings = test_settings(32, 41);
        let fixture = Fixture::generate(cached_settings);
        cache.store(fixture.complete()).unwrap();

        let app = app_with(cache, test_settings(48, 42));

        assert_eq!(
            *app.world().resource::<GenerationSettings>(),
            cached_settings
        );
        let world = app.world().resource::<GeneratedWorld>();
        assert_eq!(world.complete().unwrap().settings(), cached_settings);
        assert_eq!(world.tectonics().unwrap().voronoi.cell_count(), 32);
        assert!(world.tectonics().unwrap().timings.stages().is_empty());
        assert!(matches!(
            app.world().resource::<GenerationStatus>(),
            GenerationStatus::StartupLoaded { .. }
        ));

        fs::remove_dir_all(cache_dir).unwrap();
    }

    #[test]
    fn generating_one_phase_reuses_the_upstream_results_in_memory() {
        let (cache_dir, cache) = test_cache("phase-reuse");
        let mut app = app_with(cache, test_settings(32, 51));
        generate(&mut app, GenerateRequest::Phase(Phase::Tectonics));

        // A later tectonics edit must not reach a geology-only run.
        let mut edited = *app.world().resource::<GenerationSettings>();
        let generated_tectonics = edited.tectonics;
        edited.tectonics.fibonacci.seed += 1;
        app.world_mut().insert_resource(edited);
        generate(&mut app, GenerateRequest::Phase(Phase::Geology));

        let world = app.world().resource::<GeneratedWorld>();
        assert_eq!(world.tectonics().unwrap().config, generated_tectonics);
        assert!(world.holds(Phase::Geology));
        assert!(!world.holds(Phase::Climate));
        assert!(!cache_dir.exists(), "a partial world leaves no snapshot");
    }

    #[test]
    fn generating_a_phase_generates_the_upstream_phases_it_is_missing() {
        let (cache_dir, cache) = test_cache("phase-upstream");
        let mut app = app_with(cache, test_settings(32, 52));

        generate(&mut app, GenerateRequest::Phase(Phase::Climate));

        let world = app.world().resource::<GeneratedWorld>();
        assert!(Phase::ALL.iter().all(|&phase| world.holds(phase)));

        fs::remove_dir_all(cache_dir).unwrap();
    }

    #[test]
    fn regenerating_a_phase_drops_the_downstream_results() {
        let (cache_dir, cache) = test_cache("phase-drop");
        let mut app = app_with(cache, test_settings(32, 53));
        generate(&mut app, GenerateRequest::AllPhases);
        assert!(
            Phase::ALL
                .iter()
                .all(|&phase| app.world().resource::<GeneratedWorld>().holds(phase))
        );

        generate(&mut app, GenerateRequest::Phase(Phase::Tectonics));

        let world = app.world().resource::<GeneratedWorld>();
        assert!(world.holds(Phase::Tectonics));
        assert!(!world.holds(Phase::Geology));
        assert!(!world.holds(Phase::Climate));
        assert_eq!(
            world.surface_elevations().unwrap(),
            (
                Phase::Tectonics,
                &world.tectonics().unwrap().elevation.cell_elevations[..]
            )
        );

        fs::remove_dir_all(cache_dir).unwrap();
    }

    #[test]
    fn only_a_complete_world_reaches_the_cache() {
        let (cache_dir, cache) = test_cache("complete-only");
        let mut app = app_with(cache.clone(), test_settings(32, 54));

        generate(&mut app, GenerateRequest::Phase(Phase::Geology));
        let load_error = cache.load().err().expect("a partial world is not cached");
        assert!(WorldCache::is_missing(&load_error));

        generate(&mut app, GenerateRequest::Phase(Phase::Climate));
        let stored = cache.load().unwrap();
        assert_eq!(
            stored.complete().unwrap().settings(),
            *app.world().resource::<GenerationSettings>()
        );

        fs::remove_dir_all(cache_dir).unwrap();
    }

    #[test]
    fn generating_every_phase_replaces_the_cached_snapshot() {
        let (cache_dir, cache) = test_cache("regenerate");
        let previous = Fixture::generate(test_settings(32, 55));
        cache.store(previous.complete()).unwrap();
        let mut app = app_with(cache.clone(), GenerationSettings::default());
        let requested = test_settings(64, 56);
        app.world_mut().insert_resource(requested);

        generate(&mut app, GenerateRequest::AllPhases);

        let world = app.world().resource::<GeneratedWorld>();
        assert_eq!(world.complete().unwrap().settings(), requested);
        assert_eq!(world.tectonics().unwrap().voronoi.cell_count(), 64);
        assert!(matches!(
            app.world().resource::<GenerationStatus>(),
            GenerationStatus::Generated { phases, .. } if phases == Phase::ALL
        ));
        let cached = cache.load().unwrap();
        assert_eq!(cached.complete().unwrap().settings(), requested);
        assert_eq!(
            cached.geology().unwrap().terrain_control_bake,
            world.geology().unwrap().terrain_control_bake
        );

        fs::remove_dir_all(cache_dir).unwrap();
    }
}
