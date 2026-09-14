//! Controls and diagnostics for the physical exploration viewer.
use super::{Inspector, Navigation, PhysicalCamera};
use crate::physical_render::Coloring;
use bevy::{camera::Viewport, prelude::*, window::PrimaryWindow};
use bevy_egui::{EguiContexts, egui};
use procgen_realtime_pilot::{PLAYER_EYE_M, PLAYER_SPEED_MPS, VoxelCollision};

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
            ui.heading("Physical planet");
            ui.label(&state.path);
            ui.label(format!("Radius: {:.1} km", state.field.config().radius_m / 1000.0));
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Orbit").clicked() { state.orbit(); }
                if ui.button("Fly").clicked() {
                    state.mode = Navigation::Fly;
                    state.walker = None;
                    state.landing = false;
                    state.descent = false;
                }
                if ui.button("Go to ground").clicked() { state.near_ground(); }
            });
            ui.horizontal(|ui| {
                let label = if state.descent { "Stop descent" } else { "Descend continuously" };
                if ui.button(label).clicked() {
                    state.descent = !state.descent;
                    state.walker = None;
                    state.landing = false;
                    state.mode = Navigation::Orbit;
                    state.face_ground();
                }
                if ui.add_enabled(state.clearance_estimate() < 20.0, egui::Button::new("Walk")).clicked() {
                    state.descent = false;
                    state.landing = true;
                }
            });
            ui.label(match state.mode {
                Navigation::Orbit => "Orbit: drag left · scroll to descend",
                Navigation::Fly => "Fly: W A S D · E / Q up / down",
                Navigation::Walk => "Walk: W A S D",
            });
            ui.label("Hold right mouse to look · scroll adjusts flight speed");
            ui.label(format!("Flight speed factor: {:.2}×", state.speed_factor));
            ui.label(format!("Player: {PLAYER_EYE_M:.1} m eye · {PLAYER_SPEED_MPS:.1} m/s walking"));
            ui.separator();
            let altitude = state.eye.altitude_m(state.field.config().radius_m);
            ui.label(if altitude.abs() < 10_000.0 {
                format!("Reference altitude: {altitude:.2} m")
            } else {
                format!("Reference altitude: {:.3} km", altitude / 1000.0)
            });
            ui.label(match state.ground_clearance {
                Some(m) => format!("Ground clearance (collision): {m:.2} m"),
                None => "Ground clearance (collision): unavailable".into(),
            });
            ui.label(format!("Field clearance estimate: {:.2} m", state.clearance_estimate()));
            ui.label(if state.collision_busy {
                "Nearby collision: building"
            } else if state.collision.is_some() {
                "Nearby collision: retained"
            } else {
                "Nearby collision: outside ground range"
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.selectable_value(&mut state.coloring, Coloring::Neutral, "Neutral");
                ui.selectable_value(&mut state.coloring, Coloring::Lod, "LOD");
                ui.selectable_value(&mut state.coloring, Coloring::Normals, "Normals");
            });
            ui.label("LOD 0 (red) = 1 m. Each level doubles spacing.");
            ui.separator();
            if let Some(bridge) = &state.gpu {
                ui.label("Backend: GPU · direct resident buffers");
                if let Ok(output) = bridge.output.try_lock() {
                    let s = &output.stats;
                    ui.label(&s.status);
                    ui.label(format!("Chunks: {} resident / {} target · {} pending · {} retiring",s.resident,s.target,s.in_flight,s.retiring));
                    ui.label(format!("{} visible chunks · finest spacing {} m",s.drawn,s.finest_spacing_m.map(|s|s.to_string()).unwrap_or_else(|| "pending".into())));
                    ui.label(format!("GPU allocation: {:.1} MiB / {:.0} MiB",s.bytes as f64/1048576.0,crate::physical_gpu::GPU_WORLD_CONFIG.memory_budget_bytes as f64/1048576.0));
                    ui.label(format!("Resident {:.1} MiB · retiring {:.1} MiB",s.resident_bytes as f64/1048576.0,s.retiring_bytes as f64/1048576.0));
                    ui.label(format!("Worker selection: {:.2} ms",s.selection_ms));
                    ui.label(format!("Last job: {:.2} ms preparation · {:.2} ms encoding",s.preparation_ms,s.encoding_ms));
                    ui.label(format!("Submission to receipt: {:.2} ms",s.submission_latency_ms));
                    ui.label(format!("Job completion: {:.2} ms · local publication {:.2} ms",s.completion_ms,s.publication_ms));
                    ui.label(format!("Render scheduling: {:.3} ms · draw encoding {:.3} ms",s.scheduler_ms,s.draw_ms));
                    if let Some(times) = s.gpu_times {
                        ui.label(format!("GPU: {:.3} ms density · {:.3} ms extraction",times.density_ms,times.extraction_ms));
                    } else { ui.label("GPU execution timestamps: unavailable on this device"); }
                }
                ui.label(format!("Frame: {:.1} ms · peak {:.1} ms",state.frame_ms,state.peak_frame_ms));
                ui.label(format!("Collision: {:.2} s · walk update {:.3} ms",state.collision_seconds,state.query_ms));
                ui.label(&state.status);
            } else {
                ui.label("Backend: CPU audit");
            ui.label(&state.status);
            ui.label(format!("{} displayed triangles", state.triangles));
            if let Some(staging) = &state.staging {
                ui.label(format!("{} upload pieces remaining", staging.result.packed.pieces.len()));
            }
            ui.label(format!("Generation: {:.2} s · collision: {:.2} s", state.generation_seconds, state.collision_seconds));
            ui.label(format!("Frame: {:.1} ms · peak {:.1} ms", state.frame_ms, state.peak_frame_ms));
            ui.label(format!("Upload: {:.0} KiB · CPU {:.2} ms · peak {:.2} ms", state.upload_bytes as f32 / 1024.0, state.upload_ms, state.peak_upload_ms));
            ui.label(format!("Walk update: {:.3} ms", state.query_ms));
            for (label, bytes) in [
                ("Build source", state.source_bytes),
                ("Build mesh", state.mesh_bytes),
                ("Displayed buffers", state.display_bytes),
                ("Collision", state.collision.as_ref().map_or(0, VoxelCollision::payload_bytes)),
            ] {
                ui.label(format!("{label}: {:.1} MiB", bytes as f32 / 1048576.0));
            }
            ui.label("Payload counters exclude allocator and driver overhead. One replacement at a time; 512 KiB upload per frame.");
            }
            if ui.button("Reset timing peaks").clicked() {
                state.peak_frame_ms = 0.0;
                state.peak_upload_ms = 0.0;
            }
        });
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
