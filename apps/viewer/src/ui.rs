mod controls;
mod summary;

use crate::model::{GeneratedWorld, GenerationSettings, GenerationStatus, RegenerateWorld};
use crate::render::{DiagnosticLayer, LayerSettings};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use procgen_tectonics::FieldSummary;

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
                layer_controls(ui, &mut layers);

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

fn layer_controls(ui: &mut egui::Ui, layers: &mut LayerSettings) {
    ui.label("Layers");
    for &layer in DiagnosticLayer::ALL {
        // Only mutably access the resource when egui reports a real change.
        let mut visible = layers.is_visible(layer);
        if ui.checkbox(&mut visible, layer.label()).changed() {
            layers.set_visible(layer, visible);
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(SECTION_SPACING);
    ui.label(title);
    content(ui);
}

fn stat_grid(ui: &mut egui::Ui, title: &str, id: &str, content: impl FnOnce(&mut egui::Ui)) {
    section(ui, title, |ui| {
        egui::Grid::new(id).num_columns(2).show(ui, content);
    });
}

fn stat(ui: &mut egui::Ui, label: &str, value: impl std::fmt::Display) {
    ui.label(label);
    ui.monospace(value.to_string());
    ui.end_row();
}

fn field_summary_stats(ui: &mut egui::Ui, summary: &FieldSummary) {
    stat(ui, "Range", format_field_range(summary));
    stat(ui, "Mean", format!("{:.3}", summary.mean));
}

fn format_field_range(summary: &FieldSummary) -> String {
    format!("{:.3} - {:.3}", summary.minimum, summary.maximum)
}

fn millis(duration: std::time::Duration) -> String {
    format!("{:.2} ms", duration.as_secs_f64() * 1_000.0)
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

fn slider(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::Slider::new(value, range));
    });
}
