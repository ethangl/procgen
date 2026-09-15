//! Controls and diagnostics for the physical exploration viewer.
use super::{Inspector, Navigation, PhysicalCamera};
use crate::physical_render::Coloring;
use bevy::{camera::Viewport, prelude::*, window::PrimaryWindow};
use bevy_egui::{EguiContexts, egui};
use procgen_realtime_pilot::CAMERA_CLEARANCE_M;

pub(super) fn panel(
    mut contexts: EguiContexts,
    mut state: NonSendMut<Inspector>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cameras: Query<&mut Camera, With<PhysicalCamera>>,
) -> Result {
    // The UI schedule can run once after the primary window is closed.
    let Ok(window) = windows.single() else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    let panel = egui::SidePanel::left("physical-exploration")
        .exact_width(340.0)
        .show(ctx, |ui| {
            if state.record.is_some() {
                ui.label("Recording a fixed route; live controls are disabled.");
                ui.disable();
            }
            ui.heading("Physical planet");
            ui.label("Orbit / descent · live terrain controls");
            state.editor.tabs(ui);
            ui.label(format!(
                "Radius: {:.1} km",
                state.field.config().radius_m / 1000.0
            ));
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Orbit").clicked() {
                    state.orbit();
                }
                if ui.button("Fly").clicked() {
                    state.mode = Navigation::Fly;
                    state.descent = false;
                }
                if ui.button("Go to ground").clicked() {
                    state.near_ground();
                }
            });
            ui.horizontal(|ui| {
                let label = if state.descent {
                    "Stop descent"
                } else {
                    "Descend continuously"
                };
                if ui.button(label).clicked() {
                    state.descent = !state.descent;
                    state.mode = Navigation::Orbit;
                    state.face_ground();
                }
            });
            ui.label(match state.mode {
                Navigation::Orbit => "Orbit: drag left · scroll to descend",
                Navigation::Fly => "Fly: W A S D · E / Q up / down",
            });
            ui.label("Hold right mouse to look · scroll adjusts flight speed");
            ui.label(format!(
                "Flight speed: {:.1} m/s ({:.2}×)",
                super::navigation::flight_speed_mps(state.clearance_estimate(), state.speed_factor),
                state.speed_factor
            ));
            ui.checkbox(
                &mut state.keep_above_terrain,
                format!("Keep camera {CAMERA_CLEARANCE_M:.0} m above terrain"),
            );
            ui.separator();
            if state.editor.tab != crate::design_panel::Tab::Status {
                let revision = state.revision;
                let failure = state.gpu.designs.lock().unwrap().failure.clone();
                state.editor.controls(ui, revision, failure.as_deref());
                return;
            }
            let altitude = state.eye.altitude_m(state.field.config().radius_m);
            ui.label(if altitude.abs() < 10_000.0 {
                format!("Reference altitude: {altitude:.2} m")
            } else {
                format!("Reference altitude: {:.3} km", altitude / 1000.0)
            });
            ui.label(format!(
                "Radial terrain clearance: {:.2} m",
                state.clearance_estimate()
            ));
            if state.editor.ocean.enabled {
                ui.label(format!(
                    "Ocean: sea level {:.1} m · camera {}",
                    state.editor.ocean.sea_level_m,
                    if altitude < state.editor.ocean.sea_level_m as f64 {
                        "underwater"
                    } else {
                        "above water"
                    }
                ));
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.selectable_value(&mut state.coloring, Coloring::Height, "Height");
                ui.selectable_value(&mut state.coloring, Coloring::Neutral, "Neutral");
                ui.selectable_value(&mut state.coloring, Coloring::Lod, "LOD");
                ui.selectable_value(&mut state.coloring, Coloring::Normals, "Normals");
            });
            match state.coloring {
                Coloring::Height => height_legend(ui, state.field.config().height_limit_m),
                Coloring::Lod => {
                    ui.label("Colors show sample spacing: red = 1 m or finer.");
                }
                _ => {}
            }
            ui.separator();
            let displayed = state.gpu.display.lock().unwrap().clone();
            if let Ok(output) = displayed.output.try_lock() {
                let s = &output.stats;
                ui.label(&s.status);
                ui.label(format!(
                    "Height tiles: {} · {:.1} MiB · update {:.1} ms",
                    s.height_tiles,
                    s.height_bytes as f64 / 1048576.0,
                    s.height_update_ms
                ));
                ui.label(format!(
                    "Drawn tiles: {} / {} across active snapshots",
                    s.height_drawn_tiles, s.height_tested_tiles
                ));
                ui.label(format!(
                    "Tile work: {} generated · {} reused",
                    s.height_generated_tiles, s.height_reused_tiles
                ));
                ui.label(format!(
                    "Height build {:.1} ms · blend wait {:.1} ms",
                    s.height_build_ms, s.height_wait_ms
                ));
                ui.label(format!(
                    "Surface blend targets: {:.1} MiB",
                    s.surface_target_bytes as f64 / 1048576.0
                ));
                ui.label(format!(
                    "Height tiles capped at {}",
                    procgen_realtime_pilot::MAX_HEIGHT_TILES
                ));
                ui.label(format!("Worker selection: {:.2} ms", s.selection_ms));
                ui.label(format!(
                    "Render scheduling: {:.3} ms · draw encoding {:.3} ms",
                    s.scheduler_ms, s.draw_ms
                ));
            }
            ui.label(format!(
                "Frame: {:.1} ms · peak {:.1} ms",
                state.frame_ms, state.peak_frame_ms
            ));
            ui.label(&state.status);
            if ui.button("Reset timing peaks").clicked() {
                state.peak_frame_ms = 0.0;
            }
        });
    if state.editor.edits.observe(std::time::Instant::now()) {
        let revision = state.editor.edits.revision();
        state.gpu.designs.lock().unwrap().invalidate(revision);
    }
    let left = (panel.response.rect.width() * ctx.pixels_per_point()).round() as u32;
    let size = UVec2::new(
        window.physical_width().saturating_sub(left),
        window.physical_height(),
    );
    for mut camera in &mut cameras {
        camera.is_active = size.min_element() > 0;
        if camera.is_active {
            camera.viewport = Some(Viewport {
                physical_position: UVec2::new(left, 0),
                physical_size: size,
                ..default()
            });
        }
    }
    Ok(())
}

fn height_legend(ui: &mut egui::Ui, limit_m: f32) {
    use crate::physical_color::HEIGHT_COLORS;
    ui.label("Altitude above reference radius");
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 12.0), egui::Sense::hover());
    let mut mesh = egui::Mesh::default();
    for stop in HEIGHT_COLORS {
        let [r, g, b] = stop.rgb;
        let height = stop.relative_height;
        let x = egui::lerp(rect.x_range(), (height + 1.0) * 0.5);
        let color = egui::Rgba::from_rgb(r, g, b).into();
        mesh.colored_vertex(egui::pos2(x, rect.top()), color);
        mesh.colored_vertex(egui::pos2(x, rect.bottom()), color);
    }
    for i in 0..HEIGHT_COLORS.len() as u32 - 1 {
        let a = i * 2;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 2, a + 1, a + 3);
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    let (labels, _) = ui.allocate_exact_size(
        egui::vec2(
            ui.available_width(),
            ui.text_style_height(&egui::TextStyle::Body),
        ),
        egui::Sense::hover(),
    );
    for (x, align, text) in [
        (
            labels.left(),
            egui::Align2::LEFT_CENTER,
            format!("−{:.1} km", limit_m / 1000.0),
        ),
        (
            labels.center().x,
            egui::Align2::CENTER_CENTER,
            "0 km".into(),
        ),
        (
            labels.right(),
            egui::Align2::RIGHT_CENTER,
            format!("+{:.1} km", limit_m / 1000.0),
        ),
    ] {
        ui.painter().text(
            egui::pos2(x, labels.center().y),
            align,
            text,
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().text_color(),
        );
    }
    ui.label("Terrain colors show elevation only.");
}
