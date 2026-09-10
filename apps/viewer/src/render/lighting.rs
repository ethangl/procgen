use bevy::prelude::*;

#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct LightingSettings {
    pub azimuth_degrees: f32,
    pub elevation_degrees: f32,
    pub illuminance: f32,
    pub ambient_brightness: f32,
}

impl Default for LightingSettings {
    fn default() -> Self {
        Self {
            azimuth_degrees: 20.0,
            elevation_degrees: 20.0,
            illuminance: 4_000.0,
            ambient_brightness: 1_000.0,
        }
    }
}

#[derive(Component)]
pub(super) struct ViewerDirectionalLight;

pub(super) fn spawn(commands: &mut Commands) {
    commands.spawn((
        DirectionalLight {
            shadows_enabled: false,
            ..default()
        },
        ViewerDirectionalLight,
    ));
}

pub(super) fn sync(
    settings: Res<LightingSettings>,
    light: Single<(&mut DirectionalLight, &mut Transform), With<ViewerDirectionalLight>>,
    mut ambient: ResMut<GlobalAmbientLight>,
) {
    let (mut directional_light, mut transform) = light.into_inner();
    directional_light.illuminance = settings.illuminance;
    *transform = Transform::default().looking_to(light_direction(&settings), Vec3::Y);
    ambient.brightness = settings.ambient_brightness;
}

fn light_direction(settings: &LightingSettings) -> Vec3 {
    let azimuth = settings.azimuth_degrees.to_radians();
    let elevation = settings.elevation_degrees.to_radians();
    let horizontal = elevation.cos();
    -Vec3::new(
        horizontal * azimuth.sin(),
        elevation.sin(),
        horizontal * azimuth.cos(),
    )
}
