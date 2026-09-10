mod climate;
mod geology;
mod tectonics;

use crate::model::{GenerationSettings, Phase};
use bevy_egui::egui;

/// Each phase edits only its own settings; the sidebar shows one phase at a
/// time.
pub(super) fn phase_controls(ui: &mut egui::Ui, phase: Phase, settings: &mut GenerationSettings) {
    match phase {
        Phase::Tectonics => tectonics::controls(ui, &mut settings.tectonics),
        Phase::Geology => geology::controls(
            ui,
            &mut settings.geology,
            settings.tectonics.kinematics,
            settings.tectonics.elevation.sea_level,
        ),
        Phase::Climate => climate::controls(ui, &mut settings.climate),
    }
}
