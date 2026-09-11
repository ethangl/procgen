//! The viewer's camera entity and the orbit controls that drive it.

use bevy::{
    camera::PerspectiveProjection,
    camera::visibility::RenderLayers,
    core_pipeline::tonemapping::Tonemapping,
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    prelude::*,
};
use bevy_egui::input::egui_wants_any_pointer_input;

/// Closest orbit altitude above the unit surface: 6.371 km at Earth scale.
pub(crate) const MIN_CAMERA_ALTITUDE: f32 = 0.001;
/// Perspective near plane: 63.71 m at Earth scale.
pub(crate) const CAMERA_NEAR_PLANE: f32 = 0.000_01;

const MIN_CAMERA_DISTANCE: f32 = 1.0 + MIN_CAMERA_ALTITUDE;
const MAX_CAMERA_DISTANCE: f32 = 12.0;
const DRAG_SENSITIVITY: f32 = 0.006;
const LINE_ZOOM_SENSITIVITY: f32 = 0.08;
const PIXEL_ZOOM_SENSITIVITY: f32 = 0.0025;
const START_YAW: f32 = 0.7;
const START_PITCH: f32 = 0.35;
const START_DISTANCE: f32 = 3.2;
/// Pitch stops short of the poles so the view never gimbals.
const MAX_PITCH: f32 = 1.5;

/// Marks the camera the orbit controls drive.
#[derive(Component)]
pub(crate) struct OrbitCamera;

/// Where the camera is looking from. The plugin owns it; the mouse drives it
/// and the camera's transform reads it back.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Orbit {
    fn new() -> Self {
        Self {
            yaw: START_YAW,
            pitch: START_PITCH,
            distance: START_DISTANCE.clamp(MIN_CAMERA_DISTANCE, MAX_CAMERA_DISTANCE),
        }
    }

    /// The camera transform this orbit describes.
    fn transform(&self) -> Transform {
        let horizontal = self.distance * self.pitch.cos();
        let position = Vec3::new(
            horizontal * self.yaw.sin(),
            self.distance * self.pitch.sin(),
            horizontal * self.yaw.cos(),
        );
        Transform::from_translation(position).looking_at(Vec3::ZERO, Vec3::Y)
    }

    fn drag(&mut self, delta: Vec2) {
        self.yaw -= delta.x * DRAG_SENSITIVITY;
        self.pitch = (self.pitch + delta.y * DRAG_SENSITIVITY).clamp(-MAX_PITCH, MAX_PITCH);
    }

    fn zoom(&mut self, delta: f32, unit: MouseScrollUnit) {
        let sensitivity = match unit {
            MouseScrollUnit::Line => LINE_ZOOM_SENSITIVITY,
            MouseScrollUnit::Pixel => PIXEL_ZOOM_SENSITIVITY,
        };
        self.distance = (self.distance * (-delta * sensitivity).exp())
            .clamp(MIN_CAMERA_DISTANCE, MAX_CAMERA_DISTANCE);
    }
}

/// Spawns the viewer's camera and drives its transform from mouse drag and
/// scroll.
pub struct ViewerCameraPlugin;

impl Plugin for ViewerCameraPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Orbit::new())
            .add_systems(Startup, spawn_camera)
            .add_systems(
                Update,
                (
                    read_input.run_if(not(egui_wants_any_pointer_input)),
                    apply_orbit.run_if(resource_changed::<Orbit>),
                )
                    .chain(),
            );
    }
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            near: CAMERA_NEAR_PLANE,
            ..default()
        }),
        Tonemapping::None,
        RenderLayers::layer(0),
        OrbitCamera,
    ));
}

fn read_input(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut orbit: ResMut<Orbit>,
) {
    let mut next = *orbit;
    if buttons.pressed(MouseButton::Left) {
        next.drag(motion.delta);
    }
    if scroll.delta.y != 0.0 {
        next.zoom(scroll.delta.y, scroll.unit);
    }
    orbit.set_if_neq(next);
}

fn apply_orbit(orbit: Res<Orbit>, cameras: Query<&mut Transform, With<OrbitCamera>>) {
    let transform = orbit.transform();
    for mut camera in cameras {
        *camera = transform;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downward_drag_increases_camera_pitch() {
        let mut orbit = Orbit::new();
        let initial = orbit.pitch;
        orbit.drag(Vec2::new(0.0, 10.0));
        assert!(orbit.pitch > initial);
    }

    #[test]
    fn trackpad_pixels_zoom_more_gently_than_wheel_lines() {
        let mut pixels = Orbit::new();
        let mut lines = Orbit::new();
        pixels.zoom(10.0, MouseScrollUnit::Pixel);
        lines.zoom(1.0, MouseScrollUnit::Line);
        assert!(pixels.distance > lines.distance);
    }

    #[test]
    fn zoom_stops_at_the_configured_limits() {
        let mut orbit = Orbit::new();
        orbit.zoom(10_000.0, MouseScrollUnit::Line);
        assert_eq!(orbit.distance, MIN_CAMERA_DISTANCE);
        orbit.zoom(-10_000.0, MouseScrollUnit::Line);
        assert_eq!(orbit.distance, MAX_CAMERA_DISTANCE);
    }

    #[test]
    fn the_transform_looks_at_the_origin_from_the_orbit_distance() {
        let transform = Orbit::new().transform();
        assert!((transform.translation.length() - START_DISTANCE).abs() <= 1.0e-6);
        assert!(transform.forward().dot(-transform.translation.normalize()) > 0.999);
    }
}
