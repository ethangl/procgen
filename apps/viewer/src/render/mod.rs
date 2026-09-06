mod assets;
mod layers;
mod palette;
mod surfaces;

pub use layers::{DiagnosticLayer, OverlayKind};

use crate::{camera::ViewerCamera, model::GeneratedWorld};
use bevy::{camera::visibility::RenderLayers, gizmos::config::GizmoLineConfig, prelude::*};
use layers::GizmoSpec;
use surfaces::empty_surface_mesh;

const SURFACE_RADIUS: f32 = 1.0;
const DEPTH_SCALE_STEP: f32 = 0.004;

#[derive(Resource)]
pub struct OverlaySettings {
    visible: [bool; DiagnosticLayer::COUNT],
}

impl OverlaySettings {
    pub fn is_visible(&self, layer: DiagnosticLayer) -> bool {
        self.visible[layer.index()]
    }

    pub fn set_visible(&mut self, layer: DiagnosticLayer, visible: bool) {
        assert!(
            layer.overlay_kind().is_some(),
            "only overlays can be toggled"
        );
        self.visible[layer.index()] = visible;
    }

    fn depth_scale(&self, layer: DiagnosticLayer) -> Option<f32> {
        let layer_order = layer.depth_order()?;
        let visible_layers_before = DiagnosticLayer::ALL
            .iter()
            .filter(|&&candidate| {
                self.is_visible(candidate)
                    && candidate
                        .depth_order()
                        .is_some_and(|order| order < layer_order)
            })
            .count();

        Some(1.0 + (visible_layers_before + 1) as f32 * DEPTH_SCALE_STEP)
    }
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self {
            visible: [false; DiagnosticLayer::COUNT],
        }
    }
}

#[derive(Resource)]
pub struct SurfaceSelection(Option<DiagnosticLayer>);

impl SurfaceSelection {
    pub fn selected(&self) -> Option<DiagnosticLayer> {
        self.0
    }

    pub fn set(&mut self, selected: Option<DiagnosticLayer>) {
        assert!(selected.is_none_or(DiagnosticLayer::is_fill));
        self.0 = selected;
    }
}

impl Default for SurfaceSelection {
    fn default() -> Self {
        Self(Some(DiagnosticLayer::IsostaticElevation))
    }
}

#[derive(Component)]
struct SurfaceLayer;

pub struct DiagnosticRenderPlugin;

impl Plugin for DiagnosticRenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SurfaceSelection>()
            .init_resource::<OverlaySettings>()
            .add_systems(Startup, setup_scene)
            .add_systems(
                Update,
                (
                    rebuild_diagnostic_assets.run_if(resource_changed::<GeneratedWorld>),
                    rebuild_surface.run_if(
                        resource_changed::<GeneratedWorld>.or(resource_changed::<SurfaceSelection>),
                    ),
                    sync_layer_render_state.run_if(
                        resource_changed::<SurfaceSelection>
                            .or(resource_changed::<OverlaySettings>),
                    ),
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

    let surface = meshes.add(empty_surface_mesh());
    commands.spawn((
        Mesh3d(surface.clone()),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            unlit: true,
            ..default()
        })),
        SurfaceLayer,
        Visibility::Hidden,
    ));
    for &layer in DiagnosticLayer::ALL {
        if let Some(gizmo) = layer.gizmo() {
            let handle = gizmo_assets.add(GizmoAsset::new());
            spawn_layer(&mut commands, handle, layer, gizmo);
        }
    }
    spawn_axes(&mut commands, &mut gizmo_assets);
}

fn spawn_layer(
    commands: &mut Commands,
    handle: Handle<GizmoAsset>,
    layer: DiagnosticLayer,
    spec: GizmoSpec,
) {
    commands.spawn((
        Gizmo {
            handle,
            line_config: GizmoLineConfig {
                width: spec.line_width(),
                perspective: false,
                ..default()
            },
            depth_bias: -0.0005,
        },
        layer,
        spec,
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
    gizmos: Query<(&GizmoSpec, &Gizmo)>,
    mut gizmo_assets: ResMut<Assets<GizmoAsset>>,
) {
    for (spec, gizmo) in &gizmos {
        *gizmo_assets.get_mut(&gizmo.handle).unwrap() = spec.build(&world);
    }
}

fn rebuild_surface(
    world: Res<GeneratedWorld>,
    selection: Res<SurfaceSelection>,
    mut meshes: ResMut<Assets<Mesh>>,
    surface: Single<(&Mesh3d, &mut Visibility), With<SurfaceLayer>>,
) {
    let (surface_mesh, mut visibility) = surface.into_inner();
    if let Some(layer) = selection.selected() {
        *meshes.get_mut(&surface_mesh.0).unwrap() = layer
            .surface()
            .expect("surface selection only stores fill layers")
            .build(&world);
        *visibility = Visibility::Inherited;
    } else {
        *visibility = Visibility::Hidden;
    }
}

fn sync_layer_render_state(
    surface: Res<SurfaceSelection>,
    overlays: Res<OverlaySettings>,
    mut camera_layers: Single<&mut RenderLayers, With<ViewerCamera>>,
    mut layer_transforms: Query<(&DiagnosticLayer, &mut Transform)>,
) {
    for (layer, mut transform) in &mut layer_transforms {
        if let Some(depth_scale) = overlays.depth_scale(*layer) {
            transform.scale = Vec3::splat(depth_scale);
        }
    }

    // Retained gizmos ignore `Visibility`, so filtering must happen on the camera's render layers.
    let mut layers = vec![0];
    layers.extend(
        surface
            .selected()
            .filter(|layer| layer.gizmo().is_some())
            .map(DiagnosticLayer::render_layer),
    );
    layers.extend(
        DiagnosticLayer::ALL
            .iter()
            .copied()
            .filter(|&layer| overlays.is_visible(layer))
            .map(DiagnosticLayer::render_layer),
    );
    **camera_layers = RenderLayers::from_layers(&layers);
}

fn to_bevy(point: procgen_core::Vec3) -> Vec3 {
    Vec3::new(point.x, point.y, point.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_scales_only_count_visible_layers() {
        let mut overlays = OverlaySettings::default();
        overlays.set_visible(DiagnosticLayer::Motion, true);

        assert_eq!(
            overlays.depth_scale(DiagnosticLayer::Motion),
            Some(1.0 + DEPTH_SCALE_STEP)
        );
    }

    #[test]
    fn depth_scales_follow_overlay_kind_order() {
        let mut overlays = OverlaySettings::default();
        overlays.set_visible(DiagnosticLayer::Motion, true);
        overlays.set_visible(DiagnosticLayer::Boundaries, true);

        assert_eq!(
            overlays.depth_scale(DiagnosticLayer::Boundaries),
            Some(1.0 + DEPTH_SCALE_STEP)
        );
        assert_eq!(
            overlays.depth_scale(DiagnosticLayer::Motion),
            Some(1.0 + 2.0 * DEPTH_SCALE_STEP)
        );
    }
}
