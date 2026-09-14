use procgen_realtime_pilot::{
    BUILD_ID, DesignPreviewConfig, PlanetDesignConfig, PreviewArea, PreviewBands, VOXEL_ROOT_LOD,
    generate_design_preview,
};
use std::{error::Error, path::PathBuf};

pub fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut record = None;
    let mut backend = None;
    let mut seed = None;
    let mut path = None;
    let mut output = None;
    let mut mode = DesignMode::Editor;
    let mut chunk_lod = None;
    let mut chunk_point = None;
    let mut preview_explicit = false;
    let mut preview = DesignPreviewConfig {
        area: PreviewArea::Planet,
        bands: PreviewBands::Combined,
        quads: 128,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--design" => {}
            "--explore-record" => {
                record = Some(PathBuf::from(
                    args.next().ok_or("--explore-record needs a CSV path")?,
                ))
            }
            "--backend" => {
                let value = args.next().ok_or("--backend needs gpu or cpu")?;
                if !matches!(value.as_str(), "gpu" | "cpu") {
                    return Err("--backend needs gpu or cpu".into());
                }
                backend = Some(value);
            }
            "--seed" => seed = Some(args.next().ok_or("--seed needs u64")?.parse()?),
            "--design-file" => {
                path = Some(PathBuf::from(
                    args.next().ok_or("--design-file needs a path")?,
                ))
            }
            "--write-design" => {
                output = Some(PathBuf::from(
                    args.next().ok_or("--write-design needs a path")?,
                ))
            }
            "--check-explore" => mode.set(DesignMode::ExplorationAudit)?,
            "--explore" => mode.set(DesignMode::Explore)?,
            "--check" => mode.set(DesignMode::PreviewAudit)?,
            "--check-chunks" => mode.set(DesignMode::ChunkAudit)?,
            "--check-residency" => mode.set(DesignMode::ResidencyAudit)?,
            "--check-surfaces" => mode.set(DesignMode::SurfaceAudit)?,
            "--chunk-lod" => {
                chunk_lod = Some(args.next().ok_or("--chunk-lod needs an integer")?.parse()?)
            }
            "--chunk-point" => {
                let input = args
                    .next()
                    .ok_or("--chunk-point needs integer x,y,z meters")?;
                let values: Vec<i32> =
                    input.split(',').map(str::parse).collect::<Result<_, _>>()?;
                let [x_m, y_m, z_m] = values.as_slice() else {
                    return Err("--chunk-point needs three comma-separated integers".into());
                };
                chunk_point = Some(procgen_realtime_pilot::VoxelPosition {
                    x_m: *x_m,
                    y_m: *y_m,
                    z_m: *z_m,
                });
            }
            "--patch-span" => {
                preview_explicit = true;
                preview.area = PreviewArea::Patch {
                    latitude_deg: 0.0,
                    longitude_deg: 0.0,
                    span_m: args.next().ok_or("--patch-span needs meters")?.parse()?,
                }
            }
            "--solo" => {
                preview_explicit = true;
                preview.bands = PreviewBands::Only(
                    args.next()
                        .ok_or("--solo needs a zero-based index")?
                        .parse()?,
                )
            }
            "--preview-quads" => {
                preview_explicit = true;
                preview.quads = args
                    .next()
                    .ok_or("--preview-quads needs a count")?
                    .parse()?
            }
            "--help" => {
                println!(
                    "--design [--seed U64 | --design-file FILE] [--write-design FILE] [--check]\nPreview: --patch-span METERS --solo INDEX --preview-quads 16|32|64|128|256\nChunk audit: --check-chunks [--chunk-lod 0..{VOXEL_ROOT_LOD}] [--chunk-point X,Y,Z] (integer meters).\nResidency audit: --check-residency (orbit, ground, rapid travel, revisit).\nSurface audit: --check-surfaces (resident chunk meshes, mixed LOD seams, fine collision).\nPhysical exploration: --explore [--backend gpu|cpu] (GPU by default; CPU audit explicit); --explore-record FILE.csv records a 90-second native route and six screenshots. --check-explore for a headless coverage and walking audit.\nWithout a mode flag, opens the physical planet and octave editor. --write-design saves the full config."
                );
                return Ok(());
            }
            _ => {
                return Err(
                    format!("unsupported design option: {arg}; use --design --help").into(),
                );
            }
        }
    }
    if matches!(
        mode,
        DesignMode::Explore
            | DesignMode::ExplorationAudit
            | DesignMode::ChunkAudit
            | DesignMode::ResidencyAudit
            | DesignMode::SurfaceAudit
    ) && preview_explicit
    {
        return Err("voxel modes do not accept height-preview options".into());
    }
    if mode != DesignMode::ChunkAudit && (chunk_lod.is_some() || chunk_point.is_some()) {
        return Err("--chunk-lod/--chunk-point require --check-chunks".into());
    }
    if record.is_some() && backend.as_deref() == Some("cpu") {
        return Err("--explore-record requires the GPU exploration backend".into());
    }
    if record.is_some() && mode != DesignMode::Explore {
        return Err("--explore-record requires --explore".into());
    }
    if backend.is_some() && mode != DesignMode::Explore {
        return Err("--backend requires --explore".into());
    }
    if path.is_some() && seed.is_some() {
        return Err("--design-file includes its seed; do not also pass --seed".into());
    }
    let config = match &path {
        Some(p) => crate::design_file::load(p)?,
        None => PlanetDesignConfig::starter(seed.unwrap_or(42)),
    };
    let field = config.validate()?;
    if mode == DesignMode::ExplorationAudit {
        let result =
            procgen_realtime_pilot::audit_physical_exploration(std::sync::Arc::new(field))?;
        if let Some(p) = &output {
            crate::design_file::save(p, &config)?;
        }
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    if mode == DesignMode::SurfaceAudit {
        let result = procgen_realtime_pilot::audit_voxel_surfaces(std::sync::Arc::new(field))?;
        if let Some(p) = &output {
            crate::design_file::save(p, &config)?;
        }
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"build":BUILD_ID,"seed":config.seed,"radius_m":config.radius_m,"surface":result})
            )?
        );
        return Ok(());
    }
    if mode == DesignMode::ResidencyAudit {
        let result = procgen_realtime_pilot::audit_voxel_travel(
            std::sync::Arc::new(field),
            procgen_realtime_pilot::VoxelResidencyConfig::default(),
        )?;
        if let Some(p) = &output {
            crate::design_file::save(p, &config)?;
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "build": BUILD_ID, "seed": config.seed, "radius_m": config.radius_m, "residency": result,
            }))?
        );
        return Ok(());
    }
    if mode == DesignMode::ChunkAudit {
        use procgen_realtime_pilot::{VoxelAuditConfig, VoxelPosition, audit_voxel_chunk};
        // Quantize this headless probe once on the host. Chunk addresses and
        // shared samples thereafter use integer arithmetic only.
        let point = match chunk_point {
            Some(point) => point,
            None => VoxelPosition {
                x_m: (config.radius_m + field.elevation_m(procgen_core::Vec3::X, 0.0)?).round()
                    as i32,
                y_m: 0,
                z_m: 0,
            },
        };
        let result = audit_voxel_chunk(
            &field,
            VoxelAuditConfig {
                point,
                lod: chunk_lod.unwrap_or(0),
            },
        )?;
        if let Some(p) = &output {
            crate::design_file::save(p, &config)?;
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "build": BUILD_ID, "seed":config.seed,"radius_m":config.radius_m,"anchor_m":point,"chunk":result,
            }))?
        );
        return Ok(());
    }
    if mode == DesignMode::PreviewAudit {
        let result = generate_design_preview(&config, preview)?;
        if let Some(p) = &output {
            crate::design_file::save(p, &config)?;
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "build": BUILD_ID, "seed": config.seed, "radius_m": config.radius_m,
                "vertices": result.positions_m().len(), "triangles": result.triangles().len(),
                "preview_spacing_m": result.spacing_m, "octave_weights": result.octave_weights,
                "full_resolution_selected_band_heights_m": result.distribution,
            }))?
        );
        return Ok(());
    }
    #[cfg(feature = "inspector")]
    {
        if let Some(p) = &output {
            crate::design_file::save(p, &config)?;
        }
        if mode == DesignMode::Explore {
            let backend = match backend.as_deref().unwrap_or("gpu") {
                "gpu" => crate::physical_gpu::ExplorationBackend::Gpu,
                "cpu" => crate::physical_gpu::ExplorationBackend::Cpu,
                _ => unreachable!("validated backend"),
            };
            let record = record
                .map(crate::physical_record::PhysicalRecord::new)
                .transpose()?;
            crate::physical_inspector::run(config, path, backend, record);
        } else {
            crate::design_inspector::run(config, preview, output.or(path));
        }
        Ok(())
    }
    #[cfg(not(feature = "inspector"))]
    Err("enable the inspector feature or use --design --check".into())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DesignMode {
    Editor,
    Explore,
    ExplorationAudit,
    PreviewAudit,
    ChunkAudit,
    ResidencyAudit,
    SurfaceAudit,
}
impl DesignMode {
    fn set(&mut self, mode: Self) -> Result<(), Box<dyn Error>> {
        if *self != Self::Editor {
            return Err("select only one design mode".into());
        }
        *self = mode;
        Ok(())
    }
}
