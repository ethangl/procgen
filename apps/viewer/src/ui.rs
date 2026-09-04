use crate::model::{GeneratedWorld, GenerationSettings, GenerationStatus, RegenerateWorld};
use crate::render::{DiagnosticLayer, LayerSettings};
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
            drag_value(
                ui,
                "Cells",
                &mut generation.fibonacci.count,
                4..=65_536,
                16.0,
            );
            ui.horizontal(|ui| {
                ui.label("Jitter");
                ui.add(egui::Slider::new(
                    &mut generation.fibonacci.jitter,
                    0.0..=1.0,
                ));
            });
            drag_value(
                ui,
                "Seed",
                &mut generation.fibonacci.seed,
                u64::MIN..=u64::MAX,
                1.0,
            );
            ui.add_space(4.0);
            ui.label("Tectonic plates");
            drag_value(
                ui,
                "Major",
                &mut generation.plates.major_plate_count,
                1..=128,
                1.0,
            );
            drag_value(
                ui,
                "Minor",
                &mut generation.plates.minor_plate_count,
                0..=256,
                1.0,
            );
            drag_value(
                ui,
                "Major head start",
                &mut generation.plates.major_head_start_rounds,
                0..=64,
                1.0,
            );
            drag_value(
                ui,
                "Seed",
                &mut generation.plates.seed,
                u64::MIN..=u64::MAX,
                1.0,
            );
            if ui.button("Regenerate").clicked() {
                regenerate.write_default();
            }

            if let Some(error) = &status.last_error {
                ui.colored_label(egui::Color32::from_rgb(255, 110, 110), error);
            }

            ui.separator();
            ui.label("Layers");
            for layer in DiagnosticLayer::ALL {
                // Only mutably access the resource when egui reports a real change.
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
                stat(ui, "Plates", world.plates.plate_count());
                stat(ui, "Seed", world.config.fibonacci.seed);
                stat(
                    ui,
                    "Jitter",
                    format!("{:.2}", world.config.fibonacci.jitter),
                );
            });

            ui.add_space(6.0);
            ui.label("Timings");
            egui::Grid::new("timings").num_columns(2).show(ui, |ui| {
                for stage in world.timings.stages() {
                    stat(
                        ui,
                        stage.label,
                        format!("{:.2} ms", stage.duration.as_secs_f64() * 1_000.0),
                    );
                }
                stat(
                    ui,
                    "Total",
                    format!("{:.2} ms", world.timings.total().as_secs_f64() * 1_000.0),
                );
            });

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
