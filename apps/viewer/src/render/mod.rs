mod assets;
mod layers;

pub use layers::DiagnosticLayer;

use crate::{camera::ViewerCamera, model::GeneratedWorld};
use bevy::{camera::visibility::RenderLayers, gizmos::config::GizmoLineConfig, prelude::*};

const DEPTH_SCALE_STEP: f32 = 0.004;

#[derive(Resource)]
pub struct LayerSettings {
    visible: [bool; DiagnosticLayer::COUNT],
}

impl LayerSettings {
    pub fn is_visible(&self, layer: DiagnosticLayer) -> bool {
        self.visible[layer.index()]
    }

    pub fn set_visible(&mut self, layer: DiagnosticLayer, visible: bool) {
        self.visible[layer.index()] = visible;
    }

    fn depth_scale(&self, layer: DiagnosticLayer) -> f32 {
        let layer_order = layer.depth_order();
        let visible_layers_before = DiagnosticLayer::ALL
            .iter()
            .filter(|&&candidate| {
                self.is_visible(candidate) && candidate.depth_order() < layer_order
            })
            .count();

        1.0 + visible_layers_before as f32 * DEPTH_SCALE_STEP
    }
}

impl Default for LayerSettings {
    fn default() -> Self {
        let mut visible = [false; DiagnosticLayer::COUNT];
        visible[DiagnosticLayer::IsostaticElevation.index()] = true;
        Self { visible }
    }
}

#[derive(Resource)]
struct DiagnosticAssets([Handle<GizmoAsset>; DiagnosticLayer::COUNT]);

pub struct DiagnosticRenderPlugin;

impl Plugin for DiagnosticRenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LayerSettings>()
            .add_systems(Startup, setup_scene)
            .add_systems(
                Update,
                (
                    rebuild_diagnostic_assets.run_if(resource_changed::<GeneratedWorld>),
                    sync_layer_render_state.run_if(resource_changed::<LayerSettings>),
                ),
            );
    }
}

fn setup_scene(
    mut commands: Commands,
    mut gizmo_assets: ResMut<Assets<GizmoAsset>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(0.985).mesh().ico(5).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.025, 0.035, 0.055),
            perceptual_roughness: 1.0,
            unlit: true,
            ..default()
        })),
    ));

    let diagnostic_assets = std::array::from_fn(|index| {
        let layer = DiagnosticLayer::ALL[index];
        let handle = gizmo_assets.add(GizmoAsset::new());
        spawn_layer(&mut commands, handle.clone(), layer);
        handle
    });
    spawn_axes(&mut commands, &mut gizmo_assets);

    commands.insert_resource(DiagnosticAssets(diagnostic_assets));
}

fn spawn_layer(commands: &mut Commands, handle: Handle<GizmoAsset>, layer: DiagnosticLayer) {
    commands.spawn((
        Gizmo {
            handle,
            line_config: GizmoLineConfig {
                width: layer.line_width(),
                perspective: false,
                ..default()
            },
            depth_bias: -0.0005,
        },
        layer,
        RenderLayers::layer(layer.render_layer()),
    ));
}

fn spawn_axes(commands: &mut Commands, assets: &mut Assets<GizmoAsset>) {
    let mut axes = GizmoAsset::new();
    axes.line(
        Vec3::X * 1.08,
        Vec3::X * 1.35,
        Color::srgb(0.95, 0.25, 0.25),
    );
    axes.line(Vec3::Y * 1.08, Vec3::Y * 1.35, Color::srgb(0.3, 0.9, 0.4));
    axes.line(Vec3::Z * 1.08, Vec3::Z * 1.35, Color::srgb(0.3, 0.55, 1.0));
    commands.spawn(Gizmo {
        handle: assets.add(axes),
        line_config: GizmoLineConfig {
            width: 3.0,
            ..default()
        },
        ..default()
    });
}

fn rebuild_diagnostic_assets(
    world: Res<GeneratedWorld>,
    assets: Res<DiagnosticAssets>,
    mut gizmo_assets: ResMut<Assets<GizmoAsset>>,
) {
    for &layer in DiagnosticLayer::ALL {
        *gizmo_assets.get_mut(&assets.0[layer.index()]).unwrap() = layer.build_asset(&world);
    }
}

fn sync_layer_render_state(
    settings: Res<LayerSettings>,
    mut camera_layers: Single<&mut RenderLayers, With<ViewerCamera>>,
    mut layer_transforms: Query<(&DiagnosticLayer, &mut Transform)>,
) {
    for (layer, mut transform) in &mut layer_transforms {
        transform.scale = Vec3::splat(settings.depth_scale(*layer));
    }

    // Retained gizmos ignore `Visibility`, so filtering must happen on the camera's render layers.
    let mut layers = vec![0];
    layers.extend(
        DiagnosticLayer::ALL
            .iter()
            .filter(|&&layer| settings.is_visible(layer))
            .map(|&layer| layer.render_layer()),
    );
    **camera_layers = RenderLayers::from_layers(&layers);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_scales_only_count_visible_layers() {
        let mut settings = LayerSettings {
            visible: [false; DiagnosticLayer::COUNT],
        };
        settings.set_visible(DiagnosticLayer::Motion, true);

        assert_eq!(settings.depth_scale(DiagnosticLayer::Motion), 1.0);
    }

    #[test]
    fn depth_scales_follow_semantic_bucket_order() {
        let mut settings = LayerSettings {
            visible: [false; DiagnosticLayer::COUNT],
        };
        settings.set_visible(DiagnosticLayer::Motion, true);
        settings.set_visible(DiagnosticLayer::Boundaries, true);
        settings.set_visible(DiagnosticLayer::IsostaticElevation, true);

        assert_eq!(
            settings.depth_scale(DiagnosticLayer::IsostaticElevation),
            1.0
        );
        assert_eq!(
            settings.depth_scale(DiagnosticLayer::Boundaries),
            1.0 + DEPTH_SCALE_STEP
        );
        assert_eq!(
            settings.depth_scale(DiagnosticLayer::Motion),
            1.0 + 2.0 * DEPTH_SCALE_STEP
        );
    }
}
