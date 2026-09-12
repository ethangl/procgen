mod controls;
mod summary;

use crate::model::{
    ClearWorldCache, GenerateRequest, GeneratedWorld, GenerationSettings, GenerationStatus, Phase,
};
use crate::render::{
    DiagnosticLayer, LightingSettings, OverlayKind, OverlaySettings, ReliefSettings,
    SurfaceSelection,
};
use bevy::{ecs::system::SystemParam, prelude::*};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use procgen_planet::Planet;
use procgen_sphere_mesh::default_hop_length;

const SECTION_SPACING: f32 = 6.0;
const SIDEBAR_WIDTH: f32 = 250.0;

/// What the sidebar shows: one generation phase, or the display controls.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Resource)]
enum SidebarTab {
    Phase(Phase),
    Render,
}

impl Default for SidebarTab {
    fn default() -> Self {
        Self::Phase(Phase::Tectonics)
    }
}

#[derive(SystemParam)]
struct ViewerSettings<'w> {
    generation: ResMut<'w, GenerationSettings>,
    surface: ResMut<'w, SurfaceSelection>,
    overlays: ResMut<'w, OverlaySettings>,
    relief: ResMut<'w, ReliefSettings>,
    lighting: ResMut<'w, LightingSettings>,
}

#[derive(SystemParam)]
struct ViewerRequests<'w> {
    generate: MessageWriter<'w, GenerateRequest>,
    clear_cache: MessageWriter<'w, ClearWorldCache>,
}

pub struct ViewerUiPlugin;

impl Plugin for ViewerUiPlugin {
    fn build(&self, app: &mut App) {
        debug_assert!(app.is_plugin_added::<EguiPlugin>());
        app.init_resource::<SidebarTab>()
            .add_systems(EguiPrimaryContextPass, viewer_ui);
    }
}

fn viewer_ui(
    mut contexts: EguiContexts,
    mut tab: ResMut<SidebarTab>,
    mut settings: ViewerSettings,
    mut requests: ViewerRequests,
    status: Res<GenerationStatus>,
    world: Res<GeneratedWorld>,
) -> Result {
    let context = contexts.ctx_mut()?;
    egui::TopBottomPanel::top("navbar").show(context, |ui| {
        navbar(ui, tab.reborrow(), &world, &mut requests.generate);
    });
    egui::SidePanel::left("sidebar")
        .default_width(SIDEBAR_WIDTH)
        .resizable(false)
        .show(context, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                match *tab {
                    SidebarTab::Phase(phase) => {
                        phase_panel(ui, phase, &mut settings, &mut requests.generate, &world)
                    }
                    SidebarTab::Render => render_panel(ui, &mut settings, &world),
                }
                ui.separator();
                footer(ui, &status, &mut requests.clear_cache);
            });
        });
    Ok(())
}

/// Phases on the left select what the sidebar shows; the render tab and the
/// whole-world generate button sit on the right.
fn navbar(
    ui: &mut egui::Ui,
    mut tab: Mut<SidebarTab>,
    world: &GeneratedWorld,
    generate: &mut MessageWriter<GenerateRequest>,
) {
    ui.horizontal(|ui| {
        for &phase in Phase::ALL {
            let label = egui::RichText::new(phase.label());
            // Phases without results read as unavailable until they are run.
            let label = if world.holds(phase) {
                label
            } else {
                label.weak()
            };
            let selected = *tab == SidebarTab::Phase(phase);
            if ui.selectable_label(selected, label).clicked() {
                *tab = SidebarTab::Phase(phase);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Generate").clicked() {
                generate.write(GenerateRequest::AllPhases);
            }
            if ui
                .selectable_label(*tab == SidebarTab::Render, "Render")
                .clicked()
            {
                *tab = SidebarTab::Render;
            }
        });
    });
}

fn phase_panel(
    ui: &mut egui::Ui,
    phase: Phase,
    settings: &mut ViewerSettings,
    generate: &mut MessageWriter<GenerateRequest>,
    world: &GeneratedWorld,
) {
    ui.heading(phase.label());
    ui.add_space(SECTION_SPACING);
    if ui
        .button(format!("Generate {}", phase.label().to_lowercase()))
        .clicked()
    {
        generate.write(GenerateRequest::Phase(phase));
    }
    ui.separator();
    controls::phase_controls(ui, phase, &mut settings.generation);
    ui.separator();
    summary::phase_summary(ui, phase, world);
}

fn render_panel(ui: &mut egui::Ui, settings: &mut ViewerSettings, world: &GeneratedWorld) {
    ui.heading("Render");
    ui.add_space(SECTION_SPACING);
    let mut next_relief = *settings.relief;
    let mut next_lighting = *settings.lighting;
    render_controls(ui, &mut next_relief, &mut next_lighting);
    settings.relief.set_if_neq(next_relief);
    settings.lighting.set_if_neq(next_lighting);

    ui.separator();
    layer_controls(
        ui,
        world,
        settings.surface.reborrow(),
        settings.overlays.reborrow(),
    );
}

fn footer(
    ui: &mut egui::Ui,
    status: &GenerationStatus,
    clear_cache: &mut MessageWriter<ClearWorldCache>,
) {
    generation_status(ui, status);
    if ui.small_button("Clear cache").clicked() {
        clear_cache.write_default();
    }
}

fn generation_status(ui: &mut egui::Ui, status: &GenerationStatus) {
    match status {
        GenerationStatus::Empty { notice } => {
            ui.label("No world generated yet.");
            if let Some(notice) = notice {
                ui.small(notice);
            }
        }
        GenerationStatus::StartupLoaded { duration } => {
            ui.label(format!(
                "Loaded cached world in {:.2} ms",
                millis(*duration)
            ));
        }
        GenerationStatus::Generated {
            phases,
            cache_notices,
        } => {
            ui.label(generated_label(phases));
            for notice in cache_notices {
                ui.small(notice);
            }
        }
        GenerationStatus::GenerationFailed { error }
        | GenerationStatus::CacheClearFailed { error } => {
            ui.colored_label(egui::Color32::from_rgb(255, 110, 110), error);
        }
        GenerationStatus::CacheCleared { existed: true } => {
            ui.small("World cache cleared.");
        }
        GenerationStatus::CacheCleared { existed: false } => {
            ui.small("World cache is already empty.");
        }
    }
}

fn generated_label(phases: &[Phase]) -> String {
    let names: Vec<_> = phases
        .iter()
        .map(|phase| phase.label().to_lowercase())
        .collect();
    format!("Generated {}.", names.join(", "))
}

fn millis(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn render_controls(
    ui: &mut egui::Ui,
    relief: &mut ReliefSettings,
    lighting: &mut LightingSettings,
) {
    section(ui, "Terrain relief", |ui| {
        slider(ui, "Exaggeration", &mut relief.exaggeration, 0.0..=0.4)
    });
    section(ui, "Directional light", |ui| {
        slider(ui, "Azimuth", &mut lighting.azimuth_degrees, -180.0..=180.0);
        slider(
            ui,
            "Elevation",
            &mut lighting.elevation_degrees,
            -89.0..=89.0,
        );
        drag_value(
            ui,
            "Illuminance",
            &mut lighting.illuminance,
            0.0..=30_000.0,
            500.0,
        );
        drag_value(
            ui,
            "Ambient fill",
            &mut lighting.ambient_brightness,
            0.0..=2_000.0,
            50.0,
        );
    });
}

/// Only layers whose phase has results are offered.
fn layer_controls(
    ui: &mut egui::Ui,
    world: &GeneratedWorld,
    mut surface: Mut<SurfaceSelection>,
    mut overlays: Mut<OverlaySettings>,
) {
    let available = |layer: DiagnosticLayer| world.holds(layer.phase());

    ui.label("Surface fill");
    let selected_surface = surface.selected();
    if ui.radio(selected_surface.is_none(), "None").clicked() && selected_surface.is_some() {
        surface.set(None);
    }
    for &layer in DiagnosticLayer::ALL {
        if layer.is_fill()
            && available(layer)
            && ui
                .radio(selected_surface == Some(layer), layer.label())
                .clicked()
            && selected_surface != Some(layer)
        {
            surface.set(Some(layer));
        }
    }

    for &kind in OverlayKind::ALL {
        let layers: Vec<_> = DiagnosticLayer::ALL
            .iter()
            .copied()
            .filter(|&layer| layer.overlay_kind() == Some(kind) && available(layer))
            .collect();
        if layers.is_empty() {
            continue;
        }
        ui.add_space(4.0);
        ui.label(kind.label());
        for layer in layers {
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

/// A model length on the unit sphere, with the kilometres it spans on an
/// Earth-sized planet in its tooltip. The length itself is a fraction of a
/// radius and reads as noise, so the tooltip is what makes the number mean
/// something; the planet radius the climate phase carries belongs to another
/// phase, so the reference here is Earth's, as the crates' own documentation
/// states these defaults in.
fn length_slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        let kilometres = f64::from(*value) * Planet::EARTH.radius_meters / 1_000.0;
        ui.add(egui::Slider::new(value, range))
            .on_hover_text(format!("{kilometres:.0} km at Earth radius"));
    });
}

/// A slider range stated in hops of the default mesh, as the model lengths
/// those hops span. Every length default was set against that mesh, so its
/// hops are the units these ranges were chosen in.
fn hop_range(hops: std::ops::RangeInclusive<f32>) -> std::ops::RangeInclusive<f32> {
    hops.start() * default_hop_length()..=hops.end() * default_hop_length()
}
