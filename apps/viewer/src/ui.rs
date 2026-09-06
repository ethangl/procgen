mod controls;
mod summary;

use crate::model::{GeneratedWorld, GenerationSettings, GenerationStatus, RegenerateWorld};
use crate::render::{DiagnosticLayer, OverlayKind, OverlaySettings, SurfaceSelection};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

const SECTION_SPACING: f32 = 6.0;

pub struct ViewerUiPlugin;

impl Plugin for ViewerUiPlugin {
    fn build(&self, app: &mut App) {
        debug_assert!(app.is_plugin_added::<EguiPlugin>());
        app.add_systems(EguiPrimaryContextPass, viewer_ui);
    }
}

fn viewer_ui(
    mut contexts: EguiContexts,
    mut generation: ResMut<GenerationSettings>,
    mut surface: ResMut<SurfaceSelection>,
    mut overlays: ResMut<OverlaySettings>,
    status: Res<GenerationStatus>,
    world: Res<GeneratedWorld>,
    mut regenerate: MessageWriter<RegenerateWorld>,
) -> Result {
    egui::SidePanel::left("controls")
        .default_width(250.0)
        .resizable(false)
        .show(contexts.ctx_mut()?, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Sphere topology");
                ui.add_space(6.0);
                controls::generation_controls(ui, &mut generation, &mut regenerate);

                if let Some(error) = &status.last_error {
                    ui.colored_label(egui::Color32::from_rgb(255, 110, 110), error);
                }

                ui.separator();
                layer_controls(ui, surface.reborrow(), overlays.reborrow());

                ui.separator();
                summary::world_summary(ui, &world);

                ui.separator();
                ui.label("Drag the viewport to orbit.");
                ui.label("Scroll to zoom.");
                ui.label("Axes: X red, Y green, Z blue.");
            });
        });
    Ok(())
}

fn layer_controls(
    ui: &mut egui::Ui,
    mut surface: Mut<SurfaceSelection>,
    mut overlays: Mut<OverlaySettings>,
) {
    ui.label("Surface fill");
    let selected_surface = surface.selected();
    if ui.radio(selected_surface.is_none(), "None").clicked() && selected_surface.is_some() {
        surface.set(None);
    }
    for &layer in DiagnosticLayer::ALL {
        if layer.is_fill()
            && ui
                .radio(selected_surface == Some(layer), layer.label())
                .clicked()
            && selected_surface != Some(layer)
        {
            surface.set(Some(layer));
        }
    }

    for &kind in OverlayKind::ALL {
        ui.add_space(4.0);
        ui.label(kind.label());
        for &layer in DiagnosticLayer::ALL {
            if layer.overlay_kind() != Some(kind) {
                continue;
            }
            let mut visible = overlays.is_visible(layer);
            if ui.checkbox(&mut visible, layer.label()).changed() {
                overlays.set_visible(layer, visible);
            }
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(SECTION_SPACING);
    ui.label(title);
    content(ui);
}

fn drag_value<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    speed: f64,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(value).range(range).speed(speed));
    });
}

fn slider<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::Slider::new(value, range));
    });
}
