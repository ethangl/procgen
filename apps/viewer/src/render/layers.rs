use super::assets::{
    boundary_asset, delaunay_asset, motion_asset, oceanic_peak_markers, plate_border_asset,
    point_asset, volcanic_arc_markers, voronoi_asset, wind_asset,
};
use super::palette::*;
use super::surfaces::{
    basin_surface_mesh, crust_surface_mesh, insolation_surface_mesh, plate_surface_mesh,
    scalar_surface_mesh, seafloor_age_surface_mesh,
};
use crate::model::GeneratedWorld;
use bevy::prelude::{Component, GizmoAsset, Mesh, Vec3};

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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

const DRAW_RADIUS_BASE: f32 = 1.0;
const PLATE_BORDER_RADIUS_OFFSET: f32 = 0.002;

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

type CellValues = for<'a> fn(&'a GeneratedWorld) -> &'a [f32];

enum LayerSpec {
    Fill {
        label: &'static str,
        surface: SurfaceSource,
        gizmo: Option<GizmoSpec>,
    },
    Overlay {
        label: &'static str,
        gizmo: GizmoSpec,
    },
}

enum SurfaceSource {
    Scalar {
        values: CellValues,
        stops: &'static [(f32, Vec3)],
    },
    Custom(fn(&GeneratedWorld) -> Mesh),
}

#[derive(Clone, Copy)]
struct GizmoSpec {
    kind: OverlayKind,
    line_width: f32,
    build: fn(&GeneratedWorld, f32) -> GizmoAsset,
}

impl LayerSpec {
    fn scalar(label: &'static str, values: CellValues, stops: &'static [(f32, Vec3)]) -> Self {
        Self::Fill {
            label,
            surface: SurfaceSource::Scalar { values, stops },
            gizmo: None,
        }
    }

    fn scalar_with_overlay(
        label: &'static str,
        values: CellValues,
        stops: &'static [(f32, Vec3)],
        kind: OverlayKind,
        line_width: f32,
        overlay: fn(&GeneratedWorld, f32) -> GizmoAsset,
    ) -> Self {
        Self::Fill {
            label,
            surface: SurfaceSource::Scalar { values, stops },
            gizmo: Some(GizmoSpec::new(kind, line_width, overlay)),
        }
    }

    fn surface(
        label: &'static str,
        build: fn(&GeneratedWorld) -> Mesh,
        gizmo: Option<GizmoSpec>,
    ) -> Self {
        Self::Fill {
            label,
            surface: SurfaceSource::Custom(build),
            gizmo,
        }
    }

    fn overlay(
        label: &'static str,
        kind: OverlayKind,
        line_width: f32,
        source: fn(&GeneratedWorld, f32) -> GizmoAsset,
    ) -> Self {
        Self::Overlay {
            label,
            gizmo: GizmoSpec::new(kind, line_width, source),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Fill { label, .. } | Self::Overlay { label, .. } => label,
        }
    }

    fn gizmo(&self) -> Option<GizmoSpec> {
        match self {
            Self::Fill { gizmo, .. } => *gizmo,
            Self::Overlay { gizmo, .. } => Some(*gizmo),
        }
    }

    fn build_surface(self, world: &GeneratedWorld) -> Mesh {
        let Self::Fill { surface, .. } = self else {
            unreachable!("only a surface selection can build a surface mesh");
        };
        match surface {
            SurfaceSource::Scalar { values, stops } => {
                scalar_surface_mesh(world, values(world), stops)
            }
            SurfaceSource::Custom(build) => build(world),
        }
    }
}

impl GizmoSpec {
    const fn new(
        kind: OverlayKind,
        line_width: f32,
        build: fn(&GeneratedWorld, f32) -> GizmoAsset,
    ) -> Self {
        Self {
            kind,
            line_width,
            build,
        }
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
    pub(super) const fn index(self) -> usize {
        self as usize
    }

    pub(super) const fn render_layer(self) -> usize {
        self.index() + 1
    }

    pub fn label(self) -> &'static str {
        self.spec().label()
    }

    pub(super) fn gizmo_line_width(self) -> Option<f32> {
        self.spec().gizmo().map(|gizmo| gizmo.line_width)
    }

    pub(super) fn build_gizmo(self, world: &GeneratedWorld) -> Option<GizmoAsset> {
        self.spec()
            .gizmo()
            .map(|gizmo| (gizmo.build)(world, DRAW_RADIUS_BASE))
    }

    pub(super) fn build_surface(self, world: &GeneratedWorld) -> Mesh {
        self.spec().build_surface(world)
    }

    pub fn is_fill(self) -> bool {
        matches!(self.spec(), LayerSpec::Fill { .. })
    }

    pub fn overlay_kind(self) -> Option<OverlayKind> {
        match self.spec() {
            LayerSpec::Fill { .. } => None,
            LayerSpec::Overlay { gizmo, .. } => Some(gizmo.kind),
        }
    }

    fn spec(self) -> LayerSpec {
        match self {
            Self::Delaunay => {
                LayerSpec::overlay("Delaunay", OverlayKind::Edges, 1.1, |world, radius| {
                    delaunay_asset(&world.voronoi, radius)
                })
            }
            Self::Voronoi => {
                LayerSpec::overlay("Voronoi", OverlayKind::Edges, 1.5, |world, radius| {
                    voronoi_asset(&world.voronoi, radius)
                })
            }
            Self::Plates => LayerSpec::surface(
                "Tectonic plates",
                plate_surface_mesh,
                Some(GizmoSpec::new(OverlayKind::Edges, 2.4, |world, radius| {
                    plate_border_asset(
                        &world.voronoi,
                        &world.plates,
                        radius + PLATE_BORDER_RADIUS_OFFSET,
                    )
                })),
            ),
            Self::Crust => LayerSpec::surface("Crust classes", crust_surface_mesh, None),
            Self::Points => LayerSpec::overlay(
                "Cell centers",
                OverlayKind::Markers,
                1.8,
                |world, radius| point_asset(&world.voronoi, radius),
            ),
            Self::SeafloorAge => {
                LayerSpec::surface("Seafloor age", seafloor_age_surface_mesh, None)
            }
            Self::BaseElevation => LayerSpec::scalar(
                "Base elevation",
                |world| &world.base_elevation.cell_elevations,
                ELEVATION_COLOR_STOPS,
            ),
            Self::Deformation => LayerSpec::scalar(
                "Boundary deformation",
                |world| &world.deformation.cell_deformation,
                DEFORMATION_COLOR_STOPS,
            ),
            Self::Elevation => LayerSpec::scalar(
                "Tectonic elevation",
                |world| &world.elevation.cell_elevations,
                ELEVATION_COLOR_STOPS,
            ),
            Self::GeologicalElevation => LayerSpec::scalar(
                "Geological elevation",
                |world| &world.geological_elevation.cell_elevations,
                ELEVATION_COLOR_STOPS,
            ),
            Self::IsostaticSupport => LayerSpec::scalar(
                "Isostatic support",
                |world| &world.isostasy.cell_support,
                ELEVATION_COLOR_STOPS,
            ),
            Self::IsostaticElevation => LayerSpec::scalar(
                "Adjusted elevation",
                |world| &world.isostasy.cell_elevations,
                ELEVATION_COLOR_STOPS,
            ),
            Self::Insolation => {
                LayerSpec::surface("Daily-mean insolation", insolation_surface_mesh, None)
            }
            Self::CoupledAlbedo => LayerSpec::scalar(
                "Coupled surface albedo",
                |world| &world.cell_albedo,
                ALBEDO_COLOR_STOPS,
            ),
            Self::DailyTemperature => LayerSpec::scalar(
                "Daily effective temperature",
                |world| {
                    &world
                        .radiative_equilibrium
                        .daily_effective_temperature_kelvin
                },
                TEMPERATURE_COLOR_STOPS,
            ),
            Self::AnnualTemperature => LayerSpec::scalar(
                "Annual effective temperature",
                |world| {
                    &world
                        .radiative_equilibrium
                        .annual_effective_temperature_kelvin
                },
                TEMPERATURE_COLOR_STOPS,
            ),
            Self::SeasonalTemperature => LayerSpec::scalar(
                "Seasonal temperature (selected phase)",
                |world| &world.seasonal_thermal.selected_temperature_kelvin,
                TEMPERATURE_COLOR_STOPS,
            ),
            Self::SeasonalMeanTemperature => LayerSpec::scalar(
                "Seasonal temperature (annual mean)",
                |world| &world.seasonal_thermal.annual_mean_temperature_kelvin,
                TEMPERATURE_COLOR_STOPS,
            ),
            Self::SeasonalMinimumTemperature => LayerSpec::scalar(
                "Seasonal temperature (annual minimum)",
                |world| &world.seasonal_thermal.annual_minimum_temperature_kelvin,
                TEMPERATURE_COLOR_STOPS,
            ),
            Self::SeasonalMaximumTemperature => LayerSpec::scalar(
                "Seasonal temperature (annual maximum)",
                |world| &world.seasonal_thermal.annual_maximum_temperature_kelvin,
                TEMPERATURE_COLOR_STOPS,
            ),
            Self::SeasonalTemperatureAmplitude => LayerSpec::scalar(
                "Seasonal temperature amplitude",
                |world| &world.seasonal_thermal.annual_amplitude_kelvin,
                TEMPERATURE_AMPLITUDE_COLOR_STOPS,
            ),
            Self::TemperatureGradient => LayerSpec::scalar(
                "Seasonal temperature gradient",
                |world| {
                    &world
                        .atmospheric_circulation
                        .cell_temperature_gradient_kelvin_per_radian
                },
                TEMPERATURE_GRADIENT_COLOR_STOPS,
            ),
            Self::PressureGradientAcceleration => LayerSpec::scalar(
                "Pressure-gradient acceleration",
                |world| {
                    &world
                        .atmospheric_circulation
                        .cell_pressure_gradient_acceleration_meters_per_second_squared
                },
                PRESSURE_ACCELERATION_COLOR_STOPS,
            ),
            Self::CoriolisParameter => LayerSpec::scalar(
                "Coriolis parameter",
                |world| {
                    &world
                        .atmospheric_circulation
                        .cell_coriolis_parameter_per_second
                },
                CORIOLIS_COLOR_STOPS,
            ),
            Self::TerrainSteering => LayerSpec::scalar(
                "Terrain steering",
                |world| &world.atmospheric_circulation.cell_terrain_steering_fraction,
                FRACTION_COLOR_STOPS,
            ),
            Self::WindSpeed => LayerSpec::scalar(
                "Wind speed",
                |world| {
                    &world
                        .atmospheric_circulation
                        .cell_wind_speed_meters_per_second
                },
                WIND_SPEED_COLOR_STOPS,
            ),
            Self::Wind => LayerSpec::overlay(
                "Wind vectors",
                OverlayKind::Vectors,
                2.6,
                |world, radius| wind_asset(world, radius, WIND_SPEED_COLOR_STOPS),
            ),
            Self::Humidity => LayerSpec::scalar(
                "Atmospheric humidity",
                |world| &world.moisture_transport.cell_humidity_kg_per_m2,
                HUMIDITY_COLOR_STOPS,
            ),
            Self::Precipitation => LayerSpec::scalar(
                "Precipitation",
                |world| {
                    &world
                        .moisture_transport
                        .cell_precipitation_kg_per_m2_per_day
                },
                PRECIPITATION_COLOR_STOPS,
            ),
            Self::SnowCover => LayerSpec::scalar(
                "Snow cover (selected phase)",
                |world| &world.cryosphere.cell_snow_cover_fraction,
                SNOW_COVER_COLOR_STOPS,
            ),
            Self::LandIceCover => LayerSpec::scalar(
                "Land-ice cover",
                |world| &world.cryosphere.cell_land_ice_cover_fraction,
                LAND_ICE_COLOR_STOPS,
            ),
            Self::SeaIceCover => LayerSpec::scalar(
                "Sea-ice cover (selected phase)",
                |world| &world.cryosphere.cell_sea_ice_cover_fraction,
                SEA_ICE_COLOR_STOPS,
            ),
            Self::Hotspots => LayerSpec::scalar(
                "Mantle hotspots",
                |world| &world.hotspots.cell_intensities,
                HOTSPOT_COLOR_STOPS,
            ),
            Self::OceanicPeaks => LayerSpec::scalar_with_overlay(
                "Seamount / abyssal peaks",
                |world| &world.oceanic_peaks.cell_densities,
                OCEANIC_PEAK_COLOR_STOPS,
                OverlayKind::Markers,
                3.8,
                oceanic_peak_markers,
            ),
            Self::VolcanicArcs => LayerSpec::scalar_with_overlay(
                "Volcanic arcs",
                |world| &world.volcanic_arcs.cell_strengths,
                VOLCANIC_ARC_COLOR_STOPS,
                OverlayKind::Markers,
                3.8,
                |world, radius| volcanic_arc_markers(&world.voronoi, &world.volcanic_arcs, radius),
            ),
            Self::Cratons => LayerSpec::scalar(
                "Craton strength",
                |world| &world.cratons.cell_strengths,
                CRATON_COLOR_STOPS,
            ),
            Self::Basins => LayerSpec::surface("Sedimentary basins", basin_surface_mesh, None),
            Self::Boundaries => LayerSpec::overlay(
                "Boundary classes",
                OverlayKind::Edges,
                4.0,
                |world, radius| boundary_asset(&world.voronoi, &world.boundaries, radius),
            ),
            Self::Motion => LayerSpec::overlay(
                "Plate motion",
                OverlayKind::Vectors,
                2.6,
                |world, radius| {
                    motion_asset(&world.voronoi, &world.plates, &world.kinematics, radius)
                },
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
