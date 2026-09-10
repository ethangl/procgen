//! The two interior relief fields base elevation adds to its crust-class
//! answer, which neither the cooling curve nor the continental base varies
//! over a plate interior.
//!
//! Dynamic topography is the surface's response to the flow underneath it:
//! where the flow converges the surface sags and where it diverges it swells.
//! The [`FlowField`] the kinematics fit reads stands in for mantle flow, so
//! its divergence over the mesh, negated and normalised, is that response, and
//! it applies to oceanic and continental crust alike.
//!
//! The continental basement is the thickness contrast between old shields and
//! younger provinces: three octaves of the fixed-polynomial gradient noise,
//! spanning a tenth of a great circle down to about a dozen cells of the
//! default mesh, on continental crust only.
//!
//! Both interior relief fields are broad and gentle by design, so that
//! interior relief cannot drown a continental interior on its own: with both
//! terms nominally full against it an untapered continental cell stands at
//! `0.65 - 0.03 - 0.05`, which is 0.57 and still above the default sea level
//! of 0.5 that [`crate::CoarseElevationConfig`] carries. That nominal is not a
//! bound, because the dynamic term is normalised by the divergence field's
//! root-mean-square rather than its peak; the measured margin is thinner, and
//! `docs/plate-movement.md` records it. Inside the taper the two terms move a
//! coast on purpose, which is the point of a shelf sitting near the datum.
//!
//! Both read [`BaseElevationConfig`], which is the stage's data contract;
//! [`crate::base_elevation`] composes them onto the crust-class base.

use crate::{BaseElevationConfig, BaseElevationError, CellCrust, CrustClass, FlowField};
use procgen_core::{RandomStream, random_streams::PLATE_BASEMENT};
use procgen_noise::{
    OctaveConfig, OctaveGain, Validated, amplitude_sum, fbm_3d, fold_seed_u64_to_u32,
};
use procgen_sphere_mesh::SphereMesh;

/// Root-mean-square of the flow field's mesh divergence, which the dynamic
/// topography term divides by so that its amplitude is the swell a typical
/// divergence raises rather than an arbitrary scale.
///
/// Measured once over the viewer's default mesh and flow field — 65,536 cells,
/// jitter 0.8, sampling seed 7, motion seed 7, flow frequency 1.0 — where the
/// divergence field spans -3.38 to 5.53 with an RMS of 1.206, rounded here.
/// The peak is 4.6 times the RMS, so the term reaches about four and a half
/// times its amplitude at the single most divergent cell.
const DIVERGENCE_SCALE: f32 = 1.2;

/// Octaves of the basement field. Three at halving amplitude and doubling
/// frequency reach a quarter of the configured wavelength, which is where a
/// wavelength stops being basement and starts being terrain detail.
const BASEMENT_OCTAVES: u32 = 3;
const BASEMENT_LACUNARITY: f32 = 2.0;
const BASEMENT_OCTAVE_GAIN: f32 = 0.5;

/// Sag where the flow field converges and swell where it diverges, scaled so
/// that the amplitude is what a typical divergence raises.
pub(crate) fn dynamic_topography_field(
    mesh: &SphereMesh,
    flow_field: &FlowField,
    config: BaseElevationConfig,
) -> Vec<f32> {
    let mut field = mesh.cell_divergence(|direction| flow_field.velocity_at(direction));
    let scale = config.dynamic_topography_amplitude / DIVERGENCE_SCALE;
    field
        .iter_mut()
        // Convergent flow sags the surface, so a positive divergence lifts it.
        .for_each(|value| *value *= -scale);
    field
}

/// The continental basement's octave sum, normalized by its amplitude sum and
/// scaled by the configured relief. Exactly zero on oceanic crust, whose
/// elevation is the cooling curve's to set.
pub(crate) fn basement_field(
    mesh: &SphereMesh,
    cell_crust: CellCrust<'_>,
    config: BaseElevationConfig,
    octaves: Validated<OctaveConfig>,
) -> Vec<f32> {
    let key = fold_seed_u64_to_u32(RandomStream::new(config.seed, PLATE_BASEMENT).sample_u64(0, 0));
    let gain = basement_gain();
    let normalization = config.basement_amplitude / amplitude_sum(octaves, gain);
    (0..mesh.cell_count())
        .map(|cell| match cell_crust.class(cell) {
            CrustClass::Oceanic => 0.0,
            CrustClass::Continental => {
                let direction = mesh.cell_centers[cell].normalized();
                fbm_3d(key, direction, octaves, gain).value * normalization
            }
        })
        .collect()
}

/// Amplitudes halve each octave. Constructed from a constant that the octave
/// count is chosen against, so the conversion cannot fail.
fn basement_gain() -> OctaveGain {
    OctaveGain::new(BASEMENT_OCTAVE_GAIN).expect("a halving gain is in the unit interval")
}

/// Validates the two amplitudes and the basement lattice once, and returns
/// the octave progression the basement field samples, so the frequency check
/// and the progression the hot loop uses are the same arithmetic.
pub(crate) fn validate_interior_relief(
    config: BaseElevationConfig,
) -> Result<Validated<OctaveConfig>, BaseElevationError> {
    let amplitudes = [
        config.dynamic_topography_amplitude,
        config.basement_amplitude,
    ];
    if amplitudes
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(BaseElevationError::InvalidInteriorAmplitude);
    }
    OctaveConfig {
        octaves: BASEMENT_OCTAVES,
        frequency: config.basement_frequency,
        lacunarity: BASEMENT_LACUNARITY,
    }
    .validate()
    .map_err(|_| BaseElevationError::InvalidBasementFrequency)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{base_elevation_fixture, no_interior_relief};

    #[test]
    fn the_basement_reaches_continental_crust_alone() {
        let fixture = base_elevation_fixture();
        let config = BaseElevationConfig {
            dynamic_topography_amplitude: 0.0,
            ..BaseElevationConfig::default()
        };
        let base = fixture.derive(config);
        let curve = fixture.derive(no_interior_relief());

        let mut moved = 0;
        for (cell, (&with, &without)) in base
            .cell_elevations
            .iter()
            .zip(&curve.cell_elevations)
            .enumerate()
        {
            match fixture.age.cell_ages[cell] {
                Some(_) => assert_eq!(with, without, "cell {cell} is oceanic"),
                None => moved += usize::from(with != without),
            }
        }
        assert!(moved > 0, "no continental cell carries a basement");
        assert!(base.diagnostics.basement.minimum < 0.0);
        assert!(base.diagnostics.basement.maximum > 0.0);
        assert!(base.diagnostics.basement.maximum <= config.basement_amplitude);
    }

    #[test]
    fn dynamic_topography_opposes_divergence_everywhere_and_scales_with_its_amplitude() {
        let fixture = base_elevation_fixture();
        // A third of the default amplitude, so that even the peak divergence
        // of this mesh cannot push the deep floor at 0.08 into the clamp and
        // every cell's whole shift is visible. Doubling it stays clear too.
        let config = BaseElevationConfig {
            dynamic_topography_amplitude: 0.01,
            basement_amplitude: 0.0,
            ..BaseElevationConfig::default()
        };
        let base = fixture.derive(config);
        let doubled = fixture.derive(BaseElevationConfig {
            dynamic_topography_amplitude: config.dynamic_topography_amplitude * 2.0,
            ..config
        });
        let curve = fixture.derive(no_interior_relief());
        let divergence = fixture
            .mesh
            .cell_divergence(|direction| fixture.flow.velocity_at(direction));

        let mut sagged = 0;
        let mut swelled = 0;
        for (cell, &divergence) in divergence.iter().enumerate() {
            let shift = base.cell_elevations[cell] - curve.cell_elevations[cell];
            assert!(
                shift * divergence <= 0.0,
                "cell {cell} moved {shift} where the flow diverges by {divergence}"
            );
            assert!(
                (doubled.cell_elevations[cell] - curve.cell_elevations[cell] - 2.0 * shift).abs()
                    < 1.0e-6,
                "cell {cell} does not scale with the amplitude"
            );
            sagged += usize::from(shift < 0.0);
            swelled += usize::from(shift > 0.0);
        }
        assert!(
            sagged > 0 && swelled > 0,
            "{sagged} sagged, {swelled} swelled"
        );
        assert!(base.diagnostics.dynamic_topography.minimum < 0.0);
        assert!(base.diagnostics.dynamic_topography.maximum > 0.0);
    }
}
