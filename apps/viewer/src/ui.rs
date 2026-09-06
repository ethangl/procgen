mod controls;
mod summary;

use crate::model::{GeneratedWorld, GenerationSettings, GenerationStatus, RegenerateWorld};
use crate::render::{DiagnosticLayer, LayerKind, LayerSettings};
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
    mut layers: ResMut<LayerSettings>,
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
                layer_controls(ui, layers.reborrow());

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

fn layer_controls(ui: &mut egui::Ui, mut layers: Mut<LayerSettings>) {
    ui.label("Surface fill");
    let selected_surface = layers.surface_layer();
    if ui.radio(selected_surface.is_none(), "None").clicked() && selected_surface.is_some() {
        layers.set_surface_layer(None);
    }
    for &layer in DiagnosticLayer::ALL {
        if layer.kind() == LayerKind::Fill
            && ui
                .radio(selected_surface == Some(layer), layer.label())
                .clicked()
            && selected_surface != Some(layer)
        {
            layers.set_surface_layer(Some(layer));
        }
    }

    for (kind, heading) in [
        (LayerKind::Edges, "Edges"),
        (LayerKind::Markers, "Markers"),
        (LayerKind::Vectors, "Vectors"),
    ] {
        ui.add_space(4.0);
        ui.label(heading);
        for &layer in DiagnosticLayer::ALL {
            if layer.kind() != kind {
                continue;
            }
            let mut visible = layers.is_visible(layer);
            if ui.checkbox(&mut visible, layer.label()).changed() {
                layers.set_visible(layer, visible);
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
