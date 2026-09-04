use crate::model::{GeneratedWorld, GenerationSettings, GenerationStatus, RegenerateWorld};
use crate::render::{LayerSettings, TopologyLayer};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

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
            ui.heading("Sphere topology");
            ui.add_space(6.0);

            ui.label("Generation");
            ui.horizontal(|ui| {
                ui.label("Cells");
                ui.add(
                    egui::DragValue::new(&mut generation.fibonacci.count)
                        .range(4..=65_536)
                        .speed(16),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Jitter");
                ui.add(egui::Slider::new(
                    &mut generation.fibonacci.jitter,
                    0.0..=1.0,
                ));
            });
            ui.horizontal(|ui| {
                ui.label("Seed");
                ui.add(egui::DragValue::new(&mut generation.fibonacci.seed));
            });
            if ui.button("Regenerate").clicked() {
                regenerate.write_default();
            }

            if let Some(error) = &status.last_error {
                ui.colored_label(egui::Color32::from_rgb(255, 110, 110), error);
            }

            ui.separator();
            ui.label("Layers");
            for layer in TopologyLayer::ALL {
                let mut visible = layers.is_visible(layer);
                if ui.checkbox(&mut visible, layer.label()).changed() {
                    layers.set_visible(layer, visible);
                }
            }

            ui.separator();
            ui.label("Active world");
            egui::Grid::new("stats").num_columns(2).show(ui, |ui| {
                stat(ui, "Cells", world.voronoi.cell_count());
                stat(ui, "Vertices", world.voronoi.vertex_count());
                stat(ui, "Edges", world.voronoi.edge_count());
                stat(ui, "Seed", world.config.seed);
                stat(ui, "Jitter", format!("{:.2}", world.config.jitter));
            });

            ui.add_space(6.0);
            ui.label("Timings");
            timing(ui, "Sampling", world.timings.sampling.as_secs_f64());
            timing(ui, "Delaunay", world.timings.delaunay.as_secs_f64());
            timing(ui, "Voronoi", world.timings.voronoi.as_secs_f64());
            timing(ui, "Total", world.timings.total().as_secs_f64());

            ui.separator();
            ui.label("Drag the viewport to orbit.");
            ui.label("Scroll to zoom.");
            ui.label("Axes: X red, Y green, Z blue.");
        });
    Ok(())
}

fn stat(ui: &mut egui::Ui, label: &str, value: impl std::fmt::Display) {
    ui.label(label);
    ui.monospace(value.to_string());
    ui.end_row();
}

fn timing(ui: &mut egui::Ui, label: &str, seconds: f64) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.monospace(format!("{:.2} ms", seconds * 1_000.0));
    });
}
