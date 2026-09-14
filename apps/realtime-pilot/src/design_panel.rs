//! Design file actions and editable controls. No terrain generation runs here.
use crate::{design_controls, design_edits::DesignEdits, design_file};
use bevy_egui::egui;
use procgen_realtime_pilot::PlanetDesignConfig;
use std::path::{Path, PathBuf};

#[derive(PartialEq, Eq)]
pub enum Tab {
    Status,
    Design,
    Octaves,
}
pub struct DesignPanel {
    pub edits: DesignEdits,
    pub path: String,
    pub tab: Tab,
    notice: String,
}
impl DesignPanel {
    pub fn new(config: PlanetDesignConfig, path: Option<PathBuf>) -> Self {
        Self {
            edits: DesignEdits::new(config),
            path: path
                .unwrap_or_else(|| PathBuf::from("planet-design.json"))
                .display()
                .to_string(),
            tab: Tab::Status,
            notice: String::new(),
        }
    }
    pub fn tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Status, "Status");
            ui.selectable_value(&mut self.tab, Tab::Design, "Design");
            ui.selectable_value(&mut self.tab, Tab::Octaves, "Octaves");
        });
    }
    pub fn controls(&mut self, ui: &mut egui::Ui, displayed_revision: u64, failure: Option<&str>) {
        // Fixed-height notices and explicit roots keep text entry and layout stable.
        let message = failure
            .or(self.edits.validation.as_deref())
            .unwrap_or_else(|| {
                if displayed_revision == self.edits.revision() {
                    "Terrain is current. Edits are not saved automatically."
                } else {
                    "Updating terrain after valid edits…"
                }
            });
        notice(ui, message);
        egui::ScrollArea::vertical()
            .id_salt("live-design-scroll")
            .show(ui, |ui| match self.tab {
                Tab::Octaves => design_controls::octaves(ui, &mut self.edits.config),
                Tab::Design => {
                    ui.label("Design file");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.path)
                            .id(egui::Id::new("planet-design-path")),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Load").clicked() {
                            self.notice = match design_file::load(Path::new(&self.path)) {
                                Ok(config) => {
                                    self.edits.load(config);
                                    "Loaded controls.".into()
                                }
                                Err(e) => e.to_string(),
                            };
                        }
                        if ui.button("Save controls").clicked() {
                            self.notice = match self.edits.validated() {
                                Ok(field) => {
                                    match design_file::save(Path::new(&self.path), field.config()) {
                                        Ok(()) => "Saved complete design.".into(),
                                        Err(e) => e.to_string(),
                                    }
                                }
                                Err(e) => e,
                            };
                        }
                        if ui.button("Copy JSON").clicked() {
                            self.notice = match self.edits.validated() {
                                Ok(field) => match design_file::encode(field.config()) {
                                    Ok(json) => {
                                        ui.ctx().copy_text(json);
                                        "Copied complete design.".into()
                                    }
                                    Err(e) => e.to_string(),
                                },
                                Err(e) => e,
                            };
                        }
                    });
                    notice(ui, &self.notice);
                    ui.label("Seed");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.edits.seed_text)
                            .id(egui::Id::new("planet-design-seed")),
                    );
                    design_controls::planet(ui, &mut self.edits.config);
                }
                Tab::Status => unreachable!("status panel is rendered by the inspector"),
            });
    }
}

fn notice(ui: &mut egui::Ui, text: &str) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 52.0), egui::Sense::hover());
    ui.put(rect, egui::Label::new(text).wrap());
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(ctx: &egui::Context, panel: &mut DesignPanel, events: Vec<egui::Event>) -> egui::Rect {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(340.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| panel.controls(ui, 0, None));
            },
        );
        ctx.read_response(egui::Id::new("planet-design-seed"))
            .unwrap()
            .rect
    }
    #[test]
    fn typing_survives_validation_generation_and_file_notices_without_layout_shift() {
        let ctx = egui::Context::default();
        let mut panel = DesignPanel::new(PlanetDesignConfig::starter(42), None);
        panel.tab = Tab::Design;
        panel.edits.seed_text.clear();
        let rect = frame(&ctx, &mut panel, vec![]);
        let id = egui::Id::new("planet-design-seed");
        ctx.memory_mut(|m| m.request_focus(id));
        for text in ["4", "x", "2"] {
            panel.edits.observe(std::time::Instant::now());
            panel.notice =
                "A long file message that wraps across several lines in the narrow sidebar.".into();
            assert_eq!(
                frame(&ctx, &mut panel, vec![egui::Event::Text(text.into())]),
                rect
            );
            assert_eq!(ctx.memory(|m| m.focused()), Some(id));
        }
        assert_eq!(panel.edits.seed_text, "4x2");
        panel.edits.seed_text = "42".into();
        panel.edits.observe(std::time::Instant::now());
        panel
            .edits
            .request(std::time::Instant::now() + crate::design_edits::EDIT_DELAY);
        assert_eq!(
            frame(&ctx, &mut panel, vec![egui::Event::Text("3".into())]),
            rect
        );
        assert_eq!(ctx.memory(|m| m.focused()), Some(id));
    }
}
