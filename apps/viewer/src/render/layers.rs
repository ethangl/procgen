use super::assets::{
    boundary_asset, delaunay_asset, motion_asset, oceanic_peak_markers, plate_border_asset,
    point_asset, volcanic_arc_markers, voronoi_asset, wind_asset,
};
use super::palette::{
    ALBEDO_COLOR_STOPS, CORIOLIS_COLOR_STOPS, CRATON_COLOR_STOPS, DEFORMATION_COLOR_STOPS,
    FRACTION_COLOR_STOPS, HUMIDITY_COLOR_STOPS, LAND_ICE_COLOR_STOPS, OCEANIC_PEAK_COLOR_STOPS,
    PRECIPITATION_COLOR_STOPS, PRESSURE_ACCELERATION_COLOR_STOPS, SEA_ICE_COLOR_STOPS,
    SNOW_COVER_COLOR_STOPS, TEMPERATURE_AMPLITUDE_COLOR_STOPS, TEMPERATURE_COLOR_STOPS,
    TEMPERATURE_GRADIENT_COLOR_STOPS, VOLCANIC_ARC_COLOR_STOPS, WIND_SPEED_COLOR_STOPS,
    elevation_color_stops, opaque_color, piecewise_lerp,
};
use super::surfaces::{
    basin_colors, cell_surface_mesh, crust_colors, hotspot_colors, insolation_colors, plate_colors,
    seafloor_age_colors,
};
use crate::model::{ClimateWorld, GeneratedWorld, GeologyWorld, Phase, TectonicsWorld};
use bevy::prelude::{Color, Component, GizmoAsset, Mesh, Vec3};

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum DiagnosticLayer {
    Delaunay,
    Voronoi,
    Plates,
    Crust,
    Points,
    SeafloorAge,
    BaseElevation,
    Deformation,
    Elevation,
    GeologicalElevation,
    IsostaticSupport,
    IsostaticElevation,
    Insolation,
    CoupledAlbedo,
    DailyTemperature,
    AnnualTemperature,
    SeasonalTemperature,
    SeasonalMeanTemperature,
    SeasonalMinimumTemperature,
    SeasonalMaximumTemperature,
    SeasonalTemperatureAmplitude,
    TemperatureGradient,
    PressureGradientAcceleration,
    CoriolisParameter,
    TerrainSteering,
    WindSpeed,
    Wind,
    Humidity,
    Precipitation,
    SnowCover,
    LandIceCover,
    SeaIceCover,
    Hotspots,
    OceanicPeaks,
    VolcanicArcs,
    Cratons,
    Basins,
    Boundaries,
    Motion,
}

/// Declaration order is also the outward draw order for visible overlays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverlayKind {
    Edges,
    Markers,
    Vectors,
}

impl OverlayKind {
    pub const ALL: &[Self] = &[Self::Edges, Self::Markers, Self::Vectors];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Edges => "Edges",
            Self::Markers => "Markers",
            Self::Vectors => "Vectors",
        }
    }
}

/// A per-cell field read straight from the phase that produced it.
#[derive(Clone, Copy)]
pub(super) enum ValueSource {
    Tectonics(for<'a> fn(&'a TectonicsWorld) -> &'a [f32]),
    Geology(for<'a> fn(&'a GeologyWorld) -> &'a [f32]),
    Climate(for<'a> fn(&'a ClimateWorld) -> &'a [f32]),
}

/// Something built from a phase's results. The mesh lives on the tectonics
/// result, so the later phases build against it as well.
pub(super) enum BuildSource<T> {
    Tectonics(fn(&TectonicsWorld) -> T),
    Geology(fn(&TectonicsWorld, &GeologyWorld) -> T),
    Climate(fn(&TectonicsWorld, &ClimateWorld) -> T),
}

impl ValueSource {
    const fn phase(self) -> Phase {
        match self {
            Self::Tectonics(_) => Phase::Tectonics,
            Self::Geology(_) => Phase::Geology,
            Self::Climate(_) => Phase::Climate,
        }
    }

    fn read(self, world: &GeneratedWorld) -> Option<&[f32]> {
        match self {
            Self::Tectonics(read) => Some(read(world.tectonics()?)),
            Self::Geology(read) => Some(read(world.geology()?)),
            Self::Climate(read) => Some(read(world.climate()?)),
        }
    }
}

impl<T> BuildSource<T> {
    const fn phase(self) -> Phase {
        match self {
            Self::Tectonics(_) => Phase::Tectonics,
            Self::Geology(_) => Phase::Geology,
            Self::Climate(_) => Phase::Climate,
        }
    }

    fn build(self, world: &GeneratedWorld) -> Option<T> {
        let tectonics = world.tectonics()?;
        match self {
            Self::Tectonics(build) => Some(build(tectonics)),
            Self::Geology(build) => Some(build(tectonics, world.geology()?)),
            Self::Climate(build) => Some(build(tectonics, world.climate()?)),
        }
    }
}

// Only the function pointers are stored, so the sources copy regardless of
// what they build.
impl<T> Clone for BuildSource<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for BuildSource<T> {}

/// Where a scalar layer's colour stops come from. Elevation stops are built
/// against the generated world's sea-level datum, so they cannot be static.
#[derive(Clone, Copy)]
pub(super) enum ScalarPalette {
    Fixed(&'static [(f32, Vec3)]),
    Elevation,
}

impl ScalarPalette {
    fn stops(self, sea_level: f32) -> Vec<(f32, Vec3)> {
        match self {
            Self::Fixed(stops) => stops.to_vec(),
            Self::Elevation => elevation_color_stops(sea_level).to_vec(),
        }
    }
}

enum LayerSpec {
    Fill {
        label: &'static str,
        surface: SurfaceSource,
        gizmo: Option<GizmoSpec>,
    },
    Overlay {
        label: &'static str,
        kind: OverlayKind,
        gizmo: GizmoSpec,
    },
}

#[derive(Component, Clone, Copy)]
pub(super) struct GizmoSpec {
    line_width: f32,
    source: BuildSource<GizmoAsset>,
}

impl LayerSpec {
    fn scalar(label: &'static str, values: ValueSource, stops: &'static [(f32, Vec3)]) -> Self {
        Self::palette(label, values, ScalarPalette::Fixed(stops))
    }

    fn palette(label: &'static str, values: ValueSource, palette: ScalarPalette) -> Self {
        Self::Fill {
            label,
            surface: SurfaceSource::Scalar { values, palette },
            gizmo: None,
        }
    }

    fn scalar_with_overlay(
        label: &'static str,
        values: ValueSource,
        stops: &'static [(f32, Vec3)],
        line_width: f32,
        overlay: BuildSource<GizmoAsset>,
    ) -> Self {
        Self::Fill {
            label,
            surface: SurfaceSource::Scalar {
                values,
                palette: ScalarPalette::Fixed(stops),
            },
            gizmo: Some(GizmoSpec::new(line_width, overlay)),
        }
    }

    fn colors(
        label: &'static str,
        colors: BuildSource<Vec<Color>>,
        gizmo: Option<GizmoSpec>,
    ) -> Self {
        Self::Fill {
            label,
            surface: SurfaceSource::Colors(colors),
            gizmo,
        }
    }

    fn overlay(
        label: &'static str,
        kind: OverlayKind,
        line_width: f32,
        source: BuildSource<GizmoAsset>,
    ) -> Self {
        Self::Overlay {
            label,
            kind,
            gizmo: GizmoSpec::new(line_width, source),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Fill { label, .. } | Self::Overlay { label, .. } => label,
        }
    }

    fn phase(&self) -> Phase {
        match self {
            Self::Fill { surface, .. } => surface.phase(),
            Self::Overlay { gizmo, .. } => gizmo.phase(),
        }
    }

    fn gizmo(&self) -> Option<GizmoSpec> {
        match self {
            Self::Fill { gizmo, .. } => *gizmo,
            Self::Overlay { gizmo, .. } => Some(*gizmo),
        }
    }
}

impl GizmoSpec {
    const fn new(line_width: f32, source: BuildSource<GizmoAsset>) -> Self {
        Self { line_width, source }
    }

    pub(super) const fn line_width(self) -> f32 {
        self.line_width
    }

    const fn phase(self) -> Phase {
        self.source.phase()
    }

    pub(super) fn build(self, world: &GeneratedWorld) -> GizmoAsset {
        self.source.build(world).unwrap_or_default()
    }
}

#[derive(Clone, Copy)]
pub(super) enum SurfaceSource {
    Scalar {
        values: ValueSource,
        palette: ScalarPalette,
    },
    Colors(BuildSource<Vec<Color>>),
}

impl SurfaceSource {
    const fn phase(self) -> Phase {
        match self {
            Self::Scalar { values, .. } => values.phase(),
            Self::Colors(colors) => colors.phase(),
        }
    }

    pub(super) fn build(self, world: &GeneratedWorld, relief_exaggeration: f32) -> Option<Mesh> {
        let sea_level = world.sea_level()?;
        let colors = match self {
            Self::Scalar { values, palette } => {
                let stops = palette.stops(sea_level);
                values
                    .read(world)?
                    .iter()
                    .map(|&value| opaque_color(piecewise_lerp(value, &stops)))
                    .collect()
            }
            Self::Colors(colors) => colors.build(world)?,
        };
        let (_, elevations) = world.surface_elevations()?;
        Some(cell_surface_mesh(
            &world.tectonics()?.voronoi,
            &colors,
            elevations,
            sea_level,
            relief_exaggeration,
        ))
    }
}

impl DiagnosticLayer {
    pub const ALL: &[Self] = &[
        Self::Delaunay,
        Self::Voronoi,
        Self::Plates,
        Self::Crust,
        Self::Points,
        Self::SeafloorAge,
        Self::BaseElevation,
        Self::Deformation,
        Self::Elevation,
        Self::GeologicalElevation,
        Self::IsostaticSupport,
        Self::IsostaticElevation,
        Self::Insolation,
        Self::CoupledAlbedo,
        Self::DailyTemperature,
        Self::AnnualTemperature,
        Self::SeasonalTemperature,
        Self::SeasonalMeanTemperature,
        Self::SeasonalMinimumTemperature,
        Self::SeasonalMaximumTemperature,
        Self::SeasonalTemperatureAmplitude,
        Self::TemperatureGradient,
        Self::PressureGradientAcceleration,
        Self::CoriolisParameter,
        Self::TerrainSteering,
        Self::WindSpeed,
        Self::Wind,
        Self::Humidity,
        Self::Precipitation,
        Self::SnowCover,
        Self::LandIceCover,
        Self::SeaIceCover,
        Self::Hotspots,
        Self::OceanicPeaks,
        Self::VolcanicArcs,
        Self::Cratons,
        Self::Basins,
        Self::Boundaries,
        Self::Motion,
    ];
    pub(super) const COUNT: usize = Self::ALL.len();
    pub(super) const fn index(self) -> usize {
        self as usize
    }

    pub(super) const fn render_layer(self) -> usize {
        self.index() + 1
    }

    pub fn label(self) -> &'static str {
        self.spec().label()
    }

    /// The phase whose results the layer draws.
    pub fn phase(self) -> Phase {
        self.spec().phase()
    }

    /// The elevation fill a phase contributes, which is what the displaced
    /// surface shows by default. Climate contributes no elevation.
    pub(super) const fn elevation_fill(phase: Phase) -> Option<Self> {
        match phase {
            Phase::Tectonics => Some(Self::Elevation),
            Phase::Geology => Some(Self::IsostaticElevation),
            Phase::Climate => None,
        }
    }

    pub(super) fn gizmo(self) -> Option<GizmoSpec> {
        self.spec().gizmo()
    }

    pub fn is_fill(self) -> bool {
        self.surface().is_some()
    }

    pub(super) fn surface(self) -> Option<SurfaceSource> {
        match self.spec() {
            LayerSpec::Fill { surface, .. } => Some(surface),
            LayerSpec::Overlay { .. } => None,
        }
    }

    pub fn overlay_kind(self) -> Option<OverlayKind> {
        match self.spec() {
            LayerSpec::Fill { .. } => None,
            LayerSpec::Overlay { kind, .. } => Some(kind),
        }
    }

    pub(super) fn depth_order(self) -> Option<(OverlayKind, usize)> {
        self.overlay_kind().map(|kind| (kind, self.index()))
    }

    fn spec(self) -> LayerSpec {
        use BuildSource::Tectonics as TectonicsBuild;
        use BuildSource::{Climate as ClimateBuild, Geology as GeologyBuild};
        use ValueSource::{Climate, Geology, Tectonics};
        match self {
            Self::Delaunay => LayerSpec::overlay(
                "Delaunay",
                OverlayKind::Edges,
                1.1,
                TectonicsBuild(delaunay_asset),
            ),
            Self::Voronoi => LayerSpec::overlay(
                "Voronoi",
                OverlayKind::Edges,
                1.5,
                TectonicsBuild(voronoi_asset),
            ),
            Self::Plates => LayerSpec::colors(
                "Tectonic plates",
                TectonicsBuild(plate_colors),
                Some(GizmoSpec::new(2.4, TectonicsBuild(plate_border_asset))),
            ),
            Self::Crust => LayerSpec::colors("Crust classes", TectonicsBuild(crust_colors), None),
            Self::Points => LayerSpec::overlay(
                "Cell centers",
                OverlayKind::Markers,
                1.8,
                TectonicsBuild(point_asset),
            ),
            Self::SeafloorAge => {
                LayerSpec::colors("Seafloor age", TectonicsBuild(seafloor_age_colors), None)
            }
            Self::BaseElevation => LayerSpec::palette(
                "Base elevation",
                Tectonics(|world| &world.base_elevation.cell_elevations),
                ScalarPalette::Elevation,
            ),
            Self::Deformation => LayerSpec::scalar(
                "Boundary deformation",
                Tectonics(|world| &world.deformation.cell_deformation),
                DEFORMATION_COLOR_STOPS,
            ),
            Self::Elevation => LayerSpec::palette(
                "Tectonic elevation",
                Tectonics(|world| &world.elevation.cell_elevations),
                ScalarPalette::Elevation,
            ),
            Self::GeologicalElevation => LayerSpec::palette(
                "Geological elevation",
                Geology(|world| &world.geological_elevation.cell_elevations),
                ScalarPalette::Elevation,
            ),
            Self::IsostaticSupport => LayerSpec::palette(
                "Isostatic support",
                Geology(|world| &world.isostasy.cell_support),
                ScalarPalette::Elevation,
            ),
            Self::IsostaticElevation => LayerSpec::palette(
                "Adjusted elevation",
                Geology(|world| &world.isostasy.cell_elevations),
                ScalarPalette::Elevation,
            ),
            Self::Insolation => LayerSpec::colors(
                "Daily-mean insolation",
                ClimateBuild(insolation_colors),
                None,
            ),
            Self::CoupledAlbedo => LayerSpec::scalar(
                "Coupled surface albedo",
                Climate(|world| &world.cell_albedo),
                ALBEDO_COLOR_STOPS,
            ),
            Self::DailyTemperature => LayerSpec::scalar(
                "Daily effective temperature",
                Climate(|world| {
                    &world
                        .radiative_equilibrium
                        .daily_effective_temperature_kelvin
                }),
                TEMPERATURE_COLOR_STOPS,
            ),
            Self::AnnualTemperature => LayerSpec::scalar(
                "Annual effective temperature",
                Climate(|world| {
                    &world
                        .radiative_equilibrium
                        .annual_effective_temperature_kelvin
                }),
                TEMPERATURE_COLOR_STOPS,
            ),
            Self::SeasonalTemperature => LayerSpec::scalar(
                "Seasonal temperature (selected phase)",
                Climate(|world| &world.seasonal_thermal.selected_temperature_kelvin),
                TEMPERATURE_COLOR_STOPS,
            ),
            Self::SeasonalMeanTemperature => LayerSpec::scalar(
                "Seasonal temperature (annual mean)",
                Climate(|world| &world.seasonal_thermal.annual_mean_temperature_kelvin),
                TEMPERATURE_COLOR_STOPS,
            ),
            Self::SeasonalMinimumTemperature => LayerSpec::scalar(
                "Seasonal temperature (annual minimum)",
                Climate(|world| &world.seasonal_thermal.annual_minimum_temperature_kelvin),
                TEMPERATURE_COLOR_STOPS,
            ),
            Self::SeasonalMaximumTemperature => LayerSpec::scalar(
                "Seasonal temperature (annual maximum)",
                Climate(|world| &world.seasonal_thermal.annual_maximum_temperature_kelvin),
                TEMPERATURE_COLOR_STOPS,
            ),
            Self::SeasonalTemperatureAmplitude => LayerSpec::scalar(
                "Seasonal temperature amplitude",
                Climate(|world| &world.seasonal_thermal.annual_amplitude_kelvin),
                TEMPERATURE_AMPLITUDE_COLOR_STOPS,
            ),
            Self::TemperatureGradient => LayerSpec::scalar(
                "Seasonal temperature gradient",
                Climate(|world| {
                    &world
                        .atmospheric_circulation
                        .cell_temperature_gradient_kelvin_per_radian
                }),
                TEMPERATURE_GRADIENT_COLOR_STOPS,
            ),
            Self::PressureGradientAcceleration => LayerSpec::scalar(
                "Pressure-gradient acceleration",
                Climate(|world| {
                    &world
                        .atmospheric_circulation
                        .cell_pressure_gradient_acceleration_meters_per_second_squared
                }),
                PRESSURE_ACCELERATION_COLOR_STOPS,
            ),
            Self::CoriolisParameter => LayerSpec::scalar(
                "Coriolis parameter",
                Climate(|world| {
                    &world
                        .atmospheric_circulation
                        .cell_coriolis_parameter_per_second
                }),
                CORIOLIS_COLOR_STOPS,
            ),
            Self::TerrainSteering => LayerSpec::scalar(
                "Terrain steering",
                Climate(|world| &world.atmospheric_circulation.cell_terrain_steering_fraction),
                FRACTION_COLOR_STOPS,
            ),
            Self::WindSpeed => LayerSpec::scalar(
                "Wind speed",
                Climate(|world| {
                    &world
                        .atmospheric_circulation
                        .cell_wind_speed_meters_per_second
                }),
                WIND_SPEED_COLOR_STOPS,
            ),
            Self::Wind => LayerSpec::overlay(
                "Wind vectors",
                OverlayKind::Vectors,
                2.6,
                ClimateBuild(wind_asset),
            ),
            Self::Humidity => LayerSpec::scalar(
                "Atmospheric humidity",
                Climate(|world| &world.moisture_transport.cell_humidity_kg_per_m2),
                HUMIDITY_COLOR_STOPS,
            ),
            Self::Precipitation => LayerSpec::scalar(
                "Precipitation",
                Climate(|world| {
                    &world
                        .moisture_transport
                        .cell_precipitation_kg_per_m2_per_day
                }),
                PRECIPITATION_COLOR_STOPS,
            ),
            Self::SnowCover => LayerSpec::scalar(
                "Snow cover (selected phase)",
                Climate(|world| &world.cryosphere.cell_snow_cover_fraction),
                SNOW_COVER_COLOR_STOPS,
            ),
            Self::LandIceCover => LayerSpec::scalar(
                "Land-ice cover",
                Climate(|world| &world.cryosphere.cell_land_ice_cover_fraction),
                LAND_ICE_COLOR_STOPS,
            ),
            Self::SeaIceCover => LayerSpec::scalar(
                "Sea-ice cover (selected phase)",
                Climate(|world| &world.cryosphere.cell_sea_ice_cover_fraction),
                SEA_ICE_COLOR_STOPS,
            ),
            Self::Hotspots => {
                LayerSpec::colors("Mantle hotspots", GeologyBuild(hotspot_colors), None)
            }
            Self::OceanicPeaks => LayerSpec::scalar_with_overlay(
                "Seamount / abyssal peaks",
                Geology(|world| &world.oceanic_peaks.cell_densities),
                OCEANIC_PEAK_COLOR_STOPS,
                3.8,
                GeologyBuild(oceanic_peak_markers),
            ),
            Self::VolcanicArcs => LayerSpec::scalar_with_overlay(
                "Volcanic arcs",
                Geology(|world| &world.volcanic_arcs.cell_strengths),
                VOLCANIC_ARC_COLOR_STOPS,
                3.8,
                GeologyBuild(volcanic_arc_markers),
            ),
            Self::Cratons => LayerSpec::scalar(
                "Craton strength",
                Geology(|world| &world.cratons.cell_strengths),
                CRATON_COLOR_STOPS,
            ),
            Self::Basins => {
                LayerSpec::colors("Sedimentary basins", GeologyBuild(basin_colors), None)
            }
            Self::Boundaries => LayerSpec::overlay(
                "Boundary classes",
                OverlayKind::Edges,
                4.0,
                TectonicsBuild(boundary_asset),
            ),
            Self::Motion => LayerSpec::overlay(
                "Plate motion",
                OverlayKind::Vectors,
                2.6,
                TectonicsBuild(motion_asset),
            ),
        }
    }
}

const _: () = {
    let mut index = 0;
    while index < DiagnosticLayer::ALL.len() {
        assert!(
            DiagnosticLayer::ALL[index] as usize == index,
            "ALL must be in declaration order"
        );
        index += 1;
    }
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Fixture;

    #[test]
    fn every_layer_builds_from_a_complete_world() {
        let world = Fixture::new(128, 31).into_world();

        for &layer in DiagnosticLayer::ALL {
            if let Some(surface) = layer.surface() {
                assert!(
                    surface.build(&world, 0.01).is_some(),
                    "{} fill",
                    layer.label()
                );
            }
            if let Some(gizmo) = layer.gizmo() {
                assert!(
                    gizmo.source.build(&world).is_some(),
                    "{} gizmo",
                    layer.label()
                );
            }
        }
    }

    #[test]
    fn each_layer_reads_the_phase_its_fill_and_overlay_agree_on() {
        for &layer in DiagnosticLayer::ALL {
            if let (Some(surface), Some(gizmo)) = (layer.surface(), layer.gizmo()) {
                assert_eq!(surface.phase(), gizmo.phase(), "{}", layer.label());
            }
        }
    }
}
