use bevy::{
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    prelude::*,
};
use bevy_egui::input::egui_wants_any_pointer_input;

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
pub struct OrbitCamera;

/// How close and how far the camera may orbit the origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitLimits {
    pub min_distance: f32,
    pub max_distance: f32,
}

/// Where the camera is looking from. The plugin owns it; applications drive it
/// with the mouse and read it off their camera's transform.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
    limits: OrbitLimits,
}

impl Orbit {
    fn new(limits: OrbitLimits) -> Self {
        Self {
            yaw: START_YAW,
            pitch: START_PITCH,
            distance: START_DISTANCE.clamp(limits.min_distance, limits.max_distance),
            limits,
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
            .clamp(self.limits.min_distance, self.limits.max_distance);
    }
}

/// Drives every [`OrbitCamera`]'s transform from mouse drag and scroll.
///
/// The application spawns its own camera entity, so each viewer keeps its own
/// projection, render layers, and tone mapping.
pub struct OrbitCameraPlugin {
    pub limits: OrbitLimits,
}

impl Plugin for OrbitCameraPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Orbit::new(self.limits)).add_systems(
            Update,
            (
                read_input.run_if(not(egui_wants_any_pointer_input)),
                apply_orbit.run_if(resource_changed::<Orbit>),
            )
                .chain(),
        );
    }
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

    fn orbit() -> Orbit {
        Orbit::new(OrbitLimits {
            min_distance: 1.05,
            max_distance: 8.0,
        })
    }

    #[test]
    fn downward_drag_increases_camera_pitch() {
        let mut orbit = orbit();
        let initial = orbit.pitch;
        orbit.drag(Vec2::new(0.0, 10.0));
        assert!(orbit.pitch > initial);
    }

    #[test]
    fn trackpad_pixels_zoom_more_gently_than_wheel_lines() {
        let mut pixels = orbit();
        let mut lines = orbit();
        pixels.zoom(10.0, MouseScrollUnit::Pixel);
        lines.zoom(1.0, MouseScrollUnit::Line);
        assert!(pixels.distance > lines.distance);
    }

    #[test]
    fn zoom_stops_at_the_configured_limits() {
        let mut orbit = orbit();
        orbit.zoom(10_000.0, MouseScrollUnit::Line);
        assert_eq!(orbit.distance, orbit.limits.min_distance);
        orbit.zoom(-10_000.0, MouseScrollUnit::Line);
        assert_eq!(orbit.distance, orbit.limits.max_distance);
    }

    #[test]
    fn the_transform_looks_at_the_origin_from_the_orbit_distance() {
        let transform = orbit().transform();
        assert!((transform.translation.length() - START_DISTANCE).abs() <= 1.0e-6);
        assert!(transform.forward().dot(-transform.translation.normalize()) > 0.999);
    }
}
