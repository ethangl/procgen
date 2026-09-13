use bevy_egui::egui;
use procgen_realtime_pilot::{
    DesignPreviewConfig, MAX_DESIGN_OCTAVES, NoiseConfig, OctaveConfig, PlanetDesignConfig,
    PreviewArea, PreviewBands,
};

pub fn distance(meters: f32) -> String {
    if meters.abs() >= 1000.0 {
        format!("{:.2} km", meters / 1000.0)
    } else {
        format!("{meters:.2} m")
    }
}

// Auto IDs include the parent's widget position. These sections follow changing
// status/validation labels, so give their child IDs an independent stable root.
pub fn preview(ui: &mut egui::Ui, config: &mut DesignPreviewConfig) {
    ui.scope_builder(
        egui::UiBuilder::new().id("planet-design-preview-controls"),
        |ui| {
            preview_controls(ui, config);
        },
    );
}

fn preview_controls(ui: &mut egui::Ui, config: &mut DesignPreviewConfig) {
    ui.horizontal(|ui| {
        if ui
            .selectable_label(matches!(config.area, PreviewArea::Planet), "Planet")
            .clicked()
        {
            config.area = PreviewArea::Planet;
        }
        if ui
            .selectable_label(
                matches!(config.area, PreviewArea::Patch { .. }),
                "Local patch",
            )
            .clicked()
        {
            config.area = PreviewArea::Patch {
                latitude_deg: 0.0,
                longitude_deg: 0.0,
                span_m: 16_384.0,
            };
        }
    });
    if let PreviewArea::Patch {
        latitude_deg,
        longitude_deg,
        span_m,
    } = &mut config.area
    {
        ui.add(egui::Slider::new(latitude_deg, -90.0..=90.0).text("Latitude"));
        ui.add(egui::Slider::new(longitude_deg, -180.0..=180.0).text("Longitude"));
        ui.add(
            egui::Slider::new(span_m, DesignPreviewConfig::PATCH_SPAN_RANGE)
                .logarithmic(true)
                .text("Span (m)"),
        );
    }
    egui::ComboBox::from_label("Preview quads")
        .selected_text(config.quads.to_string())
        .show_ui(ui, |ui| {
            for count in [32, 64, 128, 256] {
                ui.selectable_value(&mut config.quads, count, count.to_string());
            }
        });
    if ui
        .selectable_label(config.bands == PreviewBands::Combined, "Combined terrain")
        .clicked()
    {
        config.bands = PreviewBands::Combined;
    }
}

pub fn planet(ui: &mut egui::Ui, config: &mut PlanetDesignConfig) {
    ui.scope_builder(
        egui::UiBuilder::new().id("planet-design-planet-controls"),
        |ui| {
            planet_controls(ui, config);
        },
    );
}

fn planet_controls(ui: &mut egui::Ui, config: &mut PlanetDesignConfig) {
    let mut radius_km = config.radius_m / 1000.0;
    ui.horizontal(|ui| {
        ui.label("Radius (km)");
        if ui
            .add(egui::DragValue::new(&mut radius_km).speed(10.0).range(
                *PlanetDesignConfig::RADIUS_RANGE.start() / 1000.0
                    ..=*PlanetDesignConfig::RADIUS_RANGE.end() / 1000.0,
            ))
            .changed()
        {
            config.radius_m = radius_km * 1000.0;
        }
    });
    ui.add(
        egui::Slider::new(
            &mut config.height_limit_m,
            PlanetDesignConfig::HEIGHT_LIMIT_RANGE,
        )
        .logarithmic(true)
        .text("Height bound (m)"),
    );
    ui.label("Elevation zero is the reference sphere.");
}

pub fn octaves(
    ui: &mut egui::Ui,
    config: &mut PlanetDesignConfig,
    bands: &mut PreviewBands,
    weights: &[f32],
) {
    ui.heading("Octaves · broad to fine");
    ui.label("Solo evaluates a band alone, without earlier feedback. Values below are editable; Generate applies them.");
    for (i, o) in config.octaves.iter_mut().enumerate() {
        ui.scope_builder(
            egui::UiBuilder::new().id(("planet-design-octave", i)),
            |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut o.enabled, format!("{i}"));
                    if ui
                        .selectable_label(*bands == PreviewBands::Only(i), "Solo")
                        .clicked()
                    {
                        *bands = PreviewBands::Only(i);
                    }
                    ui.label(format!(
                        "{} · {} high",
                        distance(o.wavelength_m),
                        distance(o.amplitude_m)
                    ));
                });
                ui.small(match weights.get(i) {
                    Some(weight) => format!("Applied preview weight: {:.0}%", weight * 100.0),
                    None => "Preview weight not available yet".into(),
                });
                egui::CollapsingHeader::new("Edit band")
                    .id_salt(i)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Wavelength (m)");
                            ui.add(
                                egui::DragValue::new(&mut o.wavelength_m)
                                    .speed(1.0)
                                    .range(OctaveConfig::WAVELENGTH_RANGE),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Amplitude (m)");
                            ui.add(
                                egui::DragValue::new(&mut o.amplitude_m)
                                    .speed(0.1)
                                    .range(OctaveConfig::AMPLITUDE_RANGE),
                            );
                        });
                        ui.add(
                            egui::Slider::new(&mut o.sharpness, NoiseConfig::SHARPNESS_RANGE)
                                .text("Sharpness"),
                        );
                        for (label, value) in [
                            ("Warp", &mut o.perturbation),
                            ("Slope damping", &mut o.slope_erosion),
                            ("Altitude damping", &mut o.altitude_erosion),
                            ("Ridge damping", &mut o.ridge_erosion),
                        ] {
                            ui.add(
                                egui::Slider::new(value, NoiseConfig::SHAPING_RANGE).text(label),
                            );
                        }
                    });
                ui.separator();
            },
        );
    }
    let can_add = config.octaves.len() < MAX_DESIGN_OCTAVES
        && config
            .octaves
            .last()
            .is_some_and(|o| o.wavelength_m >= 2.0 * OctaveConfig::WAVELENGTH_RANGE.start());
    if ui
        .add_enabled(can_add, egui::Button::new("Add finer octave"))
        .clicked()
    {
        let mut o = config.octaves.last().unwrap().clone();
        o.wavelength_m *= 0.5;
        o.amplitude_m *= 0.5;
        config.octaves.push(o);
    }
    if ui
        .add_enabled(
            config.octaves.len() > 1,
            egui::Button::new("Remove finest octave"),
        )
        .clicked()
    {
        config.octaves.pop();
        if matches!(*bands, PreviewBands::Only(i) if i >= config.octaves.len()) {
            *bands = PreviewBands::Combined;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        config: &mut PlanetDesignConfig,
        notice: bool,
        events: Vec<egui::Event>,
    ) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                if notice {
                    ui.label("Generation / validation status changed");
                }
                planet(ui, config);
            });
        });
    }

    #[test]
    fn numeric_edit_keeps_focus_and_text_when_preceding_status_changes() {
        let ctx = egui::Context::default();
        let mut config = PlanetDesignConfig::starter(42);
        frame(&ctx, &mut config, false, vec![]);
        frame(
            &ctx,
            &mut config,
            false,
            vec![egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let focused = ctx
            .memory(|m| m.focused())
            .expect("Tab focuses the radius input");
        frame(&ctx, &mut config, false, vec![]);
        for (text, notice) in [("3", false), ("0", true), ("0", false), ("0", true)] {
            frame(
                &ctx,
                &mut config,
                notice,
                vec![egui::Event::Text(text.into())],
            );
            assert_eq!(ctx.memory(|m| m.focused()), Some(focused));
        }
        assert_eq!(config.radius_m, 3_000_000.0);
    }
}
