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
    path: PathBuf,
    file_action: Option<FileAction>,
    pub tab: Tab,
    notice: String,
}
impl DesignPanel {
    pub fn new(config: PlanetDesignConfig, path: Option<PathBuf>) -> Self {
        Self {
            edits: DesignEdits::new(config),
            path: path.unwrap_or_else(|| PathBuf::from("planet-design.json")),
            file_action: None,
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
                    ui.label("Current design file");
                    ui.label(
                        self.path
                            .file_name()
                            .unwrap_or(self.path.as_os_str())
                            .to_string_lossy(),
                    )
                    .on_hover_text(self.path.display().to_string());
                    ui.horizontal(|ui| {
                        if ui.button("Load…").clicked() {
                            self.file_action =
                                Some(FileAction::Load(self.path.display().to_string()));
                        }
                        if ui.button("Save controls").clicked() {
                            self.notice = match self.save_to(&self.path) {
                                Ok(()) => format!("Saved {}", self.path.display()),
                                Err(e) => e,
                            };
                        }
                        if ui.button("Save as…").clicked() {
                            self.file_action =
                                Some(FileAction::SaveAs(self.path.display().to_string()));
                        }
                    });
                    ui.horizontal(|ui| {
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
        self.file_dialog(ui.ctx());
    }
    fn save_to(&self, path: &Path) -> Result<(), String> {
        let field = self.edits.validated()?;
        design_file::save(path, field.config()).map_err(|e| e.to_string())
    }
    fn apply_file_action(&mut self, action: &FileAction) -> Result<(), String> {
        let path = PathBuf::from(action.path());
        match action {
            FileAction::Load(_) => {
                let config = design_file::load(&path).map_err(|e| e.to_string())?;
                self.edits.load(config);
                self.notice = format!("Loaded {}", path.display());
            }
            FileAction::SaveAs(_) => {
                self.save_to(&path)?;
                self.notice = format!("Saved {}", path.display());
            }
        }
        self.path = path;
        Ok(())
    }
    fn file_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut action) = self.file_action.take() else {
            return;
        };
        let mut open = true;
        let mut complete = false;
        egui::Window::new(action.label())
            .id(egui::Id::new("planet-design-file-dialog"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("File path");
                ui.add(
                    egui::TextEdit::singleline(action.path_mut())
                        .id(egui::Id::new("planet-design-path"))
                        .desired_width(420.0),
                );
                ui.horizontal(|ui| {
                    if ui.button(action.label()).clicked() {
                        match self.apply_file_action(&action) {
                            Ok(()) => complete = true,
                            Err(e) => self.notice = e,
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        complete = true;
                    }
                });
            });
        if open && !complete {
            self.file_action = Some(action);
        }
    }
}

enum FileAction {
    Load(String),
    SaveAs(String),
}
impl FileAction {
    fn path(&self) -> &str {
        match self {
            Self::Load(path) | Self::SaveAs(path) => path,
        }
    }
    fn path_mut(&mut self) -> &mut String {
        match self {
            Self::Load(path) | Self::SaveAs(path) => path,
        }
    }
    fn label(&self) -> &'static str {
        match self {
            Self::Load(_) => "Load design",
            Self::SaveAs(_) => "Save design as",
        }
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
    fn save_targets_loaded_file_until_load_or_save_as_succeeds() {
        let directory =
            std::env::temp_dir().join(format!("procgen-save-target-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let current = directory.join("planet-design-300km.json");
        let other = directory.join("other.json");
        let config = PlanetDesignConfig::starter(42);
        design_file::save(&current, &config).unwrap();
        let mut panel = DesignPanel::new(config, Some(current.clone()));
        panel.tab = Tab::Design;
        panel.edits.seed_text = "43".into();
        // A draft destination (including stray movement text) cannot redirect Save.
        panel.file_action = Some(FileAction::SaveAs(format!("{}w", current.display())));
        panel.save_to(&panel.path).unwrap();
        assert_eq!(design_file::load(&current).unwrap().seed, 43);
        assert!(!current.with_extension("jsonw").exists());
        assert!(
            panel
                .apply_file_action(&FileAction::Load(other.display().to_string()))
                .is_err()
        );
        assert_eq!(panel.path, current);
        panel
            .apply_file_action(&FileAction::SaveAs(other.display().to_string()))
            .unwrap();
        assert_eq!(panel.path, other);
        panel.edits.seed_text = "44".into();
        panel.save_to(&panel.path).unwrap();
        assert_eq!(design_file::load(&other).unwrap().seed, 44);
        assert_eq!(design_file::load(&current).unwrap().seed, 43);
        panel.edits.seed_text = "invalid".into();
        assert!(
            panel
                .apply_file_action(&FileAction::SaveAs(current.display().to_string()))
                .is_err()
        );
        assert_eq!(panel.path, other);
        panel
            .apply_file_action(&FileAction::Load(current.display().to_string()))
            .unwrap();
        assert_eq!(panel.path, current);
        assert_eq!(panel.edits.seed_text, "43");
        std::fs::remove_dir_all(directory).unwrap();
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
