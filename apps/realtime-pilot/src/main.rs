mod display;
#[cfg(feature = "inspector")]
mod inspector;

use std::{error::Error, fs, path::PathBuf, time::Instant};

use procgen_realtime_pilot::{INSPECTION_GRID, PRESETS, sample_volume};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut capture = None;
    let mut seed = 42_u64;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed" => {
                seed = args
                    .next()
                    .ok_or("--seed needs an unsigned integer")?
                    .parse()?
            }
            "--capture" => {
                capture = Some(PathBuf::from(
                    args.next().ok_or("--capture needs a directory")?,
                ))
            }
            "--help" => {
                println!(
                    "procgen-realtime-pilot [--seed U64] [--capture DIRECTORY]\nWithout --capture, open the interactive volume inspector."
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    if let Some(directory) = capture {
        fs::create_dir_all(&directory)?;
        for (index, preset) in PRESETS.iter().enumerate() {
            let start = Instant::now();
            let volume = sample_volume(&preset.config.validate(seed)?, INSPECTION_GRID)?;
            let elapsed_ms = start.elapsed().as_millis();
            display::write_ppm(&directory.join(format!("preset-{index}.ppm")), &volume)?;
            let panel_names = display::VIEWS
                .iter()
                .map(|view| view.title)
                .collect::<Vec<_>>()
                .join(", ");
            let info = format!(
                "{}\nseed={seed}\ngrid={INSPECTION_GRID:?}\nconfig={:#?}\ngeneration_ms={}\nPanels: {panel_names}.\n",
                preset.name, preset.config, elapsed_ms
            );
            fs::write(directory.join(format!("preset-{index}.txt")), &info)?;
            print!("{info}");
        }
    } else {
        #[cfg(feature = "inspector")]
        inspector::run(seed);
        #[cfg(not(feature = "inspector"))]
        return Err(
            "This build requires --capture DIRECTORY. Enable the inspector feature for the UI."
                .into(),
        );
    }
    Ok(())
}
