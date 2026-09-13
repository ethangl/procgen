use procgen_realtime_pilot::{
    BUILD_ID, DesignPreviewConfig, PlanetDesignConfig, PreviewArea, PreviewBands, VOXEL_ROOT_LOD,
    generate_design_preview,
};
use std::{error::Error, path::PathBuf};

pub fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
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
            "--check" => mode.set(DesignMode::PreviewAudit)?,
            "--check-chunks" => mode.set(DesignMode::ChunkAudit)?,
            "--check-residency" => mode.set(DesignMode::ResidencyAudit)?,
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
                    "--design [--seed U64 | --design-file FILE] [--write-design FILE] [--check]\nPreview: --patch-span METERS --solo INDEX --preview-quads 16|32|64|128|256\nChunk audit: --check-chunks [--chunk-lod 0..{VOXEL_ROOT_LOD}] [--chunk-point X,Y,Z] (integer meters).\nResidency audit: --check-residency (orbit, ground, rapid travel, revisit).\nWithout an audit flag, opens the physical planet and octave editor. --write-design saves the full config."
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
    if matches!(mode, DesignMode::ChunkAudit | DesignMode::ResidencyAudit) && preview_explicit {
        return Err("chunk/residency audits do not accept height-preview options".into());
    }
    if mode != DesignMode::ChunkAudit && (chunk_lod.is_some() || chunk_point.is_some()) {
        return Err("--chunk-lod/--chunk-point require --check-chunks".into());
    }
    if path.is_some() && seed.is_some() {
        return Err("--design-file includes its seed; do not also pass --seed".into());
    }
    let config = match &path {
        Some(p) => crate::design_file::load(p)?,
        None => PlanetDesignConfig::starter(seed.unwrap_or(42)),
    };
    let field = config.validate()?;
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
        crate::design_inspector::run(config, preview, output.or(path));
        Ok(())
    }
    #[cfg(not(feature = "inspector"))]
    Err("enable the inspector feature or use --design --check".into())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DesignMode {
    Editor,
    PreviewAudit,
    ChunkAudit,
    ResidencyAudit,
}
impl DesignMode {
    fn set(&mut self, mode: Self) -> Result<(), Box<dyn Error>> {
        if *self != Self::Editor {
            return Err("select only one design audit flag".into());
        }
        *self = mode;
        Ok(())
    }
}
