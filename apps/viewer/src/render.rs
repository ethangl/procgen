use crate::{
    camera::ViewerCamera,
    model::{GeneratedWorld, ViewerSettings},
};
use bevy::{camera::visibility::RenderLayers, gizmos::config::GizmoLineConfig, prelude::*};
use procgen_core::Vec3 as SphereVec3;
use procgen_sphere_mesh::{SphereMesh, SphericalDelaunay};

const POINT_LAYER: usize = 1;
const DELAUNAY_LAYER: usize = 2;
const VORONOI_LAYER: usize = 3;

#[derive(Resource)]
struct TopologyAssets {
    points: Handle<GizmoAsset>,
    delaunay: Handle<GizmoAsset>,
    voronoi: Handle<GizmoAsset>,
}

pub struct TopologyRenderPlugin;

impl Plugin for TopologyRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_scene).add_systems(
            Update,
            (
                regenerate_world,
                sync_visible_layers.after(regenerate_world),
            ),
        );
    }
}

fn setup_scene(
    mut commands: Commands,
    world: Res<GeneratedWorld>,
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

    let points = gizmo_assets.add(point_asset(&world.delaunay));
    let delaunay = gizmo_assets.add(delaunay_asset(&world.delaunay));
    let voronoi = gizmo_assets.add(voronoi_asset(&world.voronoi));

    spawn_layer(&mut commands, points.clone(), POINT_LAYER, 1.8);
    spawn_layer(&mut commands, delaunay.clone(), DELAUNAY_LAYER, 1.1);
    spawn_layer(&mut commands, voronoi.clone(), VORONOI_LAYER, 1.5);
    spawn_axes(&mut commands, &mut gizmo_assets);

    commands.insert_resource(TopologyAssets {
        points,
        delaunay,
        voronoi,
    });
}

fn spawn_layer(commands: &mut Commands, handle: Handle<GizmoAsset>, layer: usize, width: f32) {
    commands.spawn((
        Gizmo {
            handle,
            line_config: GizmoLineConfig {
                width,
                perspective: false,
                ..default()
            },
            depth_bias: -0.0005,
        },
        RenderLayers::layer(layer),
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

fn regenerate_world(
    mut settings: ResMut<ViewerSettings>,
    mut world: ResMut<GeneratedWorld>,
    assets: Res<TopologyAssets>,
    mut gizmo_assets: ResMut<Assets<GizmoAsset>>,
) {
    if !settings.regenerate_requested {
        return;
    }
    settings.regenerate_requested = false;

    match GeneratedWorld::generate(&settings) {
        Ok(generated) => {
            *gizmo_assets.get_mut(&assets.points).unwrap() = point_asset(&generated.delaunay);
            *gizmo_assets.get_mut(&assets.delaunay).unwrap() = delaunay_asset(&generated.delaunay);
            *gizmo_assets.get_mut(&assets.voronoi).unwrap() = voronoi_asset(&generated.voronoi);
            *world = generated;
            settings.last_error = None;
        }
        Err(error) => settings.last_error = Some(error),
    }
}

fn sync_visible_layers(
    settings: Res<ViewerSettings>,
    mut camera_layers: Single<&mut RenderLayers, With<ViewerCamera>>,
) {
    let mut layers = vec![0];
    if settings.show_points {
        layers.push(POINT_LAYER);
    }
    if settings.show_delaunay {
        layers.push(DELAUNAY_LAYER);
    }
    if settings.show_voronoi {
        layers.push(VORONOI_LAYER);
    }
    **camera_layers = RenderLayers::from_layers(&layers);
}

fn point_asset(delaunay: &SphericalDelaunay) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let size = (0.018 / (delaunay.points.len() as f32).sqrt().max(8.0)).max(0.001);
    for (index, &point) in delaunay.points.iter().enumerate() {
        let point = to_bevy(point).normalize() * 1.012;
        let reference = if point.y.abs() < 0.9 {
            Vec3::Y
        } else {
            Vec3::X
        };
        let tangent = point.cross(reference).normalize() * size;
        let bitangent = point.cross(tangent).normalize() * size;
        let color = id_color(index);
        asset.line(point - tangent, point + tangent, color);
        asset.line(point - bitangent, point + bitangent, color);
    }
    asset
}

fn delaunay_asset(delaunay: &SphericalDelaunay) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let color = Color::srgba(0.35, 0.5, 0.72, 0.9);
    for edge in 0..delaunay.opposite_half_edges.len() {
        if delaunay.opposite_half_edges[edge] < edge {
            continue;
        }
        let triangle = edge / 3;
        let local = edge % 3;
        let vertices = delaunay.triangles[triangle];
        add_surface_edge(
            &mut asset,
            to_bevy(delaunay.points[vertices[local]]),
            to_bevy(delaunay.points[vertices[(local + 1) % 3]]),
            1.0,
            color,
        );
    }
    asset
}

fn voronoi_asset(mesh: &SphereMesh) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    for edge in &mesh.edges {
        add_surface_edge(
            &mut asset,
            to_bevy(mesh.vertices[edge.vertices[0]]),
            to_bevy(mesh.vertices[edge.vertices[1]]),
            1.006,
            id_color(edge.cells[0]),
        );
    }
    asset
}

fn add_surface_edge(asset: &mut GizmoAsset, start: Vec3, end: Vec3, radius: f32, color: Color) {
    let start = start.normalize();
    let end = end.normalize();
    let mut previous = start * radius;
    for segment in 1..=3 {
        let t = segment as f32 / 3.0;
        let current = start.lerp(end, t).normalize() * radius;
        asset.line(previous, current, color);
        previous = current;
    }
}

fn to_bevy(point: SphereVec3) -> Vec3 {
    Vec3::new(point.x, point.y, point.z)
}

fn id_color(id: usize) -> Color {
    let hue = (id as f32 * 137.508) % 360.0;
    Color::hsla(hue, 0.62, 0.62, 0.95)
}
