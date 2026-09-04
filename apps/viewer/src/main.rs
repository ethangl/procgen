mod camera;
mod model;
mod render;
mod ui;

use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use camera::OrbitCameraPlugin;
use model::{GeneratedWorld, GenerationSettings, WorldModelPlugin};
use render::{LayerSettings, TopologyRenderPlugin};
use ui::ViewerUiPlugin;

fn main() {
    let settings = GenerationSettings::default();
    let world = GeneratedWorld::generate(settings.fibonacci)
        .expect("default world generation must succeed");

    App::new()
        .insert_resource(ClearColor(Color::srgb(0.012, 0.016, 0.025)))
        .insert_resource(settings)
        .insert_resource(LayerSettings::default())
        .insert_resource(world)
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Procgen sphere viewer".into(),
                resolution: (1280, 800).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((
            EguiPlugin::default(),
            WorldModelPlugin,
            OrbitCameraPlugin,
            TopologyRenderPlugin,
            ViewerUiPlugin,
        ))
        .run();
}
