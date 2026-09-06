mod assets;
mod layers;
mod palette;
mod surfaces;

pub use layers::{DiagnosticLayer, OverlayKind};

use crate::{camera::ViewerCamera, model::GeneratedWorld};
use bevy::{camera::visibility::RenderLayers, gizmos::config::GizmoLineConfig, prelude::*};
use std::collections::BTreeSet;
use surfaces::empty_surface_mesh;

const DEPTH_SCALE_STEP: f32 = 0.004;

#[derive(Resource, Default)]
pub struct OverlaySettings {
    visible: BTreeSet<DiagnosticLayer>,
}

impl OverlaySettings {
    pub fn is_visible(&self, layer: DiagnosticLayer) -> bool {
        self.visible.contains(&layer)
    }

    pub fn set_visible(&mut self, layer: DiagnosticLayer, visible: bool) {
        assert!(
            layer.overlay_kind().is_some(),
            "only overlays can be toggled"
        );
        if visible {
            self.visible.insert(layer);
        } else {
            self.visible.remove(&layer);
        }
    }

    fn depth_scale(&self, surface: &SurfaceSelection, layer: DiagnosticLayer) -> f32 {
        if layer.is_fill() {
            return 1.0;
        }
        let layer_order = (layer.overlay_kind().unwrap(), layer.index());
        let visible_layers_before = usize::from(surface.selected().is_some())
            + self
                .visible
                .iter()
                .filter(|&&candidate| {
                    (candidate.overlay_kind().unwrap(), candidate.index()) < layer_order
                })
                .count();

        1.0 + visible_layers_before as f32 * DEPTH_SCALE_STEP
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

#[derive(Resource)]
struct DiagnosticAssets(Vec<(DiagnosticLayer, Handle<GizmoAsset>)>);

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
    let diagnostic_assets = DiagnosticLayer::ALL
        .iter()
        .filter_map(|&layer| {
            let line_width = layer.gizmo_line_width()?;
            let handle = gizmo_assets.add(GizmoAsset::new());
            spawn_layer(&mut commands, handle.clone(), layer, line_width);
            Some((layer, handle))
        })
        .collect();
    spawn_axes(&mut commands, &mut gizmo_assets);

    commands.insert_resource(DiagnosticAssets(diagnostic_assets));
}

fn spawn_layer(
    commands: &mut Commands,
    handle: Handle<GizmoAsset>,
    layer: DiagnosticLayer,
    line_width: f32,
) {
    commands.spawn((
        Gizmo {
            handle,
            line_config: GizmoLineConfig {
                width: line_width,
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
    for (layer, handle) in &assets.0 {
        *gizmo_assets.get_mut(handle).unwrap() = layer
            .build_gizmo(&world)
            .expect("diagnostic assets only contains gizmo layers");
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
        *meshes.get_mut(&surface_mesh.0).unwrap() = layer.build_surface(&world);
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
        transform.scale = Vec3::splat(overlays.depth_scale(&surface, *layer));
    }

    // Retained gizmos ignore `Visibility`, so filtering must happen on the camera's render layers.
    let mut layers = vec![0];
    layers.extend(
        surface
            .selected()
            .filter(|layer| layer.gizmo_line_width().is_some())
            .map(DiagnosticLayer::render_layer),
    );
    layers.extend(
        overlays
            .visible
            .iter()
            .copied()
            .map(DiagnosticLayer::render_layer),
    );
    **camera_layers = RenderLayers::from_layers(&layers);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_scales_only_count_visible_layers() {
        let surface = SurfaceSelection(None);
        let mut overlays = OverlaySettings::default();
        overlays.set_visible(DiagnosticLayer::Motion, true);

        assert_eq!(overlays.depth_scale(&surface, DiagnosticLayer::Motion), 1.0);
    }

    #[test]
    fn depth_scales_follow_overlay_kind_order() {
        let surface = SurfaceSelection::default();
        let mut overlays = OverlaySettings::default();
        overlays.set_visible(DiagnosticLayer::Motion, true);
        overlays.set_visible(DiagnosticLayer::Boundaries, true);

        assert_eq!(
            overlays.depth_scale(&surface, DiagnosticLayer::IsostaticElevation),
            1.0
        );
        assert_eq!(
            overlays.depth_scale(&surface, DiagnosticLayer::Boundaries),
            1.0 + DEPTH_SCALE_STEP
        );
        assert_eq!(
            overlays.depth_scale(&surface, DiagnosticLayer::Motion),
            1.0 + 2.0 * DEPTH_SCALE_STEP
        );
    }
}
