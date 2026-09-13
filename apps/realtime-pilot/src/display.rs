use std::{error::Error, fs, path::Path};

use procgen_realtime_pilot::{Axis, FieldError, INSPECTION_GRID, Volume};

#[derive(Clone, Copy)]
pub enum ViewData {
    Height,
    Density { axis: Axis, index: usize },
}

#[derive(Clone, Copy)]
pub struct View {
    pub title: &'static str,
    pub data: ViewData,
}

pub const VIEWS: [View; 4] = [
    View {
        title: "Base height · XZ",
        data: ViewData::Height,
    },
    View {
        title: "Density · XY",
        data: ViewData::Density {
            axis: Axis::Z,
            index: INSPECTION_GRID.side / 2,
        },
    },
    View {
        title: "Density · XZ",
        data: ViewData::Density {
            axis: Axis::Y,
            index: INSPECTION_GRID.side / 2,
        },
    },
    View {
        title: "Density · ZY",
        data: ViewData::Density {
            axis: Axis::X,
            index: INSPECTION_GRID.side / 2,
        },
    },
];

impl View {
    pub fn pixels(self, volume: &Volume) -> Result<Vec<[u8; 3]>, FieldError> {
        match self.data {
            ViewData::Height => Ok(height_pixels(volume)),
            ViewData::Density { axis, index } => density_pixels(volume, axis, index),
        }
    }
}

/// Display-only colors; sampled values stay unchanged.
fn height_pixels(volume: &Volume) -> Vec<[u8; 3]> {
    image_rows(volume.heights(), volume.grid().side, |height| {
        let t = ((height + 0.3) / 1.1).clamp(0.0, 1.0);
        blend([25, 54, 76], [240, 219, 153], t)
    })
}

fn density_pixels(volume: &Volume, axis: Axis, index: usize) -> Result<Vec<[u8; 3]>, FieldError> {
    let values = volume.slice(axis, index)?;
    Ok(image_rows(&values, volume.grid().side, |density| {
        let color = if density >= 0.0 {
            [158, 111, 65]
        } else {
            [18, 38, 58]
        };
        blend([227, 235, 226], color, (density.abs() / 0.04).min(1.0))
    }))
}

fn image_rows(values: &[f32], side: usize, color: impl Fn(f32) -> [u8; 3]) -> Vec<[u8; 3]> {
    values
        .chunks_exact(side)
        .rev()
        .flat_map(|row| row.iter().map(|&v| color(v)))
        .collect()
}

fn blend(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    std::array::from_fn(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t) as u8)
}

pub fn write_ppm(path: &Path, volume: &Volume) -> Result<(), Box<dyn Error>> {
    let side = volume.grid().side;
    let panels = VIEWS
        .iter()
        .map(|view| view.pixels(volume))
        .collect::<Result<Vec<_>, _>>()?;
    let mut bytes = format!("P6\n{} {}\n255\n", side * 2, side * 2).into_bytes();
    for panel_row in 0..2 {
        for row in 0..side {
            for panel_col in 0..2 {
                for pixel in &panels[panel_row * 2 + panel_col][row * side..(row + 1) * side] {
                    bytes.extend(pixel);
                }
            }
        }
    }
    fs::write(path, bytes)?;
    Ok(())
}
