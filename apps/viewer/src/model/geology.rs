use super::{GenerationTimings, TectonicsWorld};
use procgen_cubesphere::control_face_resolution;
use procgen_geology::{
    CratonField, CratonFieldConfig, GeologicalElevation, GeologicalElevationConfig,
    GeologicalElevationInputs, HotspotField, HotspotFieldConfig, IsostaticAdjustment,
    IsostaticAdjustmentConfig, IsostaticAdjustmentInputs, OceanicPeakField, OceanicPeakFieldConfig,
    SedimentaryBasinField, SedimentaryBasinFieldConfig, VolcanicArcField, VolcanicArcFieldConfig,
    compose_geological_elevation, derive_craton_field, derive_isostatic_adjustment,
    derive_oceanic_peak_field, derive_sedimentary_basin_field, derive_volcanic_arc_field,
    generate_hotspot_field,
};
use procgen_terrain::{
    TerrainControlBake, TerrainControlConfig, TerrainControlInputs, TerrainControls,
    bake_terrain_controls, compose_terrain_controls,
};
use std::error::Error;

/// Settings for the geology phase, which refines tectonic elevation and bakes
/// the terrain controls the detailed surface reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeologySettings {
    pub hotspots: HotspotFieldConfig,
    pub oceanic_peaks: OceanicPeakFieldConfig,
    pub volcanic_arcs: VolcanicArcFieldConfig,
    pub cratons: CratonFieldConfig,
    pub basins: SedimentaryBasinFieldConfig,
    pub geological_elevation: GeologicalElevationConfig,
    pub isostasy: IsostaticAdjustmentConfig,
    pub terrain_controls: TerrainControlConfig,
}

impl Default for GeologySettings {
    fn default() -> Self {
        Self {
            hotspots: HotspotFieldConfig::new(7),
            oceanic_peaks: OceanicPeakFieldConfig::new(7),
            volcanic_arcs: VolcanicArcFieldConfig::default(),
            cratons: CratonFieldConfig::default(),
            basins: SedimentaryBasinFieldConfig::default(),
            geological_elevation: GeologicalElevationConfig::default(),
            isostasy: IsostaticAdjustmentConfig::default(),
            terrain_controls: TerrainControlConfig::default(),
        }
    }
}

/// Results of the geology phase.
pub struct GeologyWorld {
    pub hotspots: HotspotField,
    pub oceanic_peaks: OceanicPeakField,
    pub volcanic_arcs: VolcanicArcField,
    pub cratons: CratonField,
    pub basins: SedimentaryBasinField,
    pub geological_elevation: GeologicalElevation,
    pub isostasy: IsostaticAdjustment,
    pub terrain_controls: TerrainControls,
    pub terrain_control_bake: TerrainControlBake,
    pub timings: GenerationTimings,
    pub config: GeologySettings,
}

impl GeologyWorld {
    pub fn generate(
        tectonics: &TectonicsWorld,
        config: GeologySettings,
    ) -> Result<Self, Box<dyn Error>> {
        let mesh = &tectonics.voronoi;
        let mut timings = GenerationTimings::default();
        let hotspots = timings.record("Mantle hotspots", || {
            generate_hotspot_field(
                mesh,
                &tectonics.plates,
                tectonics.cell_crust(),
                &tectonics.kinematics,
                config.hotspots,
            )
        })?;
        let oceanic_peaks = timings.record("Oceanic peaks", || {
            derive_oceanic_peak_field(
                mesh,
                &hotspots,
                &tectonics.seafloor_age,
                config.oceanic_peaks,
            )
        })?;
        let volcanic_arcs = timings.record("Volcanic arcs", || {
            derive_volcanic_arc_field(
                mesh,
                &tectonics.plates,
                tectonics.cell_crust(),
                &tectonics.boundaries,
                config.volcanic_arcs,
            )
        })?;
        let cratons = timings.record("Cratons", || {
            derive_craton_field(
                mesh,
                &tectonics.plates,
                tectonics.cell_crust(),
                &tectonics.elevation,
                config.cratons,
            )
        })?;
        let basins = timings.record("Sedimentary basins", || {
            derive_sedimentary_basin_field(
                mesh,
                tectonics.cell_crust(),
                &tectonics.elevation,
                config.basins,
            )
        })?;
        let geological_elevation = timings.record("Geological elevation", || {
            compose_geological_elevation(
                mesh,
                GeologicalElevationInputs {
                    tectonic_elevation: &tectonics.elevation,
                    hotspots: &hotspots,
                    volcanic_arcs: &volcanic_arcs,
                    cratons: &cratons,
                    basins: &basins,
                    base_elevation: &tectonics.base_elevation,
                },
                config.geological_elevation,
            )
        })?;
        let isostasy = timings.record("Isostatic adjustment", || {
            derive_isostatic_adjustment(
                mesh,
                IsostaticAdjustmentInputs {
                    crust: tectonics.cell_crust(),
                    boundaries: &tectonics.boundaries,
                    cratons: &cratons,
                    basins: &basins,
                    geological_elevation: &geological_elevation,
                },
                config.isostasy,
            )
        })?;
        let terrain_controls = timings.record("Terrain controls", || {
            compose_terrain_controls(
                mesh,
                TerrainControlInputs {
                    isostasy: &isostasy,
                    cratons: &cratons,
                    volcanic_arcs: &volcanic_arcs,
                    boundaries: &tectonics.boundaries,
                    basins: &basins,
                    seafloor_age: &tectonics.seafloor_age,
                    hotspots: &hotspots,
                    oceanic_peaks: &oceanic_peaks,
                },
                config.terrain_controls,
            )
        })?;
        let terrain_control_bake = timings.record("Terrain control bake", || {
            bake_terrain_controls(mesh, &terrain_controls)
        })?;

        Ok(Self {
            hotspots,
            oceanic_peaks,
            volcanic_arcs,
            cratons,
            basins,
            geological_elevation,
            isostasy,
            terrain_controls,
            terrain_control_bake,
            timings,
            config,
        })
    }

    pub fn validate(&self, tectonics: &TectonicsWorld) -> Result<(), Box<dyn Error>> {
        let mesh = &tectonics.voronoi;
        let plate_count = tectonics.plates.plate_count;
        if self.config.hotspots.hotspot_count != self.hotspots.hotspots.len()
            || self
                .hotspots
                .hotspots
                .iter()
                .any(|hotspot| hotspot.plate >= plate_count)
            || self
                .volcanic_arcs
                .segments
                .iter()
                .any(|segment| segment.overriding_plate >= plate_count)
            || self.terrain_control_bake.resolution() != control_face_resolution(mesh.cell_count())?
        {
            return Err("geology results do not match the tectonics they were built on".into());
        }

        self.hotspots.validate(mesh)?;
        self.oceanic_peaks.validate(mesh)?;
        self.volcanic_arcs.validate(mesh)?;
        self.cratons.validate(mesh)?;
        self.basins.validate(mesh)?;
        self.geological_elevation.validate(mesh)?;
        self.isostasy.validate(mesh)?;
        self.terrain_controls.validate(mesh)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{geology_settings, tectonics_settings, tectonics_world};

    #[test]
    fn geology_runs_on_a_tectonics_result_alone() {
        let tectonics = tectonics_world(tectonics_settings(128, 11));
        let geology = GeologyWorld::generate(&tectonics, geology_settings(11)).unwrap();

        geology.validate(&tectonics).unwrap();
        assert_eq!(
            geology.terrain_controls.cells.len(),
            tectonics.voronoi.cell_count()
        );
        assert_eq!(
            geology.terrain_control_bake.resolution(),
            control_face_resolution(tectonics.voronoi.cell_count()).unwrap()
        );
        assert!(
            geology
                .timings
                .stages()
                .iter()
                .any(|stage| stage.label == "Terrain control bake")
        );
    }
}
