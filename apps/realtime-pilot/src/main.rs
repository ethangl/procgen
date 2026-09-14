#[cfg(feature = "inspector")]
mod controls;
mod design_cli;
#[cfg(feature = "inspector")]
mod design_controls;
#[cfg(feature = "inspector")]
mod design_edits;
mod design_file;
#[cfg(feature = "inspector")]
mod design_panel;
mod display;
mod ocean;
#[cfg(feature = "inspector")]
mod physical_capture;
#[cfg(feature = "inspector")]
mod physical_color;
#[cfg(feature = "inspector")]
mod physical_gpu;
#[cfg(feature = "inspector")]
mod physical_gpu_render;
#[cfg(feature = "inspector")]
mod physical_inspector;
#[cfg(feature = "inspector")]
mod physical_jobs;
#[cfg(feature = "inspector")]
mod physical_ocean;
#[cfg(feature = "inspector")]
mod physical_record;
#[cfg(feature = "inspector")]
mod physical_render;
#[cfg(feature = "inspector")]
mod physical_surface_layers;
#[cfg(feature = "inspector")]
mod planet_inspector;
mod replay;
#[cfg(feature = "inspector")]
mod stream_inspector;
#[cfg(any(feature = "inspector", test))]
mod stream_record;
#[cfg(feature = "inspector")]
mod stream_render;
mod stress;
#[cfg(feature = "inspector")]
mod surface_material;
mod surface_view;
#[cfg(feature = "inspector")]
mod usable_inspector;

use std::{error::Error, fs, path::PathBuf, time::Instant};

use procgen_realtime_pilot::{INSPECTION_GRID, PRESETS, sample_volume};

fn main() -> Result<(), Box<dyn Error>> {
    if design_cli::selected(std::env::args().skip(1)) {
        return design_cli::run();
    }
    let mut args = std::env::args().skip(1);
    let mut capture = None;
    let mut planet = false;
    let mut stream = false;
    let mut record = None;
    let mut screenshots = false;
    let mut walk_route = false;
    let mut check = false;
    let mut preset = None;
    let mut case_path = None;
    let mut replay_path = None;
    let mut sweep = None;
    let mut samples = None;
    let mut sample_seed = None;
    let mut seed_explicit = false;
    let mut seed = 42_u64;
    let mut surface_view = surface_view::SurfaceViewConfig::default();
    let mut view_explicit = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--normals" => {
                surface_view.normals = args.next().ok_or("--normals needs a mode")?.parse()?;
                view_explicit = true;
            }
            "--surface" => {
                surface_view.overlay = args.next().ok_or("--surface needs an overlay")?.parse()?;
                view_explicit = true;
            }
            "--wireframe" => {
                surface_view.wireframe = true;
                view_explicit = true;
            }
            "--surface-detail" => {
                surface_view.detail = args
                    .next()
                    .ok_or("--surface-detail needs a mode")?
                    .parse()?;
                view_explicit = true;
            }
            "--preset" => {
                preset = Some(
                    args.next()
                        .ok_or("--preset needs hills, ridges, or basins")?,
                )
            }
            "--case" => {
                case_path = Some(PathBuf::from(
                    args.next().ok_or("--case needs a JSON path")?,
                ))
            }
            "--replay" => {
                replay_path = Some(PathBuf::from(
                    args.next().ok_or("--replay needs a JSON path")?,
                ))
            }
            "--sweep" => {
                sweep = Some(PathBuf::from(
                    args.next().ok_or("--sweep needs a directory")?,
                ))
            }
            "--samples" => {
                samples = Some(
                    args.next()
                        .ok_or("--samples needs a count")?
                        .parse::<usize>()?,
                )
            }
            "--sample-seed" => {
                sample_seed = Some(
                    args.next()
                        .ok_or("--sample-seed needs u64")?
                        .parse::<u64>()?,
                )
            }
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
                seed_explicit = true;
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
                    "Surface inspection (--stream): --normals averaged|triangle|density --surface neutral|lod|normals|agreement --surface-detail plain|textured --wireframe.\nSpherical experiments: --stream --preset hills|ridges|basins; --case FILE; --replay FILE --record CSV.\nHeadless stress: --sweep DIRECTORY [--samples N] [--sample-seed U64].\nprocgen-realtime-pilot [--seed U64] [--planet [--check]] [--capture DIRECTORY] [--stream [--record CSV [--screenshots] [--walk-route]]]\n--stream: streaming flight inspector; --record runs the fixed route and exits.\nGPU orbit/descent viewer (default): [--design-file FILE] [--backend gpu|cpu].\nDefault file: planet-design-300km.json. Live octave edits apply automatically; saving is explicit.\nDesign audits and full options: --design --help.\n--planet: spherical regions. --planet --check: headless mesh report."
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    if view_explicit && !stream {
        return Err("surface inspection options require --stream".into());
    }
    if let Some(directory) = sweep {
        if stream
            || planet
            || record.is_some()
            || capture.is_some()
            || replay_path.is_some()
            || preset.is_some()
            || seed_explicit
            || walk_route
            || screenshots
            || check
        {
            return Err("--sweep accepts sampling options or --case FILE".into());
        }
        let selection = if let Some(path) = case_path {
            if samples.is_some() || sample_seed.is_some() {
                return Err("single-case audits do not accept seed sampling options".into());
            }
            stress::SweepSelection::Case(replay::Case::load(&path)?)
        } else {
            stress::SweepSelection::Sample {
                count: samples.unwrap_or(2),
                seed: sample_seed,
            }
        };
        return stress::run(&directory, selection);
    }
    if sample_seed.is_some() || samples.is_some() {
        return Err("--samples/--sample-seed require --sweep".into());
    }
    if (case_path.is_some() || replay_path.is_some())
        && (preset.is_some() || seed_explicit || walk_route)
    {
        return Err(
            "case/replay inputs cannot be overridden by --preset, --seed or --walk-route".into(),
        );
    }
    if case_path.is_some() && replay_path.is_some() {
        return Err("choose --case or --replay".into());
    }
    let replay = replay_path
        .as_ref()
        .map(|p| replay::Replay::load(p))
        .transpose()?;
    let case = case_path
        .as_ref()
        .map(|p| replay::Case::load(p))
        .transpose()?;
    let chosen = procgen_realtime_pilot::PLANET_PRESETS
        .iter()
        .find(|p| p.id == preset.as_deref().unwrap_or("hills"))
        .ok_or("unknown spherical preset")?;
    let scenario = if let Some(r) = &replay {
        r.case.scenario
    } else if let Some(c) = &case {
        c.scenario
    } else {
        procgen_realtime_pilot::Scenario {
            seed,
            planet: chosen.planet,
            route: if walk_route {
                procgen_realtime_pilot::RouteKind::Walk
            } else {
                procgen_realtime_pilot::RouteKind::Flight
            },
        }
    };
    if (preset.is_some() || case.is_some() || replay.is_some()) && !stream {
        return Err("--preset, --case and --replay require --stream".into());
    }
    if replay.is_some() && record.is_none() {
        return Err("--replay requires --record for its output capture".into());
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
            scenario,
            surface_view,
            record.map(|path| stream_record::RecordingConfig {
                path,
                screenshots,
                surface_view,
                replay_build: replay.as_ref().map(|r| r.case.build.clone()),
            }),
            replay,
        )?;
        #[cfg(not(feature = "inspector"))]
        return Err(format!(
            "--stream for seed {} with {:?} normals requires the inspector feature",
            scenario.seed, surface_view.normals
        )
        .into());
        #[cfg(feature = "inspector")]
        return Ok(());
    }
    if record.is_some() {
        return Err("--record requires --stream".into());
    }
    if planet {
        if capture.is_some() {
            return Err("--capture is for headless local-volume diagnostics".into());
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
        return Err("select an experiment or use the default GPU viewer; see --help".into());
    }
    Ok(())
}

#[cfg(feature = "inspector")]
mod physical_height;

#[cfg(feature = "inspector")]
mod physical_gpu_bridge;
