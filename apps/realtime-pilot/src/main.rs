#[cfg(feature = "inspector")]
mod controls;
mod display;
#[cfg(feature = "inspector")]
mod inspector;
#[cfg(feature = "inspector")]
mod planet_inspector;
#[cfg(feature = "inspector")]
mod stream_inspector;
#[cfg(feature = "inspector")]
mod stream_record;
#[cfg(feature = "inspector")]
mod stream_render;
#[cfg(feature = "inspector")]
mod usable_inspector;

use std::{error::Error, fs, path::PathBuf, time::Instant};

use procgen_realtime_pilot::{INSPECTION_GRID, PRESETS, sample_volume};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut capture = None;
    let mut planet = false;
    let mut stream = false;
    let mut record = None;
    let mut screenshots = false;
    let mut walk_route = false;
    let mut check = false;
    let mut seed = 42_u64;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--planet" => planet = true,
            "--stream" => stream = true,
            "--screenshots" => screenshots = true,
            "--walk-route" => walk_route = true,
            "--record" => {
                record = Some(PathBuf::from(
                    args.next().ok_or("--record requires a path")?,
                ))
            }
            "--check" => check = true,
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
                    "procgen-realtime-pilot [--seed U64] [--planet [--check]] [--capture DIRECTORY] [--stream [--record CSV [--screenshots] [--walk-route]]]\n--stream: streaming flight inspector; --record runs the fixed route and exits.\nDefault: local volume inspector. --planet: spherical regions. --planet --check: headless mesh report."
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    if walk_route && (!stream || record.is_none()) {
        return Err("--walk-route requires --stream and --record".into());
    }
    if screenshots && (!stream || record.is_none()) {
        return Err("--screenshots requires --stream and --record".into());
    }
    if stream {
        if planet || check || capture.is_some() {
            return Err("--stream cannot combine with --planet, --check, or --capture".into());
        }
        #[cfg(feature = "inspector")]
        stream_inspector::run(
            seed,
            record.map(|path| stream_record::RecordingConfig {
                path,
                screenshots,
                route: if walk_route {
                    stream_record::RecordingRoute::Walk
                } else {
                    stream_record::RecordingRoute::Flight
                },
            }),
        )?;
        #[cfg(not(feature = "inspector"))]
        return Err("--stream requires the inspector feature".into());
        #[cfg(feature = "inspector")]
        return Ok(());
    }
    if record.is_some() {
        return Err("--record requires --stream".into());
    }
    if planet {
        if capture.is_some() {
            return Err("--capture is for the local volume inspector".into());
        }
        if check {
            use procgen_realtime_pilot::{
                PILOT_PLANET, PILOT_SHELL, contour_shell, planet_overview, sample_shell,
            };
            let start = Instant::now();
            let field = PILOT_PLANET.validate(seed)?;
            let overview = planet_overview(&field, procgen_realtime_pilot::OVERVIEW_FACE_QUADS)?;
            let volume = sample_shell(&field, PILOT_SHELL)?;
            let mesh = contour_shell(&volume)?;
            let elapsed_ms = start.elapsed().as_millis();
            println!("topology={:?}", mesh.topology());
            println!(
                "seed={seed}\nplanet={PILOT_PLANET:#?}\nshell={PILOT_SHELL:?}\nsamples={}\nvertices={}\ntriangles={}\noverview_triangles={}\ngeneration_ms={}",
                volume.sample_count(),
                mesh.positions().len(),
                mesh.triangles().len(),
                overview.triangles().len(),
                elapsed_ms
            );
        } else {
            #[cfg(feature = "inspector")]
            planet_inspector::run(seed);
            #[cfg(not(feature = "inspector"))]
            return Err("Enable inspector or use --planet --check".into());
        }
        return Ok(());
    }
    if check {
        return Err("--check requires --planet".into());
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
