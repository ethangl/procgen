//! Deterministic composition of authoritative coarse fields into terrain-detail controls.
//!
//! This crate only derives per-cell controls and stable sparse stamp inputs. It does not
//! interpolate, bake, evaluate noise, or mutate any upstream field.

use procgen_core::Vec3;
use procgen_geology::{
    CratonField, HotspotField, IsostaticAdjustment, OceanicPeakField, OceanicPeakKind,
    SedimentaryBasinField, VolcanicArcField,
};
use procgen_sphere_mesh::SphereMesh;
use procgen_tectonics::{BoundaryClass, BoundaryClassification, SeafloorAge};
use std::fmt;

/// Coefficients for converting completed coarse fields into normalized detail controls.
///
/// Baselines and positive weights are in `[0, 1]`; signed deltas are in `[-1, 1]`.
/// `boundary_strength_saturation` is in upstream velocity units and must be positive.
/// `abyssal_age_saturation` is in mesh hops and must be positive. Every composed channel
/// is clamped to `[0, 1]` after its terms are applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainControlConfig {
    /// Normalized-elevation amplitude away from geological modifiers.
    pub base_detail_amplitude: f32,
    /// Normalized-elevation amplitude change at full craton strength.
    pub craton_amplitude_delta: f32,
    /// Normalized-elevation amplitude added at full volcanic-arc strength.
    pub volcanic_arc_amplitude: f32,
    /// Normalized-elevation amplitude added at saturated convergent motion.
    pub convergent_boundary_amplitude: f32,
    /// Normalized-elevation amplitude added at saturated divergent motion.
    pub divergent_boundary_amplitude: f32,
    /// Normalized-elevation amplitude added at saturated transform motion.
    pub transform_boundary_amplitude: f32,
    /// Normalized-elevation amplitude change inside a retained basin.
    pub basin_amplitude_delta: f32,
    /// Upstream velocity magnitude mapped to full boundary strength.
    pub boundary_strength_saturation: f32,
    /// Unitless ridged-noise blend away from geological modifiers.
    pub base_ridge_weight: f32,
    /// Unitless ridged-noise blend added at saturated convergent motion.
    pub convergent_ridge_weight: f32,
    /// Unitless ridged-noise blend added at full volcanic-arc strength.
    pub volcanic_arc_ridge_weight: f32,
    /// Unitless gain per octave away from geological modifiers.
    pub base_octave_gain: f32,
    /// Unitless octave-gain change at full craton strength.
    pub craton_octave_gain_delta: f32,
    /// Unitless octave-gain change inside a retained basin.
    pub basin_octave_gain_delta: f32,
    /// Normalized-elevation abyssal amplitude at age zero.
    pub maximum_abyssal_amplitude: f32,
    /// Mesh-hop age at which abyssal amplitude reaches zero.
    pub abyssal_age_saturation: usize,
    /// Unitless multiplier for hotspot candidate strength.
    pub hotspot_stamp_scale: f32,
    /// Unitless multiplier for volcanic-arc peak strength.
    pub volcanic_arc_stamp_scale: f32,
    /// Unitless multiplier for oceanic peak strength.
    pub oceanic_peak_stamp_scale: f32,
}

impl Default for TerrainControlConfig {
    fn default() -> Self {
        Self {
            base_detail_amplitude: 0.08,
            craton_amplitude_delta: -0.04,
            volcanic_arc_amplitude: 0.35,
            convergent_boundary_amplitude: 0.30,
            divergent_boundary_amplitude: 0.15,
            transform_boundary_amplitude: 0.10,
            basin_amplitude_delta: -0.04,
            boundary_strength_saturation: 1.0,
            base_ridge_weight: 0.0,
            convergent_ridge_weight: 0.8,
            volcanic_arc_ridge_weight: 0.7,
            base_octave_gain: 0.5,
            craton_octave_gain_delta: -0.12,
            basin_octave_gain_delta: -0.18,
            maximum_abyssal_amplitude: 0.12,
            abyssal_age_saturation: 8,
            hotspot_stamp_scale: 1.0,
            volcanic_arc_stamp_scale: 0.8,
            oceanic_peak_stamp_scale: 1.0,
        }
    }
}

impl TerrainControlConfig {
    pub fn validate(self) -> Result<(), TerrainControlError> {
        let unit = [
            self.base_detail_amplitude,
            self.volcanic_arc_amplitude,
            self.convergent_boundary_amplitude,
            self.divergent_boundary_amplitude,
            self.transform_boundary_amplitude,
            self.base_ridge_weight,
            self.convergent_ridge_weight,
            self.volcanic_arc_ridge_weight,
            self.base_octave_gain,
            self.maximum_abyssal_amplitude,
            self.hotspot_stamp_scale,
            self.volcanic_arc_stamp_scale,
            self.oceanic_peak_stamp_scale,
        ];
        let signed = [
            self.craton_amplitude_delta,
            self.basin_amplitude_delta,
            self.craton_octave_gain_delta,
            self.basin_octave_gain_delta,
        ];
        if unit
            .into_iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
            || signed
                .into_iter()
                .any(|value| !value.is_finite() || !(-1.0..=1.0).contains(&value))
            || !self.boundary_strength_saturation.is_finite()
            || self.boundary_strength_saturation <= 0.0
            || self.abyssal_age_saturation == 0
        {
            return Err(TerrainControlError::InvalidConfig);
        }
        Ok(())
    }
}

/// Borrowed completed-stage inputs. No viewer-owned aggregate or snapshot is required.
#[derive(Clone, Copy, Debug)]
pub struct TerrainControlInputs<'a> {
    pub mesh: &'a SphereMesh,
    pub isostasy: &'a IsostaticAdjustment,
    pub cratons: &'a CratonField,
    pub volcanic_arcs: &'a VolcanicArcField,
    pub boundaries: &'a BoundaryClassification,
    pub basins: &'a SedimentaryBasinField,
    pub seafloor_age: &'a SeafloorAge,
    pub hotspots: &'a HotspotField,
    pub oceanic_peaks: &'a OceanicPeakField,
}

/// Per-cell controls consumed by later interpolation and height-function slices.
///
/// `base_elevation` is the final isostatically adjusted normalized elevation and is copied
/// unchanged. `detail_amplitude` and `abyssal_amplitude` are normalized-elevation offsets;
/// `ridge_weight` and `octave_gain` are unitless. All four are clamped to `[0, 1]`, and
/// `octave_gain` is the multiplicative gain used between successive noise octaves. Composition
/// order is fixed: baselines, cratons, volcanic arcs, convergent/divergent/transform boundaries,
/// then basins.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TerrainCellControls {
    pub base_elevation: f32,
    pub detail_amplitude: f32,
    pub ridge_weight: f32,
    pub octave_gain: f32,
    pub abyssal_amplitude: f32,
}

/// Stable type order used when multiple stamps overlap: hotspot, volcanic arc, seamount,
/// then abyssal hill. Within a type, upstream source order is retained.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TerrainStampKind {
    Hotspot,
    VolcanicArc,
    OceanicSeamount,
    OceanicAbyssalHill,
}

/// Sparse input for a later terrain stamp evaluator.
///
/// `position` uses the mesh's surface-coordinate units. `strength` is unitless and clamped
/// to `[0, 1]`. `source_index` is the stable upstream index, with volcanic-arc peaks indexed by
/// their flattened segment/peak order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainStampInput {
    pub cell: usize,
    pub kind: TerrainStampKind,
    pub source_index: usize,
    pub position: Vec3,
    pub strength: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerrainControls {
    pub cells: Vec<TerrainCellControls>,
    /// Sorted by cell, kind, then source index, defining deterministic overlap evaluation.
    pub stamps: Vec<TerrainStampInput>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainControlError {
    InvalidConfig,
    InvalidMesh,
    InvalidInput(TerrainInputField),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainInputField {
    Isostasy,
    Cratons,
    VolcanicArcs,
    Boundaries,
    Basins,
    SeafloorAge,
    Hotspots,
    OceanicPeaks,
}

impl fmt::Display for TerrainControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str("terrain-control configuration is invalid"),
            Self::InvalidMesh => formatter.write_str("terrain-control mesh is invalid"),
            Self::InvalidInput(field) => {
                write!(formatter, "terrain-control {field:?} input is invalid")
            }
        }
    }
}

impl std::error::Error for TerrainControlError {}

/// Composes controls without mutating or rederiving any upstream field.
pub fn compose_terrain_controls(
    inputs: TerrainControlInputs<'_>,
    config: TerrainControlConfig,
) -> Result<TerrainControls, TerrainControlError> {
    validate_inputs(inputs, config)?;

    let mut boundary_strengths = vec![[0.0_f32; 3]; inputs.mesh.cell_count()];
    for (edge_index, edge) in inputs.mesh.edges.iter().enumerate() {
        let slot = match inputs.boundaries.edge_classes[edge_index] {
            BoundaryClass::Convergent => Some(0),
            BoundaryClass::Divergent => Some(1),
            BoundaryClass::Transform => Some(2),
            BoundaryClass::Interior => None,
        };
        let Some(slot) = slot else { continue };
        let strength = (inputs.boundaries.strength(edge_index).unwrap_or(0.0)
            / config.boundary_strength_saturation)
            .clamp(0.0, 1.0);
        for &cell in &edge.cells {
            boundary_strengths[cell][slot] = boundary_strengths[cell][slot].max(strength);
        }
    }

    let cells = (0..inputs.mesh.cell_count())
        .map(|cell| {
            let craton = inputs.cratons.cell_strengths[cell];
            let arc = inputs.volcanic_arcs.cell_strengths[cell];
            let basin = f32::from(inputs.basins.cell_basins[cell].is_some());
            let [convergent, divergent, transform] = boundary_strengths[cell];
            let detail_amplitude = (config.base_detail_amplitude
                + craton * config.craton_amplitude_delta
                + arc * config.volcanic_arc_amplitude
                + convergent * config.convergent_boundary_amplitude
                + divergent * config.divergent_boundary_amplitude
                + transform * config.transform_boundary_amplitude
                + basin * config.basin_amplitude_delta)
                .clamp(0.0, 1.0);
            let ridge_weight = (config.base_ridge_weight
                + convergent * config.convergent_ridge_weight
                + arc * config.volcanic_arc_ridge_weight)
                .clamp(0.0, 1.0);
            let octave_gain = (config.base_octave_gain
                + craton * config.craton_octave_gain_delta
                + basin * config.basin_octave_gain_delta)
                .clamp(0.0, 1.0);
            let abyssal_amplitude = inputs.seafloor_age.cell_ages[cell].map_or(0.0, |age| {
                let youth = 1.0
                    - (age.min(config.abyssal_age_saturation) as f32
                        / config.abyssal_age_saturation as f32);
                youth * config.maximum_abyssal_amplitude
            });
            TerrainCellControls {
                base_elevation: inputs.isostasy.cell_elevations[cell],
                detail_amplitude,
                ridge_weight,
                octave_gain,
                abyssal_amplitude,
            }
        })
        .collect();

    Ok(TerrainControls {
        cells,
        stamps: compose_stamps(inputs, config),
    })
}

fn compose_stamps(
    inputs: TerrainControlInputs<'_>,
    config: TerrainControlConfig,
) -> Vec<TerrainStampInput> {
    let mut stamps = Vec::new();
    stamps.extend(
        inputs
            .hotspots
            .hotspots
            .iter()
            .enumerate()
            .map(|(index, hotspot)| TerrainStampInput {
                cell: hotspot.source_cell,
                kind: TerrainStampKind::Hotspot,
                source_index: index,
                position: hotspot.mantle_position,
                strength: (hotspot.trail[0].intensity * config.hotspot_stamp_scale).clamp(0.0, 1.0),
            }),
    );
    let mut volcanic_peak_index = 0;
    for segment in &inputs.volcanic_arcs.segments {
        for &cell in &segment.peaks {
            let strength = segment
                .arc_cells
                .iter()
                .find(|arc| arc.cell == cell)
                .map_or(0.0, |arc| arc.strength);
            stamps.push(TerrainStampInput {
                cell,
                kind: TerrainStampKind::VolcanicArc,
                source_index: volcanic_peak_index,
                position: inputs.mesh.cell_centers[cell],
                strength: (strength * config.volcanic_arc_stamp_scale).clamp(0.0, 1.0),
            });
            volcanic_peak_index += 1;
        }
    }
    stamps.extend(
        inputs
            .oceanic_peaks
            .peaks
            .iter()
            .enumerate()
            .map(|(index, peak)| TerrainStampInput {
                cell: peak.cell,
                kind: match peak.kind {
                    OceanicPeakKind::Seamount => TerrainStampKind::OceanicSeamount,
                    OceanicPeakKind::AbyssalHill => TerrainStampKind::OceanicAbyssalHill,
                },
                source_index: index,
                position: peak.position,
                strength: (peak.strength * config.oceanic_peak_stamp_scale).clamp(0.0, 1.0),
            }),
    );
    stamps.sort_by(|left, right| {
        (left.cell, left.kind, left.source_index).cmp(&(right.cell, right.kind, right.source_index))
    });
    stamps
}

fn validate_inputs(
    inputs: TerrainControlInputs<'_>,
    config: TerrainControlConfig,
) -> Result<(), TerrainControlError> {
    config.validate()?;
    inputs
        .mesh
        .validate()
        .map_err(|_| TerrainControlError::InvalidMesh)?;
    inputs
        .isostasy
        .validate(inputs.mesh)
        .map_err(|_| TerrainControlError::InvalidInput(TerrainInputField::Isostasy))?;
    inputs
        .cratons
        .validate(inputs.mesh)
        .map_err(|_| TerrainControlError::InvalidInput(TerrainInputField::Cratons))?;
    inputs
        .volcanic_arcs
        .validate(inputs.mesh)
        .map_err(|_| TerrainControlError::InvalidInput(TerrainInputField::VolcanicArcs))?;
    inputs
        .boundaries
        .validate(inputs.mesh)
        .map_err(|_| TerrainControlError::InvalidInput(TerrainInputField::Boundaries))?;
    inputs
        .basins
        .validate(inputs.mesh)
        .map_err(|_| TerrainControlError::InvalidInput(TerrainInputField::Basins))?;
    inputs
        .seafloor_age
        .validate(inputs.mesh)
        .map_err(|_| TerrainControlError::InvalidInput(TerrainInputField::SeafloorAge))?;
    inputs
        .hotspots
        .validate(inputs.mesh)
        .map_err(|_| TerrainControlError::InvalidInput(TerrainInputField::Hotspots))?;
    inputs
        .oceanic_peaks
        .validate(inputs.mesh)
        .map_err(|_| TerrainControlError::InvalidInput(TerrainInputField::OceanicPeaks))?;

    validate_values(inputs)
}

fn validate_values(inputs: TerrainControlInputs<'_>) -> Result<(), TerrainControlError> {
    let unit_values = |values: &[f32]| {
        values
            .iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
    };
    if !unit_values(&inputs.isostasy.cell_elevations) {
        return Err(TerrainControlError::InvalidInput(
            TerrainInputField::Isostasy,
        ));
    }
    if !unit_values(&inputs.cratons.cell_strengths) {
        return Err(TerrainControlError::InvalidInput(
            TerrainInputField::Cratons,
        ));
    }
    if !unit_values(&inputs.volcanic_arcs.cell_strengths) {
        return Err(TerrainControlError::InvalidInput(
            TerrainInputField::VolcanicArcs,
        ));
    }
    if inputs.volcanic_arcs.segments.iter().any(|segment| {
        segment
            .arc_cells
            .iter()
            .any(|arc| !arc.strength.is_finite() || !(0.0..=1.0).contains(&arc.strength))
            || segment
                .peaks
                .iter()
                .any(|peak| !segment.arc_cells.iter().any(|arc| arc.cell == *peak))
    }) {
        return Err(TerrainControlError::InvalidInput(
            TerrainInputField::VolcanicArcs,
        ));
    }
    if inputs
        .boundaries
        .edge_normal_speeds
        .iter()
        .flatten()
        .any(|v| !v.is_finite())
        || inputs
            .boundaries
            .edge_shear
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0)
    {
        return Err(TerrainControlError::InvalidInput(
            TerrainInputField::Boundaries,
        ));
    }
    if inputs
        .hotspots
        .cell_intensities
        .iter()
        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        || inputs.hotspots.hotspots.iter().any(|hotspot| {
            !hotspot.mantle_position.is_finite()
                || hotspot.trail.first().map(|point| point.cell) != Some(hotspot.source_cell)
                || hotspot.trail.iter().any(|point| {
                    !point.intensity.is_finite() || !(0.0..=1.0).contains(&point.intensity)
                })
        })
    {
        return Err(TerrainControlError::InvalidInput(
            TerrainInputField::Hotspots,
        ));
    }
    if !unit_values(&inputs.oceanic_peaks.cell_densities)
        || inputs.oceanic_peaks.peaks.iter().any(|peak| {
            !peak.position.is_finite()
                || !peak.strength.is_finite()
                || !(0.0..=1.0).contains(&peak.strength)
                || !peak.height.is_finite()
                || peak.height < 0.0
                || inputs.oceanic_peaks.cell_kinds[peak.cell] != Some(peak.kind)
                || inputs.oceanic_peaks.cell_densities[peak.cell] != peak.strength
        })
    {
        return Err(TerrainControlError::InvalidInput(
            TerrainInputField::OceanicPeaks,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use procgen_geology::{
        CratonDiagnostics, Hotspot, HotspotDiagnostics, IsostaticAdjustmentDiagnostics,
        OceanicPeak, OceanicPeakDiagnostics, SedimentaryBasin, SedimentaryBasinDiagnostics,
        VolcanicArcCell, VolcanicArcDiagnostics, VolcanicArcSegment,
    };
    use procgen_sphere::{FibonacciConfig, fibonacci_sphere};
    use procgen_sphere_mesh::build_sphere_mesh;
    use procgen_tectonics::SeafloorAgeDiagnostics;

    struct Fixture {
        mesh: SphereMesh,
        isostasy: IsostaticAdjustment,
        cratons: CratonField,
        arcs: VolcanicArcField,
        boundaries: BoundaryClassification,
        basins: SedimentaryBasinField,
        ages: SeafloorAge,
        hotspots: HotspotField,
        peaks: OceanicPeakField,
    }

    impl Fixture {
        fn new() -> Self {
            let mesh = build_sphere_mesh(
                fibonacci_sphere(FibonacciConfig {
                    count: 16,
                    jitter: 0.0,
                    seed: 1,
                })
                .unwrap(),
                1.0,
            )
            .unwrap();
            let cells = mesh.cell_count();
            let edges = mesh.edge_count();
            Self {
                isostasy: IsostaticAdjustment {
                    cell_support: vec![0.5; cells],
                    cell_elevations: vec![0.5; cells],
                    diagnostics: IsostaticAdjustmentDiagnostics::default(),
                },
                cratons: CratonField {
                    cell_strengths: vec![0.0; cells],
                    diagnostics: CratonDiagnostics::default(),
                },
                arcs: VolcanicArcField {
                    segments: vec![],
                    cell_strengths: vec![0.0; cells],
                    cell_segments: vec![None; cells],
                    diagnostics: VolcanicArcDiagnostics::default(),
                },
                boundaries: BoundaryClassification {
                    edge_classes: vec![BoundaryClass::Interior; edges],
                    edge_normal_speeds: vec![[0.0; 2]; edges],
                    edge_shear: vec![0.0; edges],
                },
                basins: SedimentaryBasinField {
                    cell_basins: vec![None; cells],
                    basins: vec![],
                    diagnostics: SedimentaryBasinDiagnostics::default(),
                },
                ages: SeafloorAge {
                    cell_ages: vec![None; cells],
                    diagnostics: SeafloorAgeDiagnostics::default(),
                },
                hotspots: HotspotField {
                    hotspots: vec![],
                    cell_intensities: vec![0.0; cells],
                    cell_hotspots: vec![None; cells],
                    diagnostics: HotspotDiagnostics::default(),
                },
                peaks: OceanicPeakField {
                    cell_densities: vec![0.0; cells],
                    cell_kinds: vec![None; cells],
                    peaks: vec![],
                    diagnostics: OceanicPeakDiagnostics::default(),
                },
                mesh,
            }
        }

        fn inputs(&self) -> TerrainControlInputs<'_> {
            TerrainControlInputs {
                mesh: &self.mesh,
                isostasy: &self.isostasy,
                cratons: &self.cratons,
                volcanic_arcs: &self.arcs,
                boundaries: &self.boundaries,
                basins: &self.basins,
                seafloor_age: &self.ages,
                hotspots: &self.hotspots,
                oceanic_peaks: &self.peaks,
            }
        }

        fn compose(&self) -> Result<TerrainControls, TerrainControlError> {
            compose_terrain_controls(self.inputs(), TerrainControlConfig::default())
        }
    }

    #[test]
    fn neutral_inputs_copy_base_and_use_baselines_deterministically() {
        let fixture = Fixture::new();
        let first = fixture.compose().unwrap();
        assert_eq!(first, fixture.compose().unwrap());
        assert!(first.stamps.is_empty());
        assert!(first.cells.iter().all(|cell| *cell
            == TerrainCellControls {
                base_elevation: 0.5,
                detail_amplitude: 0.08,
                ridge_weight: 0.0,
                octave_gain: 0.5,
                abyssal_amplitude: 0.0,
            }));
    }

    #[test]
    fn every_dense_source_has_its_isolated_effect() {
        let config = TerrainControlConfig::default();

        let mut craton = Fixture::new();
        craton.cratons.cell_strengths[0] = 1.0;
        let cell = craton.compose().unwrap().cells[0];
        assert_eq!(
            cell.detail_amplitude,
            config.base_detail_amplitude + config.craton_amplitude_delta
        );
        assert_eq!(
            cell.octave_gain,
            config.base_octave_gain + config.craton_octave_gain_delta
        );

        let mut arc = Fixture::new();
        arc.arcs.cell_strengths[0] = 1.0;
        let cell = arc.compose().unwrap().cells[0];
        assert_eq!(
            cell.detail_amplitude,
            config.base_detail_amplitude + config.volcanic_arc_amplitude
        );
        assert_eq!(cell.ridge_weight, config.volcanic_arc_ridge_weight);

        for (class, slot, amplitude) in [
            (
                BoundaryClass::Convergent,
                0,
                config.convergent_boundary_amplitude,
            ),
            (
                BoundaryClass::Divergent,
                1,
                config.divergent_boundary_amplitude,
            ),
            (
                BoundaryClass::Transform,
                2,
                config.transform_boundary_amplitude,
            ),
        ] {
            let mut boundary = Fixture::new();
            let edge = boundary.mesh.edges[0];
            boundary.boundaries.edge_classes[0] = class;
            boundary.boundaries.edge_normal_speeds[0] = if slot == 0 {
                [0.5, 0.5]
            } else if slot == 1 {
                [-0.5, -0.5]
            } else {
                [0.0; 2]
            };
            boundary.boundaries.edge_shear[0] = if slot == 2 { 1.0 } else { 0.0 };
            for cell in edge.cells {
                let controls = boundary.compose().unwrap().cells[cell];
                assert_eq!(
                    controls.detail_amplitude,
                    config.base_detail_amplitude + amplitude
                );
                assert_eq!(
                    controls.ridge_weight,
                    if slot == 0 {
                        config.convergent_ridge_weight
                    } else {
                        0.0
                    }
                );
            }
        }

        let mut basin = Fixture::new();
        basin.basins.basins.push(SedimentaryBasin {
            root_cell: 0,
            cell_count: 1,
            ocean_perimeter_fraction: 0.0,
            minimum_elevation: 0.5,
        });
        basin.basins.cell_basins[0] = Some(0);
        let cell = basin.compose().unwrap().cells[0];
        assert_eq!(
            cell.detail_amplitude,
            config.base_detail_amplitude + config.basin_amplitude_delta
        );
        assert_eq!(
            cell.octave_gain,
            config.base_octave_gain + config.basin_octave_gain_delta
        );

        let mut age = Fixture::new();
        age.ages.cell_ages[0] = Some(0);
        age.ages.cell_ages[1] = Some(config.abyssal_age_saturation / 2);
        age.ages.cell_ages[2] = Some(config.abyssal_age_saturation);
        let cells = age.compose().unwrap().cells;
        assert_eq!(cells[0].abyssal_amplitude, config.maximum_abyssal_amplitude);
        assert_eq!(
            cells[1].abyssal_amplitude,
            config.maximum_abyssal_amplitude * 0.5
        );
        assert_eq!(cells[2].abyssal_amplitude, 0.0);
    }

    #[test]
    fn channels_are_bounded() {
        let mut fixture = Fixture::new();
        fixture.cratons.cell_strengths.fill(1.0);
        fixture.arcs.cell_strengths.fill(1.0);
        fixture.ages.cell_ages.fill(Some(0));
        for edge in 0..fixture.mesh.edge_count() {
            fixture.boundaries.edge_classes[edge] = BoundaryClass::Convergent;
            fixture.boundaries.edge_normal_speeds[edge] = [10.0, 10.0];
        }
        let result = compose_terrain_controls(
            fixture.inputs(),
            TerrainControlConfig {
                base_detail_amplitude: 1.0,
                craton_amplitude_delta: 1.0,
                volcanic_arc_amplitude: 1.0,
                convergent_boundary_amplitude: 1.0,
                base_ridge_weight: 1.0,
                convergent_ridge_weight: 1.0,
                volcanic_arc_ridge_weight: 1.0,
                base_octave_gain: 1.0,
                craton_octave_gain_delta: 1.0,
                ..TerrainControlConfig::default()
            },
        )
        .unwrap();
        assert!(
            result
                .cells
                .iter()
                .flat_map(|cell| [
                    cell.base_elevation,
                    cell.detail_amplitude,
                    cell.ridge_weight,
                    cell.octave_gain,
                    cell.abyssal_amplitude
                ])
                .all(|value| (0.0..=1.0).contains(&value))
        );
    }

    #[test]
    fn sparse_stamps_have_stable_overlap_order() {
        let mut fixture = Fixture::new();
        let cell = 0;
        let position = fixture.mesh.cell_centers[cell];
        fixture.hotspots.hotspots.push(Hotspot {
            mantle_position: position,
            source_cell: cell,
            plate: 0,
            trail: vec![procgen_geology::HotspotTrailCell {
                cell,
                intensity: 0.7,
            }],
        });
        fixture.hotspots.cell_intensities[cell] = 0.7;
        fixture.hotspots.cell_hotspots[cell] = Some(0);
        fixture.arcs.segments.push(VolcanicArcSegment {
            overriding_plate: 0,
            boundary_edges: vec![],
            boundary_cells: vec![],
            arc_cells: vec![VolcanicArcCell {
                cell,
                strength: 0.6,
            }],
            peaks: vec![cell],
            inland_depth: 0,
        });
        fixture.arcs.cell_strengths[cell] = 0.6;
        fixture.arcs.cell_segments[cell] = Some(0);
        fixture.peaks.peaks.extend([
            OceanicPeak {
                cell: 1,
                kind: OceanicPeakKind::AbyssalHill,
                position: fixture.mesh.cell_centers[1],
                strength: 0.4,
                height: 0.1,
            },
            OceanicPeak {
                cell,
                kind: OceanicPeakKind::Seamount,
                position,
                strength: 0.5,
                height: 0.5,
            },
        ]);
        fixture.peaks.cell_kinds[cell] = Some(OceanicPeakKind::Seamount);
        fixture.peaks.cell_densities[cell] = 0.5;
        fixture.peaks.cell_kinds[1] = Some(OceanicPeakKind::AbyssalHill);
        fixture.peaks.cell_densities[1] = 0.4;
        let result = fixture.compose().unwrap();
        assert_eq!(
            result
                .stamps
                .iter()
                .map(|stamp| stamp.kind)
                .collect::<Vec<_>>(),
            vec![
                TerrainStampKind::Hotspot,
                TerrainStampKind::VolcanicArc,
                TerrainStampKind::OceanicSeamount,
                TerrainStampKind::OceanicAbyssalHill
            ]
        );
        assert!(
            result
                .stamps
                .iter()
                .all(|stamp| (0.0..=1.0).contains(&stamp.strength))
        );
        assert_eq!(
            result
                .stamps
                .iter()
                .map(|stamp| stamp.strength)
                .collect::<Vec<_>>(),
            vec![
                0.7,
                0.6 * TerrainControlConfig::default().volcanic_arc_stamp_scale,
                0.5,
                0.4
            ]
        );
    }

    #[test]
    fn rejects_invalid_config_lengths_references_and_values() {
        let fixture = Fixture::new();
        assert_eq!(
            compose_terrain_controls(
                fixture.inputs(),
                TerrainControlConfig {
                    boundary_strength_saturation: 0.0,
                    ..TerrainControlConfig::default()
                }
            ),
            Err(TerrainControlError::InvalidConfig)
        );

        let mut short = Fixture::new();
        short.cratons.cell_strengths.pop();
        assert_eq!(
            short.compose(),
            Err(TerrainControlError::InvalidInput(
                TerrainInputField::Cratons
            ))
        );

        let mut reference = Fixture::new();
        reference.basins.cell_basins[0] = Some(0);
        assert_eq!(
            reference.compose(),
            Err(TerrainControlError::InvalidInput(TerrainInputField::Basins))
        );

        let mut value = Fixture::new();
        value.isostasy.cell_elevations[0] = f32::NAN;
        assert_eq!(
            value.compose(),
            Err(TerrainControlError::InvalidInput(
                TerrainInputField::Isostasy
            ))
        );
    }
}
