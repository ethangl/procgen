mod camera;
mod model;
mod render;
mod ui;

use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use camera::OrbitCameraPlugin;
use model::{GeneratedWorld, ViewerSettings};
use render::TopologyRenderPlugin;
use ui::ViewerUiPlugin;

fn main() {
    let settings = ViewerSettings::default();
    let world = GeneratedWorld::generate(&settings).expect("default world generation must succeed");

    App::new()
        .insert_resource(ClearColor(Color::srgb(0.012, 0.016, 0.025)))
        .insert_resource(settings)
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
            OrbitCameraPlugin,
            TopologyRenderPlugin,
            ViewerUiPlugin,
        ))
        .run();
}
