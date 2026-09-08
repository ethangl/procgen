//! Pilot application for GPU-resident tectonics on a cube-sphere raster.
//!
//! The app is a pure consumer: it uploads settings, asks
//! `procgen-raster-tectonics` to run its kernels on Bevy's own device, and
//! renders six face grids that sample the resulting buffers. No generation
//! logic lives here.

mod camera;
mod partition;
mod render;
mod ui;

use bevy::prelude::*;
use bevy_egui::EguiPlugin;

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
            camera::OrbitCameraPlugin,
            partition::PlatePartitionPlugin,
            render::FaceGridRenderPlugin,
            ui::RasterViewerUiPlugin,
        ))
        .run();
}
