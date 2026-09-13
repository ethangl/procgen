//! One forward PBR shader variant for all surface inspection modes.
use crate::surface_view::{NormalMode, SurfaceOverlay, SurfaceViewConfig};
use bevy::{
    asset::embedded_asset,
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};
use bevy_egui::egui;

pub type SurfaceMaterial = ExtendedMaterial<StandardMaterial, SurfaceExtension>;
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct SurfaceExtension {
    #[uniform(100)]
    modes: UVec4,
}
impl MaterialExtension for SurfaceExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://procgen_realtime_pilot/surface.wgsl".into()
    }
}
impl From<SurfaceViewConfig> for SurfaceExtension {
    fn from(v: SurfaceViewConfig) -> Self {
        Self {
            modes: UVec4::new(
                v.normals as u32,
                v.overlay as u32,
                u32::from(v.wireframe),
                0,
            ),
        }
    }
}
pub struct SurfacePlugin;
impl Plugin for SurfacePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "surface.wgsl");
        app.add_plugins(MaterialPlugin::<SurfaceMaterial>::default());
    }
}
pub fn material(view: SurfaceViewConfig) -> SurfaceMaterial {
    SurfaceMaterial {
        base: StandardMaterial {
            base_color: Color::srgb(0.55, 0.55, 0.55),
            perceptual_roughness: 0.9,
            ..default()
        },
        extension: view.into(),
    }
}
pub fn controls(ui: &mut egui::Ui, view: &mut SurfaceViewConfig, recording: bool) {
    ui.label("Surface inspection · fixed lighting");
    ui.add_enabled_ui(!recording, |ui| {
        egui::ComboBox::from_label("Normals")
            .selected_text(format!("{:?}", view.normals))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut view.normals, NormalMode::Averaged, "Averaged mesh");
                ui.selectable_value(&mut view.normals, NormalMode::Triangle, "Triangle faces");
                ui.selectable_value(&mut view.normals, NormalMode::Density, "Final density");
            });
        egui::ComboBox::from_label("Overlay")
            .selected_text(format!("{:?}", view.overlay))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut view.overlay, SurfaceOverlay::Neutral, "Neutral");
                ui.selectable_value(&mut view.overlay, SurfaceOverlay::Lod, "LOD colors");
                ui.selectable_value(
                    &mut view.overlay,
                    SurfaceOverlay::Normals,
                    "Normal directions",
                );
                ui.selectable_value(
                    &mut view.overlay,
                    SurfaceOverlay::Agreement,
                    "Mesh / density agreement",
                );
            });
        ui.checkbox(&mut view.wireframe, "Triangle edges");
    });
    match view.overlay {
        SurfaceOverlay::Lod => {
            ui.label("Blue: coarse · Green: medium · Gold: fine");
        }
        SurfaceOverlay::Normals => {
            ui.label("RGB = (planet-space normal + 1) / 2");
        }
        SurfaceOverlay::Agreement => {
            ui.label(
                "Averaged mesh vs density: dark = aligned; yellow = perpendicular; red = opposed",
            );
        }
        SurfaceOverlay::Neutral => {}
    }
    ui.label("Magenta = undefined normal. Density normals are sampled at mesh vertices.");
    if recording {
        ui.label("View settings are fixed during recording.");
    }
}
