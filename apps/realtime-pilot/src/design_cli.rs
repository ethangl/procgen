use procgen_realtime_pilot::{
    BUILD_ID, DesignPreviewConfig, PlanetDesignConfig, PreviewArea, PreviewBands,
    generate_design_preview,
};
use std::{error::Error, path::PathBuf};

pub fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut seed = None;
    let mut path = None;
    let mut output = None;
    let mut check = false;
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
            "--check" => check = true,
            "--patch-span" => {
                preview.area = PreviewArea::Patch {
                    latitude_deg: 0.0,
                    longitude_deg: 0.0,
                    span_m: args.next().ok_or("--patch-span needs meters")?.parse()?,
                }
            }
            "--solo" => {
                preview.bands = PreviewBands::Only(
                    args.next()
                        .ok_or("--solo needs a zero-based index")?
                        .parse()?,
                )
            }
            "--preview-quads" => {
                preview.quads = args
                    .next()
                    .ok_or("--preview-quads needs a count")?
                    .parse()?
            }
            "--help" => {
                println!(
                    "--design [--seed U64 | --design-file FILE] [--write-design FILE] [--check]\nPreview: --patch-span METERS --solo INDEX --preview-quads 16|32|64|128|256\nWithout --check, opens the physical planet and octave editor. --write-design saves the full config."
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
    if path.is_some() && seed.is_some() {
        return Err("--design-file includes its seed; do not also pass --seed".into());
    }
    let config = match &path {
        Some(p) => crate::design_file::load(p)?,
        None => PlanetDesignConfig::starter(seed.unwrap_or(42)),
    };
    config.validate()?;
    if check {
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
