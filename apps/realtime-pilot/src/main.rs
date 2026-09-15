#[cfg(feature = "inspector")]
mod design_controls;
#[cfg(feature = "inspector")]
mod design_edits;
mod design_file;
#[cfg(feature = "inspector")]
mod design_panel;
mod ocean;
#[cfg(feature = "inspector")]
mod physical_capture;
#[cfg(feature = "inspector")]
mod physical_color;
#[cfg(feature = "inspector")]
mod physical_gpu;
#[cfg(feature = "inspector")]
mod physical_gpu_bridge;
#[cfg(feature = "inspector")]
mod physical_gpu_render;
#[cfg(feature = "inspector")]
mod physical_height;
#[cfg(feature = "inspector")]
mod physical_inspector;
#[cfg(feature = "inspector")]
mod physical_ocean;
#[cfg(feature = "inspector")]
mod physical_record;
#[cfg(feature = "inspector")]
mod physical_render;
#[cfg(feature = "inspector")]
mod physical_surface_layers;
#[cfg(feature = "inspector")]
mod physical_visibility;

use std::{error::Error, path::PathBuf};

use procgen_realtime_pilot::{
    BUILD_ID, DesignPreviewConfig, PlanetDesignConfig, PreviewArea, PreviewBands,
    generate_design_preview,
};

const HELP: &str = "\
procgen-realtime-pilot [--seed U64 | --design-file FILE] [--write-design FILE]
Opens the GPU height-terrain viewer: orbit and free flight over live octave
controls, oceans, and planet-design-300km.json. Unchanged height tiles retain
their GPU buffers while the camera moves. Optional radial camera protection
keeps 5 m of terrain clearance; it is not swept collision. Sea level in the
Design tab applies immediately; Save controls writes terrain and ocean settings.

  --design-file FILE       load a saved design (includes its seed)
  --seed U64               start from the starter design for this seed
  --write-design FILE      save the full config before running
  --check                  headless preview audit, JSON to stdout
    --patch-span METERS      audit a ground patch instead of the whole planet
    --solo INDEX             audit one octave band, zero-based
    --preview-quads N        16|32|64|128|256 quads per cube face
  --explore-record FILE.csv   record a 90 s orbit/descent/flight route and six screenshots
  --visibility-record FILE.csv
                           record fixed down/horizon views at 5 m and 500 m
                           clearance (35 s, four screenshots)
  --help                   this message";

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut record = None;
    let mut visibility_record = None;
    let mut seed = None;
    let mut path = None;
    let mut output = None;
    let mut check = false;
    let mut preview_explicit = false;
    let mut preview = DesignPreviewConfig {
        area: PreviewArea::Planet,
        bands: PreviewBands::Combined,
        quads: 128,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--explore-record" => {
                record = Some(PathBuf::from(
                    args.next().ok_or("--explore-record needs a CSV path")?,
                ))
            }
            "--visibility-record" => {
                visibility_record = Some(PathBuf::from(
                    args.next().ok_or("--visibility-record needs a CSV path")?,
                ))
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
            "--check" => check = true,
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
                println!("{HELP}");
                return Ok(());
            }
            _ => return Err(format!("unsupported option: {arg}; use --help").into()),
        }
    }
    if preview_explicit && !check {
        return Err("height-preview options require --check".into());
    }
    if record.is_some() && visibility_record.is_some() {
        return Err("select one recording route".into());
    }
    if check && (record.is_some() || visibility_record.is_some()) {
        return Err("recording requires the viewer".into());
    }
    if path.is_some() && seed.is_some() {
        return Err("--design-file includes its seed; do not also pass --seed".into());
    }
    let document = match &path {
        Some(p) => design_file::load(p)?,
        None if seed.is_some() => {
            design_file::DesignFile::new(PlanetDesignConfig::starter(seed.unwrap()))
        }
        None => {
            let default_path = default_design_path();
            let config = design_file::load(&default_path)?;
            #[cfg(feature = "inspector")]
            {
                path = Some(default_path);
            }
            config
        }
    };
    let config = &document.design;
    if check {
        let result = generate_design_preview(config, preview)?;
        if let Some(p) = &output {
            design_file::save(p, &document)?;
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "build": BUILD_ID, "seed": config.seed, "radius_m": config.radius_m,
                "vertices": result.positions_m().len(), "triangles": result.triangles().len(),
                "preview_spacing_m": result.spacing_m, "octave_weights": result.octave_weights,
                "full_resolution_selected_band_heights_m": result.distribution,
                // The preview is a height surface and never evaluates this
                // term; it is reported so the audit records the whole design.
                "volume": config.volume,
            }))?
        );
        return Ok(());
    }
    #[cfg(feature = "inspector")]
    {
        config.validate()?;
        if let Some(p) = &output {
            design_file::save(p, &document)?;
        }
        let record = match visibility_record {
            Some(path) => Some(physical_record::PhysicalRecord::visibility(path)?),
            None => record
                .map(physical_record::PhysicalRecord::new)
                .transpose()?,
        };
        physical_inspector::run(document, output.or(path), record);
        Ok(())
    }
    #[cfg(not(feature = "inspector"))]
    Err("enable the inspector feature or use --check".into())
}

fn default_design_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../planet-design-300km.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_default_design_file_loads_at_its_documented_radius() {
        let config = design_file::load(&default_design_path()).unwrap();
        assert_eq!(config.design.radius_m, 300_000.0);
    }
}
