//! The viewer's camera entity. The orbit controls themselves are shared with
//! the raster viewer through `procgen-viewer-support`.

use bevy::{
    camera::PerspectiveProjection, camera::visibility::RenderLayers,
    core_pipeline::tonemapping::Tonemapping, prelude::*,
};
use procgen_viewer_support::{OrbitCamera, OrbitCameraPlugin, OrbitLimits};

/// Closest orbit altitude above the unit surface: 6.371 km at Earth scale.
pub(crate) const MIN_CAMERA_ALTITUDE: f32 = 0.001;
/// Perspective near plane: 63.71 m at Earth scale.
pub(crate) const CAMERA_NEAR_PLANE: f32 = 0.000_01;
const MAX_CAMERA_DISTANCE: f32 = 12.0;

pub struct ViewerCameraPlugin;

impl Plugin for ViewerCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(OrbitCameraPlugin {
            limits: OrbitLimits {
                min_distance: 1.0 + MIN_CAMERA_ALTITUDE,
                max_distance: MAX_CAMERA_DISTANCE,
            },
        })
        .add_systems(Startup, spawn_camera);
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
