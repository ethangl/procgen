use std::collections::BTreeMap;

use procgen_core::Vec3;
use procgen_cubesphere::{CubeFace, equiangular_tangent};
use rayon::prelude::*;

use crate::{PlanetError, PlanetField};

/// One resident region per cube face in slice 2; all six have the same resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RegionAddress {
    pub face: CubeFace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellConfig {
    pub face_quads: usize,
    pub radial_cells: usize,
}
pub const PILOT_SHELL: ShellConfig = ShellConfig {
    face_quads: 64,
    radial_cells: 16,
};
impl ShellConfig {
    pub(crate) fn validate(self) -> Result<(), PlanetError> {
        if !self.face_quads.is_power_of_two()
            || !(4..=128).contains(&self.face_quads)
            || !self.radial_cells.is_power_of_two()
            || !(2..=64).contains(&self.radial_cells)
        {
            return Err(PlanetError::Resolution);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Sample {
    pub position: Vec3,
    pub density: f32,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Quad {
    pub region: RegionAddress,
    pub columns: [usize; 4],
}

/// Shared samples are stored once, including the three-face cube corners.
pub struct ShellVolume {
    config: ShellConfig,
    pub(crate) samples: Vec<Sample>,
    pub(crate) quads: Vec<Quad>,
}
impl ShellVolume {
    pub fn config(&self) -> ShellConfig {
        self.config
    }
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }
    pub fn validate(&self) -> Result<(), PlanetError> {
        self.config.validate()?;
        let layers = self.config.radial_cells + 1;
        let columns = 6 * self.config.face_quads.pow(2) + 2;
        if self.samples.len() != columns * layers
            || self.quads.len() != 6 * self.config.face_quads.pow(2)
            || self
                .quads
                .iter()
                .any(|q| q.columns.iter().any(|&c| c >= columns))
            || self
                .samples
                .iter()
                .any(|s| !s.position.is_finite() || !s.density.is_finite())
            || self
                .samples
                .chunks_exact(layers)
                .any(|c| c[0].density <= 0.0 || c[layers - 1].density >= 0.0)
        {
            return Err(PlanetError::Samples);
        }
        Ok(())
    }
    pub(crate) fn cells(
        &self,
    ) -> impl IndexedParallelIterator<Item = (RegionAddress, [usize; 8])> + '_ {
        let layers = self.config.radial_cells + 1;
        (0..self.quads.len() * self.config.radial_cells)
            .into_par_iter()
            .map(move |i| {
                let quad = self.quads[i / self.config.radial_cells];
                let layer = i % self.config.radial_cells;
                // Cell coordinates: bit 0 = face u, bit 1 = face v, bit 2 = radial.
                (
                    quad.region,
                    std::array::from_fn(|corner| {
                        quad.columns[corner % 4] * layers + layer + corner / 4
                    }),
                )
            })
    }
}

pub fn sample_shell(field: &PlanetField, config: ShellConfig) -> Result<ShellVolume, PlanetError> {
    config.validate()?;
    let (directions, quads) = face_grid(config.face_quads);
    let band = field.config().band;
    let columns: Vec<Vec<Sample>> = directions
        .par_iter()
        .map(|&direction| {
            let column = field.column(direction);
            (0..=config.radial_cells)
                .map(|layer| {
                    let offset = -band.below
                        + (band.below + band.above) * (layer as f32 / config.radial_cells as f32);
                    Sample {
                        position: column.position(field, offset),
                        density: column.density(field, offset),
                    }
                })
                .collect()
        })
        .collect();
    let volume = ShellVolume {
        config,
        samples: columns.into_iter().flatten().collect(),
        quads,
    };
    volume.validate()?;
    Ok(volume)
}

// Signed integer cube coordinates identify a sample independently of the requesting face.
fn cube_key(face: CubeFace, x: usize, y: usize, side: usize) -> [i32; 3] {
    let frame = face.frame();
    let p = frame.normal * side as f32 + frame.u_axis * (2 * x) as f32 - frame.u_axis * side as f32
        + frame.v_axis * (2 * y) as f32
        - frame.v_axis * side as f32;
    [p.x as i32, p.y as i32, p.z as i32]
}
pub(crate) fn face_grid(side: usize) -> (Vec<Vec3>, Vec<Quad>) {
    let mut ids = BTreeMap::new();
    let mut directions = Vec::new();
    let mut quads = Vec::new();
    for face in CubeFace::ALL {
        let mut face_ids = Vec::with_capacity((side + 1).pow(2));
        for y in 0..=side {
            for x in 0..=side {
                let key = cube_key(face, x, y, side);
                let id = *ids.entry(key).or_insert_with(|| {
                    let id = directions.len();
                    let p = key.map(|v| equiangular_tangent(v as f32 / side as f32));
                    directions.push(Vec3::new(p[0], p[1], p[2]).normalized());
                    id
                });
                face_ids.push(id);
            }
        }
        for y in 0..side {
            for x in 0..side {
                let i = y * (side + 1) + x;
                quads.push(Quad {
                    region: RegionAddress { face },
                    columns: [
                        face_ids[i],
                        face_ids[i + 1],
                        face_ids[i + side + 1],
                        face_ids[i + side + 2],
                    ],
                });
            }
        }
    }
    (directions, quads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PILOT_PLANET;
    use procgen_cubesphere::{FaceCoordinates, face_to_direction};
    #[test]
    fn every_edge_and_corner_has_one_address_and_matches_face_mapping() {
        let n = 8;
        let mut requests: BTreeMap<[i32; 3], Vec<Vec3>> = BTreeMap::new();
        for face in CubeFace::ALL {
            for y in 0..=n {
                for x in 0..=n {
                    let direction = face_to_direction(FaceCoordinates {
                        face,
                        u: -1.0 + 2.0 * x as f32 / n as f32,
                        v: -1.0 + 2.0 * y as f32 / n as f32,
                    })
                    .unwrap();
                    requests
                        .entry(cube_key(face, x, y, n))
                        .or_default()
                        .push(direction);
                }
            }
        }
        assert_eq!(requests.len(), 6 * n * n + 2);
        assert_eq!(requests.values().filter(|v| v.len() == 3).count(), 8);
        assert_eq!(
            requests.values().filter(|v| v.len() == 2).count(),
            12 * (n - 1)
        );
        for points in requests.values() {
            for p in points {
                assert_eq!(*p, points[0]);
            }
        }
        let (directions, quads) = face_grid(n);
        assert!(
            directions
                .iter()
                .all(|d| requests.values().any(|p| p[0] == *d))
        );
        let mut incidence = vec![0; directions.len()];
        for q in quads {
            for c in q.columns {
                incidence[c] += 1;
            }
        }
        assert_eq!(incidence.iter().filter(|&&n| n == 3).count(), 8);
        assert!(incidence.iter().all(|&n| n == 3 || n == 4));
    }
    #[test]
    fn shell_samples_are_schedule_independent_and_match_direct_queries() {
        let field = PILOT_PLANET.validate(42).unwrap();
        let config = ShellConfig {
            face_quads: 8,
            radial_cells: 8,
        };
        let generate = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| sample_shell(&field, config).unwrap())
        };
        let a = generate(1);
        let b = generate(4);
        for (a, b) in a.samples.iter().zip(&b.samples) {
            assert_eq!(a.position, b.position);
            assert_eq!(a.density, b.density);
            let altitude = a.position.length() - field.config().radius;
            // Reconstructing a direction and altitude adds f32 normalization error.
            assert!((field.density(a.position, altitude).unwrap() - a.density).abs() < 0.00001);
        }
    }
}
