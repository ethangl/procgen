//! Orbit camera for the raster viewer.

use bevy::{
    camera::PerspectiveProjection,
    core_pipeline::tonemapping::Tonemapping,
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    prelude::*,
};
use bevy_egui::input::egui_wants_any_pointer_input;

#[derive(Component)]
pub struct RasterCamera;

const DRAG_SENSITIVITY: f32 = 0.006;
const LINE_ZOOM_SENSITIVITY: f32 = 0.08;
const PIXEL_ZOOM_SENSITIVITY: f32 = 0.0025;
const MIN_CAMERA_DISTANCE: f32 = 1.05;
const MAX_CAMERA_DISTANCE: f32 = 8.0;

#[derive(Resource)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Default for Orbit {
    fn default() -> Self {
        Self {
            yaw: 0.7,
            pitch: 0.35,
            distance: 3.2,
        }
    }
}

pub struct OrbitCameraPlugin;

impl Plugin for OrbitCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Orbit>()
            .add_systems(Startup, spawn_camera)
            .add_systems(
                Update,
                orbit_camera.run_if(not(egui_wants_any_pointer_input)),
            );
    }
}

fn spawn_camera(mut commands: Commands, orbit: Res<Orbit>) {
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection::default()),
        Tonemapping::None,
        camera_transform(&orbit),
        RasterCamera,
    ));
}

fn orbit_camera(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut orbit: ResMut<Orbit>,
    mut camera: Single<&mut Transform, With<RasterCamera>>,
) {
    let mut changed = false;
    if buttons.pressed(MouseButton::Left) {
        apply_drag(&mut orbit, motion.delta);
        changed |= motion.delta != Vec2::ZERO;
    }
    if scroll.delta.y != 0.0 {
        apply_zoom(&mut orbit, scroll.delta.y, scroll.unit);
        changed = true;
    }
    if changed {
        **camera = camera_transform(&orbit);
    }
}

fn apply_drag(orbit: &mut Orbit, delta: Vec2) {
    orbit.yaw -= delta.x * DRAG_SENSITIVITY;
    orbit.pitch = (orbit.pitch + delta.y * DRAG_SENSITIVITY).clamp(-1.5, 1.5);
}

fn apply_zoom(orbit: &mut Orbit, delta: f32, unit: MouseScrollUnit) {
    let sensitivity = match unit {
        MouseScrollUnit::Line => LINE_ZOOM_SENSITIVITY,
        MouseScrollUnit::Pixel => PIXEL_ZOOM_SENSITIVITY,
    };
    orbit.distance = (orbit.distance * (-delta * sensitivity).exp())
        .clamp(MIN_CAMERA_DISTANCE, MAX_CAMERA_DISTANCE);
}

fn camera_transform(orbit: &Orbit) -> Transform {
    let horizontal = orbit.distance * orbit.pitch.cos();
    let position = Vec3::new(
        horizontal * orbit.yaw.sin(),
        orbit.distance * orbit.pitch.sin(),
        horizontal * orbit.yaw.cos(),
    );
    Transform::from_translation(position).looking_at(Vec3::ZERO, Vec3::Y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downward_drag_increases_camera_pitch() {
        let mut orbit = Orbit::default();
        let initial = orbit.pitch;
        apply_drag(&mut orbit, Vec2::new(0.0, 10.0));
        assert!(orbit.pitch > initial);
    }

    #[test]
    fn zoom_stops_above_the_surface() {
        let mut orbit = Orbit::default();
        apply_zoom(&mut orbit, 10_000.0, MouseScrollUnit::Line);
        assert_eq!(orbit.distance, MIN_CAMERA_DISTANCE);
    }
}
