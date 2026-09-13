use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    time::Instant,
};

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use procgen_realtime_pilot::{
    FieldError, INSPECTION_GRID, NoiseConfig, PRESETS, TerrainConfig, Volume, sample_volume,
};

use crate::display;

pub fn run(seed: u64) {
    let mut inspector = Inspector::new(seed);
    inspector.generate();
    App::new()
        .insert_non_send_resource(inspector)
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Real-time world pilot — local fields".into(),
                resolution: (1180, 850).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn(Camera2d);
        })
        .add_systems(EguiPrimaryContextPass, inspector_ui)
        .run();
}

struct Generated {
    volume: Volume,
    config: TerrainConfig,
    seed: u64,
    elapsed_ms: u128,
}

struct Inspector {
    config: TerrainConfig,
    seed: String,
    selected: usize,
    generated: Option<Generated>,
    pending: Option<Receiver<Result<Generated, FieldError>>>,
    textures: Vec<egui::TextureHandle>,
    views: [display::View; 4],
    status: String,
}

impl Inspector {
    fn new(seed: u64) -> Self {
        Self {
            config: PRESETS[0].config,
            seed: seed.to_string(),
            selected: 0,
            generated: None,
            pending: None,
            textures: Vec::new(),
            views: display::VIEWS,
            status: String::new(),
        }
    }

    fn generate(&mut self) {
        let Ok(seed) = self.seed.parse::<u64>() else {
            self.status = "Seed must be an unsigned 64-bit integer.".into();
            return;
        };
        let config = self.config;
        let field = match config.validate(seed) {
            Ok(field) => field,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        self.status = "Sampling volume…".into();
        std::thread::spawn(move || {
            let start = Instant::now();
            let result = sample_volume(&field, INSPECTION_GRID).map(|volume| Generated {
                volume,
                config,
                seed,
                elapsed_ms: start.elapsed().as_millis(),
            });
            // Closing the application can drop the receiver before this finishes.
            let _ = sender.send(result);
        });
    }

    fn poll(&mut self) -> bool {
        let Some(receiver) = &self.pending else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.status = "Generation worker stopped without a result.".into();
                return false;
            }
        };
        self.pending = None;
        match result {
            Ok(generated) => {
                self.status = format!(
                    "{}³ samples · {} ms",
                    generated.volume.grid().side,
                    generated.elapsed_ms
                );
                eprintln!(
                    "Generated seed={} config={:#?}",
                    generated.seed, generated.config
                );
                self.generated = Some(generated);
                true
            }
            Err(error) => {
                self.status = error.to_string();
                false
            }
        }
    }

    fn upload(&mut self, ctx: &egui::Context) -> Result<(), FieldError> {
        let Some(generated) = &self.generated else {
            return Ok(());
        };
        let volume = &generated.volume;
        let images = self
            .views
            .iter()
            .map(|view| view.pixels(volume))
            .collect::<Result<Vec<_>, _>>()?;
        self.textures = images
            .into_iter()
            .enumerate()
            .map(|(index, pixels)| {
                let rgb: Vec<u8> = pixels.into_iter().flatten().collect();
                let image = egui::ColorImage::from_rgb([volume.grid().side; 2], &rgb);
                ctx.load_texture(
                    format!("field-{index}"),
                    image,
                    egui::TextureOptions::NEAREST,
                )
            })
            .collect();
        Ok(())
    }
}

fn inspector_ui(mut contexts: EguiContexts, mut state: NonSendMut<Inspector>) -> Result {
    let ctx = contexts.ctx_mut()?;
    let mut upload = state.poll();
    egui::SidePanel::left("controls").exact_width(260.0).show(ctx, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Local terrain fields");
            ui.label("Slice 1 · CPU volume inspector");
            ui.separator();
            for (index, preset) in PRESETS.iter().enumerate() {
                if ui.selectable_label(state.selected == index, preset.name).clicked() {
                    state.selected = index;
                    state.config = preset.config;
                }
            }
            ui.label("Seed");
            ui.text_edit_singleline(&mut state.seed);
            ui.separator();
            let config = &mut state.config;
            // Leave enough room for the damping labels beside their value editors.
            ui.spacing_mut().slider_width = 75.0;
            for (label, value, range) in [
                ("Wavelength", &mut config.noise.wavelength, NoiseConfig::WAVELENGTH_RANGE),
                ("Sharpness", &mut config.noise.sharpness, NoiseConfig::SHARPNESS_RANGE),
                ("Warp", &mut config.noise.perturbation, NoiseConfig::SHAPING_RANGE),
                ("Slope damping", &mut config.noise.slope_erosion, NoiseConfig::SHAPING_RANGE),
                ("Altitude damping", &mut config.noise.altitude_erosion, NoiseConfig::SHAPING_RANGE),
                ("Ridge damping", &mut config.noise.ridge_erosion, NoiseConfig::SHAPING_RANGE),
                ("Gain", &mut config.noise.gain, NoiseConfig::GAIN_RANGE),
                ("Height", &mut config.height_scale, TerrainConfig::HEIGHT_RANGE),
                ("3D detail", &mut config.detail_scale, TerrainConfig::DETAIL_RANGE),
                ("Caves / area", &mut config.cave_density, TerrainConfig::CAVE_DENSITY_RANGE),
            ] {
                ui.add(egui::Slider::new(value, range).text(label));
            }
            ui.separator();
            if ui.add_enabled(state.pending.is_none(), egui::Button::new("Generate")).clicked() { state.generate(); }
            ui.label(&state.status);
            if let Some(generated) = &state.generated
                && (generated.config != state.config || state.seed.parse::<u64>().ok() != Some(generated.seed)) {
                ui.colored_label(egui::Color32::YELLOW, "Controls changed. Generate to apply.");
            }
            ui.separator();
            ui.label("Brown: solid · Blue: air\nLight boundary: near zero density");
            ui.label(format!("Local box: {} to {} on each axis. Height map shows the base surface; density cuts include 3D detail and caves.", INSPECTION_GRID.min.x, INSPECTION_GRID.max.x));
        });
    });
    if upload {
        state.upload(ctx)?;
        upload = false;
    }
    egui::CentralPanel::default().show(ctx, |ui| {
        let size = ((ui.available_width() - 24.0) / 2.0)
            .min((ui.available_height() - 130.0) / 2.0)
            .max(32.0);
        if state.textures.is_empty() {
            ui.label("Generating the first volume…");
            return;
        }
        egui::Grid::new("fields")
            .spacing([16.0, 14.0])
            .show(ui, |ui| {
                for index in 0..state.views.len() {
                    ui.vertical(|ui| {
                        ui.label(state.views[index].title);
                        ui.image((state.textures[index].id(), egui::vec2(size, size)));
                        if let display::ViewData::Density { axis, index: cut } =
                            &mut state.views[index].data
                        {
                            upload |= ui
                                .add(
                                    egui::Slider::new(cut, 0..=INSPECTION_GRID.side - 1)
                                        .text(format!("{axis:?} slice")),
                                )
                                .changed();
                        } else {
                            ui.label("Elevation: −0.3 (dark) to +0.8 (light)");
                        }
                    });
                    if index % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
    });
    if upload {
        state.upload(ctx)?;
    }
    Ok(())
}
