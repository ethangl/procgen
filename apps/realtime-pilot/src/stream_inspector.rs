use crate::replay::Replay;
use crate::stream_record::{FrameTiming, Recording, RecordingConfig, RecordingRoute};
use crate::stream_render::{UploadBridge, chunk_mesh, full_visibility};
use crate::surface_material::{SurfaceMaterial, SurfacePlugin};
use crate::surface_view::SurfaceViewConfig;
use crate::usable_inspector::{UsableState, population};
use bevy::{
    camera::visibility::VisibilityRange,
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use procgen_core::Vec3 as Point;
use procgen_realtime_pilot::REPLACEMENT_SECONDS;
use procgen_realtime_pilot::{
    DetailSource, FAST_FLIGHT_SPEED, FLIGHT_SPEED, INSTALL_MILLIS_PER_FRAME, OVERVIEW_FACE_QUADS,
    ROUTE_SECONDS, Scenario, StreamEvent, StreamView, StreamingWorld, Ticket,
    UPLOAD_BYTES_PER_FRAME, USABLE_MEMORY_RESERVATION, UsableError, UsableTerrain, planet_overview,
    prepare_detail, streaming_route,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::Instant,
};

enum PreparedStage {
    Surface(Arc<DetailSource>),
    Usable(UsableTerrain),
}
type SourceResult = Result<PreparedStage, UsableError>;
enum Fade {
    In(f32),
    Out(f32),
}
struct Region {
    entities: Vec<Entity>,
    handles: Vec<Handle<Mesh>>,
    committed: bool,
    fade: Option<Fade>,
}
struct Inspector {
    receiver: Option<Receiver<SourceResult>>,
    cancel: Arc<AtomicBool>,
    world: Option<StreamingWorld>,
    regions: BTreeMap<Ticket, Region>,
    status: String,
    record: Option<Recording>,
    scenario: Scenario,
    surface_view: SurfaceViewConfig,
    replay: Option<Replay>,
    last_install_ms: f64,
    last_upload_bytes: usize,
}
impl Drop for Inspector {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
#[derive(Resource)]
pub(crate) struct CameraState {
    pub(crate) position: Vec3,
    forward: Vec3,
}
impl CameraState {
    fn view(&self) -> StreamView {
        StreamView {
            position: Point::new(self.position.x, self.position.y, self.position.z),
            forward: Point::new(self.forward.x, self.forward.y, self.forward.z),
        }
    }
    fn transform(&self, up: Vec3) -> Transform {
        let camera_up = if self.forward.dot(up).abs() > 0.999 {
            self.forward.any_orthonormal_vector()
        } else {
            up
        };
        Transform::from_translation(self.position).looking_to(self.forward, camera_up)
    }
}
#[derive(Component)]
struct FlightCamera;
#[derive(Component)]
struct Overview;
#[derive(Resource)]
struct TerrainMaterial(Handle<SurfaceMaterial>);

pub fn run(
    scenario: Scenario,
    surface_view: SurfaceViewConfig,
    record: Option<RecordingConfig>,
    replay: Option<Replay>,
) -> Result<(), Box<dyn std::error::Error>> {
    let seed = scenario.seed;
    let field = scenario.validate()?;
    let overview = planet_overview(&field, OVERVIEW_FACE_QUADS)?;
    let mut overview_mesh =
        crate::planet_inspector::render_mesh(&overview, Vec3::ZERO, false, scenario.planet.radius);
    overview_mesh.remove_attribute(Mesh::ATTRIBUTE_COLOR);
    let record = record
        .map(|path| Recording::new(path, scenario))
        .transpose()?;
    let (sender, receiver) = mpsc::sync_channel(2);
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    std::thread::spawn(move || {
        let result = (|| -> Result<(), UsableError> {
            let Some(source) = prepare_detail(&field, &worker_cancel)? else {
                return Ok(());
            };
            let source = Arc::new(source);
            if sender
                .send(Ok(PreparedStage::Surface(Arc::clone(&source))))
                .is_err()
            {
                return Ok(());
            }
            if worker_cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            let terrain = UsableTerrain::prepare(source, seed)?;
            if !worker_cancel.load(Ordering::Relaxed) {
                let _ = sender.send(Ok(PreparedStage::Usable(terrain)));
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = sender.send(Err(error));
        }
    });
    let state = Inspector {
        receiver: Some(receiver),
        cancel,
        world: None,
        regions: BTreeMap::new(),
        status: "Preparing bounded CPU source…".into(),
        record,
        scenario,
        surface_view,
        replay,
        last_install_ms: 0.0,
        last_upload_bytes: 0,
    };
    let exit = App::new()
        .insert_non_send_resource(state)
        .init_resource::<UsableState>()
        .insert_resource(CameraState {
            position: Vec3::Z * 13.0,
            forward: -Vec3::Z,
        })
        .insert_resource(ClearColor(Color::srgb(0.018, 0.025, 0.04)))
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 100.0,
            ..default()
        })
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Real-time world pilot — streaming".into(),
                resolution: (1280, 900).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((
            EguiPlugin::default(),
            UploadBridge::default(),
            SurfacePlugin,
        ))
        .add_systems(
            Startup,
            move |mut commands: Commands,
                  mut meshes: ResMut<Assets<Mesh>>,
                  mut materials: ResMut<Assets<StandardMaterial>>,
                  mut surface_materials: ResMut<Assets<SurfaceMaterial>>,
                  camera: Res<CameraState>| {
                commands.spawn((
                    Camera3d::default(),
                    bevy::core_pipeline::tonemapping::Tonemapping::Reinhard,
                    Projection::Perspective(PerspectiveProjection {
                        near: 0.001,
                        far: 200.0,
                        ..default()
                    }),
                    camera.transform(Vec3::Y),
                    FlightCamera,
                ));
                commands.spawn((
                    DirectionalLight {
                        illuminance: 8000.0,
                        ..default()
                    },
                    Transform::from_xyz(8.0, 12.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
                ));
                let material = materials.add(StandardMaterial {
                    base_color: Color::srgb(0.55, 0.55, 0.55),
                    perceptual_roughness: 0.9,
                    ..default()
                });
                commands.insert_resource(TerrainMaterial(
                    surface_materials.add(crate::surface_material::material(surface_view)),
                ));
                commands.spawn((
                    Mesh3d(meshes.add(overview_mesh.clone())),
                    MeshMaterial3d(material),
                    Overview,
                    full_visibility(),
                ));
            },
        )
        .add_systems(
            Update,
            (camera_input, advance, population, record_frame).chain(),
        )
        .add_systems(EguiPrimaryContextPass, ui)
        .run();
    if exit != AppExit::Success {
        return Err("native inspector failed; see its error log".into());
    }
    Ok(())
}

#[derive(bevy::ecs::system::SystemParam)]
struct FlightInput<'w> {
    buttons: Res<'w, ButtonInput<MouseButton>>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    motion: Res<'w, AccumulatedMouseMotion>,
    scroll: Res<'w, AccumulatedMouseScroll>,
    time: Res<'w, Time<Real>>,
}
fn camera_input(
    input: FlightInput,
    mut camera: ResMut<CameraState>,
    state: NonSend<Inspector>,
    mut usable: ResMut<UsableState>,
    mut contexts: EguiContexts,
) {
    if state.record.is_some() {
        return;
    }
    if contexts
        .ctx_mut()
        .is_ok_and(|ctx| ctx.wants_pointer_input() || ctx.wants_keyboard_input())
    {
        return;
    }
    if input.keys.just_pressed(KeyCode::KeyG) {
        if let Some(eye) = usable.toggle(camera.position) {
            camera.position = eye;
        }
        let up = usable.up();
        let tangent = camera.forward - up * camera.forward.dot(up);
        camera.forward = if tangent.length_squared() > 0.001 {
            tangent.normalize()
        } else {
            up.any_orthonormal_vector()
        };
    }
    let up = usable.up();
    if input.buttons.pressed(MouseButton::Left) {
        let yaw = Quat::from_axis_angle(up, -input.motion.delta.x * 0.004);
        let right = camera.forward.cross(up).normalize();
        let pitch = Quat::from_axis_angle(right, -input.motion.delta.y * 0.004);
        let forward = yaw * pitch * camera.forward;
        if forward.dot(up).abs() < 0.99 {
            camera.forward = forward.normalize();
        }
    }
    let speed = if input.keys.pressed(KeyCode::ShiftLeft) {
        FAST_FLIGHT_SPEED
    } else {
        FLIGHT_SPEED
    };
    let mut movement = Vec3::ZERO;
    let right = camera.forward.cross(up).normalize();
    for (key, direction) in [
        (KeyCode::KeyW, camera.forward),
        (KeyCode::KeyS, -camera.forward),
        (KeyCode::KeyD, right),
        (KeyCode::KeyA, -right),
        (KeyCode::KeyE, Vec3::Y),
        (KeyCode::KeyQ, -Vec3::Y),
    ] {
        if input.keys.pressed(key)
            && (usable.walker.is_none() || !matches!(key, KeyCode::KeyQ | KeyCode::KeyE))
        {
            movement += direction;
        }
    }
    if usable.walker.is_some() {
        if let Some(eye) = usable.walk(movement.normalize_or_zero(), input.time.delta_secs()) {
            camera.position = eye;
        }
        return;
    }
    camera.position += movement.normalize_or_zero() * speed * input.time.delta_secs();
    let zoom = match input.scroll.unit {
        MouseScrollUnit::Line => 0.2,
        MouseScrollUnit::Pixel => 0.006,
    };
    let forward = camera.forward;
    camera.position += forward * input.scroll.delta.y * zoom;
}

#[derive(bevy::ecs::system::SystemParam)]
struct Scene<'w, 's> {
    commands: Commands<'w, 's>,
    meshes: ResMut<'w, Assets<Mesh>>,
    material: Res<'w, TerrainMaterial>,
    bridge: Res<'w, UploadBridge>,
    cameras: Query<'w, 's, &'static mut Transform, With<FlightCamera>>,
    overview: Query<'w, 's, Entity, With<Overview>>,
    exit: MessageWriter<'w, AppExit>,
}
fn advance(
    mut state: NonSendMut<Inspector>,
    mut camera: ResMut<CameraState>,
    time: Res<Time<Real>>,
    mut scene: Scene,
    mut usable: ResMut<UsableState>,
) {
    while let Some(receiver) = &state.receiver {
        match receiver.try_recv() {
            Ok(Ok(PreparedStage::Surface(source))) => {
                match StreamingWorld::new(source, USABLE_MEMORY_RESERVATION) {
                    Ok(world) => {
                        state.world = Some(world);
                        state.status = "Installing coarse coverage…".into();
                    }
                    Err(error) => {
                        eprintln!("Streaming preparation failed: {error}");
                        state.status = error.to_string();
                        state.receiver = None;
                        if state.record.is_some() {
                            scene.exit.write(AppExit::error());
                        }
                        return;
                    }
                }
            }
            Ok(Ok(PreparedStage::Usable(terrain))) => {
                usable.terrain = Some(terrain);
                usable.status = "Collision ready · G to land".into();
                state.receiver = None;
            }
            Ok(Err(error)) => {
                usable.status = error.to_string();
                eprintln!("Preparation failed: {error}");
                state.receiver = None;
                if state.record.is_some() {
                    scene.exit.write(AppExit::error());
                }
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                usable.status = "Preparation worker stopped".into();
                state.receiver = None;
                if state.record.is_some() {
                    scene.exit.write(AppExit::error());
                }
            }
        }
    }
    if state.world.is_none() {
        return;
    }
    let is_visible = state.world.as_ref().expect("initialized world").visible();
    if is_visible || state.replay.is_some() {
        let state = &mut *state;
        if let Some(record) = &mut state.record {
            if let Some(replay) = &state.replay {
                let view = replay.sample(record.elapsed());
                camera.position = Vec3::new(view.position.x, view.position.y, view.position.z);
                camera.forward = Vec3::new(view.forward.x, view.forward.y, view.forward.z);
            } else if record.route() == RecordingRoute::Walk {
                if usable.terrain.is_none() {
                    return;
                }
                match usable.recorded_walk(record.elapsed(), time.delta_secs()) {
                    Ok((position, forward)) => {
                        camera.position = position;
                        camera.forward = forward;
                    }
                    Err(error) => {
                        eprintln!("Walking route failed: {error}");
                        state.status = error;
                        scene.exit.write(AppExit::error());
                        return;
                    }
                }
            } else {
                let route = streaming_route(record.elapsed());
                camera.position = Vec3::new(
                    route.view.position.x,
                    route.view.position.y,
                    route.view.position.z,
                );
                camera.forward = Vec3::new(
                    route.view.forward.x,
                    route.view.forward.y,
                    route.view.forward.z,
                );
            }
            if let Some(path) = record.screenshot() {
                scene
                    .commands
                    .spawn(Screenshot::primary_window())
                    .observe(save_to_disk(path));
            }
        }
    }
    for mut transform in &mut scene.cameras {
        let up = if state.replay.is_some() && state.scenario.route == RecordingRoute::Walk {
            camera.position.normalize()
        } else {
            usable.up()
        };
        *transform = camera.transform(up);
    }
    let (ready, retired) = {
        let mut bridge = scene.bridge.0.lock().expect("upload bridge");
        (
            std::mem::take(&mut bridge.ready),
            std::mem::take(&mut bridge.retired),
        )
    };
    let replay_walking = state.replay.is_some() && state.scenario.route == RecordingRoute::Walk;
    let world = state.world.as_mut().expect("initialized world");
    // Update requests before processing readiness from the previous render frame.
    if let Err(error) = world.update(
        camera.view(),
        if usable.walker.is_some() || replay_walking {
            procgen_realtime_pilot::DetailFocus::NearbyCollision
        } else {
            procgen_realtime_pilot::DetailFocus::View
        },
    ) {
        eprintln!("Streaming update failed: {error}");
        state.status = error.to_string();
        if state.record.is_some() {
            scene.exit.write(AppExit::error());
        }
        return;
    }
    let install_start = Instant::now();
    for ticket in retired {
        world.acknowledge_retirement(ticket);
    }
    for (ticket, piece) in ready {
        world.acknowledge_upload(ticket, piece);
    }
    let events = world.events();
    for event in events {
        match event {
            StreamEvent::Retire(ticket) => {
                if let Some(region) = state.regions.get_mut(&ticket)
                    && region.committed
                {
                    region.fade = Some(Fade::Out(0.0));
                } else {
                    retire_region(&mut state, ticket, &mut scene);
                }
            }
            StreamEvent::Commit(ticket) => {
                let visible = state.world.as_ref().expect("world").visible();
                let replacement = state.regions.iter().any(|(old, region)| {
                    old.face == ticket.face && matches!(region.fade, Some(Fade::Out(_)))
                });
                if let Some(region) = state.regions.get_mut(&ticket) {
                    region.committed = true;
                    if replacement {
                        region.fade = Some(Fade::In(0.0));
                    }
                    if visible {
                        for &entity in &region.entities {
                            scene.commands.entity(entity).insert(Visibility::Visible);
                        }
                    }
                }
            }
            StreamEvent::Reveal => {
                for region in state.regions.values().filter(|r| r.committed) {
                    for &entity in &region.entities {
                        scene.commands.entity(entity).insert(Visibility::Visible);
                    }
                }
                for entity in &scene.overview {
                    scene.commands.entity(entity).despawn();
                }
            }
        }
    }
    let mut finished = Vec::new();
    let distance = camera.position.length();
    for (&ticket, region) in &mut state.regions {
        if let Some(fade) = &mut region.fade {
            let (elapsed, out) = match fade {
                Fade::In(t) => (t, false),
                Fade::Out(t) => (t, true),
            };
            *elapsed += time.delta_secs();
            if *elapsed >= REPLACEMENT_SECONDS {
                if out {
                    finished.push(ticket);
                } else {
                    region.fade = None;
                    for &entity in &region.entities {
                        scene.commands.entity(entity).insert(full_visibility());
                    }
                }
            } else {
                let progress = *elapsed / REPLACEMENT_SECONDS;
                let margin = distance - progress..distance + 1.0 - progress;
                let range = if out {
                    VisibilityRange {
                        end_margin: margin,
                        ..full_visibility()
                    }
                } else {
                    VisibilityRange {
                        start_margin: margin,
                        ..full_visibility()
                    }
                };
                for &entity in &region.entities {
                    scene.commands.entity(entity).insert(range.clone());
                }
            }
        }
    }
    for ticket in finished {
        retire_region(&mut state, ticket, &mut scene);
    }
    let mut remaining = UPLOAD_BYTES_PER_FRAME;
    while install_start.elapsed().as_secs_f64() * 1000.0 < INSTALL_MILLIS_PER_FRAME {
        let Some(piece) = state.world.as_mut().expect("world").next_upload(remaining) else {
            break;
        };
        remaining -= piece.reserved_bytes();
        let handle = scene.meshes.add(chunk_mesh(&piece));
        let entity = scene
            .commands
            .spawn((
                Mesh3d(handle.clone()),
                MeshMaterial3d(scene.material.0.clone()),
                Visibility::Hidden,
                full_visibility(),
            ))
            .id();
        scene.bridge.0.lock().expect("upload bridge").pending.push((
            piece.ticket,
            piece.index,
            handle.id(),
        ));
        let region = state.regions.entry(piece.ticket).or_insert_with(|| Region {
            entities: Vec::new(),
            handles: Vec::new(),
            committed: false,
            fade: None,
        });
        region.entities.push(entity);
        region.handles.push(handle);
    }
    state.last_install_ms = install_start.elapsed().as_secs_f64() * 1000.0;
    state.last_upload_bytes = UPLOAD_BYTES_PER_FRAME - remaining;
    let stats = state.world.as_ref().expect("world").stats();
    state.status = format!(
        "{} jobs · {} queued\n{:.1} MiB managed\n{} cancellations · {} rejected\n{} installed",
        stats.active_jobs,
        stats.queued_jobs,
        stats.managed_bytes as f64 / (1024.0 * 1024.0),
        stats.cancellations,
        stats.rejected_results,
        stats.installations
    );
}

fn retire_region(state: &mut Inspector, ticket: Ticket, scene: &mut Scene) {
    let ids = if let Some(region) = state.regions.remove(&ticket) {
        for entity in region.entities {
            scene.commands.entity(entity).despawn();
        }
        region.handles.iter().map(Handle::id).collect()
    } else {
        Vec::new()
    };
    let mut bridge = scene.bridge.0.lock().expect("upload bridge");
    bridge.pending.retain(|(t, _, _)| *t != ticket);
    bridge.retiring.push((ticket, ids));
}

fn record_frame(
    mut state: NonSendMut<Inspector>,
    camera: Res<CameraState>,
    time: Res<Time<Real>>,
    usable: Res<UsableState>,
    mut exit: MessageWriter<AppExit>,
) {
    if !state.world.as_ref().is_some_and(|w| w.visible()) {
        return;
    }
    if state
        .record
        .as_ref()
        .is_some_and(|r| r.route() == RecordingRoute::Walk)
        && usable.walker.is_none()
        && state.replay.is_none()
    {
        return;
    }
    let timing = FrameTiming {
        seconds: time.delta_secs_f64(),
        install_ms: state.last_install_ms,
        upload_bytes: state.last_upload_bytes,
        walking_ms: usable.walking_ms,
        population_ms: usable.population_ms,
        clearance: usable.clearance(),
        grounded: usable.walker.as_ref().is_some_and(|w| w.grounded()),
        clamped_frames: usable.clamped_frames,
        collision_failures: usable.collision_failures,
    };
    let state = &mut *state;
    let Some(record) = &mut state.record else {
        return;
    };
    match record.frame(state.world.as_ref().expect("world"), camera.view(), timing) {
        Ok(Some(summary)) => {
            eprintln!("{summary}");
            exit.write(if usable.collision_failures == 0 {
                AppExit::Success
            } else {
                AppExit::error()
            });
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("Recording failed: {error}");
            state.status = error.to_string();
            exit.write(AppExit::error());
        }
    }
}
fn ui(
    mut contexts: EguiContexts,
    mut state: NonSendMut<Inspector>,
    camera: Res<CameraState>,
    usable: Res<UsableState>,
    material: Res<TerrainMaterial>,
    mut materials: ResMut<Assets<SurfaceMaterial>>,
) -> Result {
    let previous = state.surface_view;
    egui::SidePanel::left("stream-controls")
        .exact_width(280.0)
        .show(contexts.ctx_mut()?, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Streaming terrain");
            let label=procgen_realtime_pilot::PLANET_PRESETS.iter().find(|p|p.planet==state.scenario.planet).map_or("Custom parameters",|p|p.name);
            ui.label(label);
            ui.collapsing("Resolved parameters", |ui| {
                ui.monospace(format!("{:#?}",state.scenario));
                ui.label(format!("Build {}",procgen_realtime_pilot::BUILD_ID));
            });
            if let Some(replay)=&state.replay {
                ui.label(format!("Camera replay · source build {}",replay.case.build));
            }
            let recording = state.record.is_some();
            crate::surface_material::controls(ui, &mut state.surface_view, recording);
            ui.separator();
            ui.label("Drag: look · Scroll: move\nWASD: fly · Q/E: down/up\nShift: fast flight");
            ui.label(format!(
                "Flight: {FLIGHT_SPEED} · Fast: {FAST_FLIGHT_SPEED}\nModel lengths per second"
            ));
            ui.separator();
            ui.label(&state.status);
            ui.label(format!(
                "Installation: {:.2} ms\nUpload: {} KiB",
                state.last_install_ms,
                state.last_upload_bytes / 1024
            ));
            ui.label(format!(
                "Altitude: {:.2}",
                camera.position.length() - state.scenario.planet.radius
            ));
            if let Some(record) = &state.record {
                ui.separator();
                ui.label(format!(
                    "Recording: {}\n{:.1} / {ROUTE_SECONDS} seconds",
                    record.phase().0,
                    record.elapsed()
                ));
            }
            ui.separator();
            ui.label(usable.description(camera.position));
            ui.separator();
            ui.label("CPU source stays resident. Complete uploads start a short dithered replacement. G enables walking on fixed fine collision. Rocks and landmarks are visual only.");
            });
        });
    if state.surface_view != previous {
        materials
            .get_mut(&material.0)
            .expect("surface material")
            .extension = state.surface_view.into();
    }
    Ok(())
}
