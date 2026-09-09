//! Pilot application for GPU-resident tectonics on a cube-sphere raster.
//!
//! The app is a pure consumer: it uploads settings, asks
//! `procgen-raster-tectonics` to run its kernels on Bevy's own device, and
//! renders six face grids that sample the resulting buffers. No generation
//! logic lives here.

mod render;
mod tectonics;
mod ui;

use bevy::{camera::PerspectiveProjection, core_pipeline::tonemapping::Tonemapping, prelude::*};
use bevy_egui::EguiPlugin;
use procgen_viewer_support::{OrbitCamera, OrbitCameraPlugin, OrbitLimits};

/// The camera orbits the display sphere of radius one and stops just above it.
const ORBIT_LIMITS: OrbitLimits = OrbitLimits {
    min_distance: 1.05,
    max_distance: 8.0,
};

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.012, 0.016, 0.025)))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Procgen raster tectonics".into(),
                resolution: (1280, 800).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((
            EguiPlugin::default(),
            OrbitCameraPlugin {
                limits: ORBIT_LIMITS,
            },
            tectonics::TectonicsPlugin,
            render::FaceGridRenderPlugin,
            ui::RasterViewerUiPlugin,
        ))
        .add_systems(Startup, spawn_camera)
        .run();
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection::default()),
        Tonemapping::None,
        OrbitCamera,
    ));
}
