use super::assets::{
    basin_asset, boundary_asset, crust_asset, delaunay_asset, insolation_asset, motion_asset,
    oceanic_peak_asset, plate_asset, point_asset, scalar_field_asset, seafloor_age_asset,
    volcanic_arc_asset, voronoi_asset, wind_asset,
};
use crate::model::GeneratedWorld;
use bevy::prelude::{GizmoAsset, Vec3};
use procgen_tectonics::SEA_LEVEL;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
const DRAW_RADIUS_STEP: f32 = 0.004;
const FIELD_LINE_WIDTH: f32 = 3.5;
const OVERLAY_LINE_WIDTH: f32 = 3.8;

const DEFORMATION_COLOR_STOPS: [(f32, Vec3); 3] = [
    (-0.5, Vec3::new(0.08, 0.35, 0.95)),
    (0.0, Vec3::new(0.12, 0.12, 0.16)),
    (0.5, Vec3::new(1.0, 0.38, 0.08)),
];

const ELEVATION_COLOR_STOPS: [(f32, Vec3); 5] = [
    (0.0, Vec3::new(0.02, 0.08, 0.3)),
    (SEA_LEVEL, Vec3::new(0.08, 0.65, 0.85)),
    // Duplicate sea-level stop deliberately separates water from land.
    (SEA_LEVEL, Vec3::new(0.16, 0.55, 0.18)),
    (0.75, Vec3::new(0.55, 0.38, 0.16)),
    (1.0, Vec3::new(0.96, 0.96, 0.94)),
];

const TEMPERATURE_COLOR_STOPS: [(f32, Vec3); 6] = [
    (0.0, Vec3::new(0.015, 0.02, 0.08)),
    (180.0, Vec3::new(0.08, 0.16, 0.46)),
    (240.0, Vec3::new(0.12, 0.62, 0.86)),
    (273.15, Vec3::new(0.82, 0.95, 0.92)),
    (320.0, Vec3::new(1.0, 0.68, 0.12)),
    (400.0, Vec3::new(0.86, 0.08, 0.035)),
];

const TEMPERATURE_AMPLITUDE_COLOR_STOPS: [(f32, Vec3); 5] = [
    (0.0, Vec3::new(0.02, 0.035, 0.09)),
    (10.0, Vec3::new(0.08, 0.32, 0.62)),
    (30.0, Vec3::new(0.12, 0.72, 0.72)),
    (75.0, Vec3::new(1.0, 0.68, 0.1)),
    (150.0, Vec3::new(0.9, 0.08, 0.035)),
];

const TEMPERATURE_GRADIENT_COLOR_STOPS: [(f32, Vec3); 4] = [
    (0.0, Vec3::new(0.02, 0.035, 0.09)),
    (25.0, Vec3::new(0.08, 0.4, 0.72)),
    (75.0, Vec3::new(0.2, 0.82, 0.65)),
    (200.0, Vec3::new(1.0, 0.42, 0.08)),
];

const PRESSURE_ACCELERATION_COLOR_STOPS: [(f32, Vec3); 4] = [
    (0.0, Vec3::new(0.02, 0.035, 0.09)),
    (0.001, Vec3::new(0.12, 0.35, 0.8)),
    (0.004, Vec3::new(0.25, 0.82, 0.65)),
    (0.012, Vec3::new(1.0, 0.35, 0.08)),
];

const CORIOLIS_COLOR_STOPS: [(f32, Vec3); 3] = [
    (-0.000_16, Vec3::new(0.15, 0.4, 1.0)),
    (0.0, Vec3::new(0.94, 0.94, 0.94)),
    (0.000_16, Vec3::new(1.0, 0.3, 0.15)),
];

const FRACTION_COLOR_STOPS: [(f32, Vec3); 3] = [
    (0.0, Vec3::new(0.03, 0.05, 0.1)),
    (0.5, Vec3::new(0.16, 0.68, 0.7)),
    (1.0, Vec3::new(1.0, 0.75, 0.15)),
];

const ALBEDO_COLOR_STOPS: [(f32, Vec3); 4] = [
    (0.0, Vec3::new(0.02, 0.035, 0.08)),
    (0.2, Vec3::new(0.12, 0.3, 0.55)),
    (0.6, Vec3::new(0.72, 0.82, 0.88)),
    (1.0, Vec3::new(1.0, 1.0, 1.0)),
];

const WIND_SPEED_COLOR_STOPS: [(f32, Vec3); 5] = [
    (0.0, Vec3::new(0.03, 0.05, 0.1)),
    (10.0, Vec3::new(0.08, 0.38, 0.72)),
    (30.0, Vec3::new(0.12, 0.75, 0.72)),
    (60.0, Vec3::new(1.0, 0.72, 0.12)),
    (100.0, Vec3::new(0.9, 0.1, 0.04)),
];

const HUMIDITY_COLOR_STOPS: [(f32, Vec3); 5] = [
    (0.0, Vec3::new(0.08, 0.045, 0.025)),
    (2.0, Vec3::new(0.55, 0.28, 0.08)),
    (10.0, Vec3::new(0.18, 0.58, 0.62)),
    (30.0, Vec3::new(0.12, 0.35, 0.85)),
    (75.0, Vec3::new(0.72, 0.88, 1.0)),
];

const PRECIPITATION_COLOR_STOPS: [(f32, Vec3); 5] = [
    (0.0, Vec3::new(0.12, 0.06, 0.025)),
    (0.25, Vec3::new(0.75, 0.38, 0.08)),
    (1.0, Vec3::new(0.28, 0.68, 0.42)),
    (4.0, Vec3::new(0.08, 0.48, 0.9)),
    (12.0, Vec3::new(0.72, 0.82, 1.0)),
];

const SNOW_COVER_COLOR_STOPS: [(f32, Vec3); 3] = [
    (0.0, Vec3::new(0.04, 0.055, 0.075)),
    (0.5, Vec3::new(0.58, 0.72, 0.82)),
    (1.0, Vec3::new(0.98, 0.99, 1.0)),
];

const LAND_ICE_COLOR_STOPS: [(f32, Vec3); 3] = [
    (0.0, Vec3::new(0.035, 0.05, 0.075)),
    (0.5, Vec3::new(0.35, 0.72, 0.9)),
    (1.0, Vec3::new(0.82, 0.96, 1.0)),
];

const SEA_ICE_COLOR_STOPS: [(f32, Vec3); 3] = [
    (0.0, Vec3::new(0.015, 0.04, 0.12)),
    (0.5, Vec3::new(0.25, 0.62, 0.82)),
    (1.0, Vec3::new(0.78, 0.94, 0.98)),
];

const HOTSPOT_COLOR_STOPS: [(f32, Vec3); 4] = [
    (0.0, Vec3::new(0.08, 0.06, 0.12)),
    (0.25, Vec3::new(0.55, 0.08, 0.3)),
    (0.65, Vec3::new(1.0, 0.25, 0.05)),
    (1.0, Vec3::new(1.0, 0.95, 0.25)),
];

const CRATON_COLOR_STOPS: [(f32, Vec3); 4] = [
    (0.0, Vec3::new(0.06, 0.08, 0.07)),
    (0.25, Vec3::new(0.18, 0.34, 0.22)),
    (0.65, Vec3::new(0.55, 0.68, 0.32)),
    (1.0, Vec3::new(0.92, 0.86, 0.5)),
];

struct LayerSpec {
    label: &'static str,
    line_width: f32,
    source: Source,
}

type CellValues = for<'a> fn(&'a GeneratedWorld) -> &'a [f32];

enum Source {
    Scalar {
        values: CellValues,
        stops: &'static [(f32, Vec3)],
    },
    Custom(fn(&GeneratedWorld, f32) -> GizmoAsset),
}

impl LayerSpec {
    fn scalar(
        label: &'static str,
        line_width: f32,
        values: CellValues,
        stops: &'static [(f32, Vec3)],
    ) -> Self {
        Self {
            label,
            line_width,
            source: Source::Scalar { values, stops },
        }
    }

    fn custom(
        label: &'static str,
        line_width: f32,
        source: fn(&GeneratedWorld, f32) -> GizmoAsset,
    ) -> Self {
        Self {
            label,
            line_width,
            source: Source::Custom(source),
        }
    }

    fn build(self, world: &GeneratedWorld, radius: f32) -> GizmoAsset {
        match self.source {
            Source::Scalar { values, stops } => {
                scalar_field_asset(&world.voronoi, values(world), stops, radius)
            }
            Source::Custom(build) => build(world, radius),
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
    pub(super) const COUNT: usize = Self::ALL.len();

    pub(super) const fn index(self) -> usize {
        self as usize
    }

    pub(super) const fn render_layer(self) -> usize {
        self.index() + 1
    }

    fn radius(self) -> f32 {
        DRAW_RADIUS_BASE + self.index() as f32 * DRAW_RADIUS_STEP
    }

    pub fn label(self) -> &'static str {
        self.spec().label
    }

    pub(super) fn line_width(self) -> f32 {
        self.spec().line_width
    }

    pub(super) fn build_asset(self, world: &GeneratedWorld) -> GizmoAsset {
        self.spec().build(world, self.radius())
    }

    fn spec(self) -> LayerSpec {
        match self {
            Self::Delaunay => LayerSpec::custom("Delaunay", 1.1, |world, radius| {
                delaunay_asset(&world.voronoi, radius)
            }),
            Self::Voronoi => LayerSpec::custom("Voronoi", 1.5, |world, radius| {
                voronoi_asset(&world.voronoi, radius)
            }),
            Self::Plates => LayerSpec::custom("Tectonic plates", 2.4, |world, radius| {
                // Borders sit just beneath the boundary-class layer so it can overlay them.
                let border_radius = DiagnosticLayer::Boundaries.radius() - DRAW_RADIUS_STEP * 0.5;
                plate_asset(&world.voronoi, &world.plates, radius, border_radius)
            }),
            Self::Crust => LayerSpec::custom("Crust classes", FIELD_LINE_WIDTH, |world, radius| {
                crust_asset(&world.voronoi, &world.plates, &world.crust, radius)
            }),
            Self::Points => LayerSpec::custom("Cell centers", 1.8, |world, radius| {
                point_asset(&world.voronoi, radius)
            }),
            Self::SeafloorAge => {
                LayerSpec::custom("Seafloor age", FIELD_LINE_WIDTH, |world, radius| {
                    seafloor_age_asset(&world.voronoi, &world.seafloor_age, radius)
                })
            }
            Self::BaseElevation => LayerSpec::scalar(
                "Base elevation",
                FIELD_LINE_WIDTH,
                |world| &world.base_elevation.cell_elevations,
                &ELEVATION_COLOR_STOPS,
            ),
            Self::Deformation => LayerSpec::scalar(
                "Boundary deformation",
                FIELD_LINE_WIDTH,
                |world| &world.deformation.cell_deformation,
                &DEFORMATION_COLOR_STOPS,
            ),
            Self::Elevation => LayerSpec::scalar(
                "Tectonic elevation",
                FIELD_LINE_WIDTH,
                |world| &world.elevation.cell_elevations,
                &ELEVATION_COLOR_STOPS,
            ),
            Self::GeologicalElevation => LayerSpec::scalar(
                "Geological elevation",
                FIELD_LINE_WIDTH,
                |world| &world.geological_elevation.cell_elevations,
                &ELEVATION_COLOR_STOPS,
            ),
            Self::IsostaticSupport => LayerSpec::scalar(
                "Isostatic support",
                FIELD_LINE_WIDTH,
                |world| &world.isostasy.cell_support,
                &ELEVATION_COLOR_STOPS,
            ),
            Self::IsostaticElevation => LayerSpec::scalar(
                "Adjusted elevation",
                FIELD_LINE_WIDTH,
                |world| &world.isostasy.cell_elevations,
                &ELEVATION_COLOR_STOPS,
            ),
            Self::Insolation => LayerSpec::custom(
                "Daily-mean insolation",
                FIELD_LINE_WIDTH,
                |world, radius| insolation_asset(&world.voronoi, &world.solar_forcing, radius),
            ),
            Self::CoupledAlbedo => LayerSpec::scalar(
                "Coupled surface albedo",
                FIELD_LINE_WIDTH,
                |world| &world.cell_albedo,
                &ALBEDO_COLOR_STOPS,
            ),
            Self::DailyTemperature => LayerSpec::scalar(
                "Daily effective temperature",
                FIELD_LINE_WIDTH,
                |world| {
                    &world
                        .radiative_equilibrium
                        .daily_effective_temperature_kelvin
                },
                &TEMPERATURE_COLOR_STOPS,
            ),
            Self::AnnualTemperature => LayerSpec::scalar(
                "Annual effective temperature",
                FIELD_LINE_WIDTH,
                |world| {
                    &world
                        .radiative_equilibrium
                        .annual_effective_temperature_kelvin
                },
                &TEMPERATURE_COLOR_STOPS,
            ),
            Self::SeasonalTemperature => LayerSpec::scalar(
                "Seasonal temperature (selected phase)",
                FIELD_LINE_WIDTH,
                |world| &world.seasonal_thermal.selected_temperature_kelvin,
                &TEMPERATURE_COLOR_STOPS,
            ),
            Self::SeasonalMeanTemperature => LayerSpec::scalar(
                "Seasonal temperature (annual mean)",
                FIELD_LINE_WIDTH,
                |world| &world.seasonal_thermal.annual_mean_temperature_kelvin,
                &TEMPERATURE_COLOR_STOPS,
            ),
            Self::SeasonalMinimumTemperature => LayerSpec::scalar(
                "Seasonal temperature (annual minimum)",
                FIELD_LINE_WIDTH,
                |world| &world.seasonal_thermal.annual_minimum_temperature_kelvin,
                &TEMPERATURE_COLOR_STOPS,
            ),
            Self::SeasonalMaximumTemperature => LayerSpec::scalar(
                "Seasonal temperature (annual maximum)",
                FIELD_LINE_WIDTH,
                |world| &world.seasonal_thermal.annual_maximum_temperature_kelvin,
                &TEMPERATURE_COLOR_STOPS,
            ),
            Self::SeasonalTemperatureAmplitude => LayerSpec::scalar(
                "Seasonal temperature amplitude",
                FIELD_LINE_WIDTH,
                |world| &world.seasonal_thermal.annual_amplitude_kelvin,
                &TEMPERATURE_AMPLITUDE_COLOR_STOPS,
            ),
            Self::TemperatureGradient => LayerSpec::scalar(
                "Seasonal temperature gradient",
                FIELD_LINE_WIDTH,
                |world| {
                    &world
                        .atmospheric_circulation
                        .cell_temperature_gradient_kelvin_per_radian
                },
                &TEMPERATURE_GRADIENT_COLOR_STOPS,
            ),
            Self::PressureGradientAcceleration => LayerSpec::scalar(
                "Pressure-gradient acceleration",
                FIELD_LINE_WIDTH,
                |world| {
                    &world
                        .atmospheric_circulation
                        .cell_pressure_gradient_acceleration_meters_per_second_squared
                },
                &PRESSURE_ACCELERATION_COLOR_STOPS,
            ),
            Self::CoriolisParameter => LayerSpec::scalar(
                "Coriolis parameter",
                FIELD_LINE_WIDTH,
                |world| {
                    &world
                        .atmospheric_circulation
                        .cell_coriolis_parameter_per_second
                },
                &CORIOLIS_COLOR_STOPS,
            ),
            Self::TerrainSteering => LayerSpec::scalar(
                "Terrain steering",
                FIELD_LINE_WIDTH,
                |world| &world.atmospheric_circulation.cell_terrain_steering_fraction,
                &FRACTION_COLOR_STOPS,
            ),
            Self::WindSpeed => LayerSpec::scalar(
                "Wind speed",
                FIELD_LINE_WIDTH,
                |world| {
                    &world
                        .atmospheric_circulation
                        .cell_wind_speed_meters_per_second
                },
                &WIND_SPEED_COLOR_STOPS,
            ),
            Self::Wind => LayerSpec::custom("Wind vectors", 2.6, |world, radius| {
                wind_asset(world, radius, &WIND_SPEED_COLOR_STOPS)
            }),
            Self::Humidity => LayerSpec::scalar(
                "Atmospheric humidity",
                FIELD_LINE_WIDTH,
                |world| &world.moisture_transport.cell_humidity_kg_per_m2,
                &HUMIDITY_COLOR_STOPS,
            ),
            Self::Precipitation => LayerSpec::scalar(
                "Precipitation",
                FIELD_LINE_WIDTH,
                |world| {
                    &world
                        .moisture_transport
                        .cell_precipitation_kg_per_m2_per_day
                },
                &PRECIPITATION_COLOR_STOPS,
            ),
            Self::SnowCover => LayerSpec::scalar(
                "Snow cover (selected phase)",
                FIELD_LINE_WIDTH,
                |world| &world.cryosphere.cell_snow_cover_fraction,
                &SNOW_COVER_COLOR_STOPS,
            ),
            Self::LandIceCover => LayerSpec::scalar(
                "Land-ice cover",
                FIELD_LINE_WIDTH,
                |world| &world.cryosphere.cell_land_ice_cover_fraction,
                &LAND_ICE_COLOR_STOPS,
            ),
            Self::SeaIceCover => LayerSpec::scalar(
                "Sea-ice cover (selected phase)",
                FIELD_LINE_WIDTH,
                |world| &world.cryosphere.cell_sea_ice_cover_fraction,
                &SEA_ICE_COLOR_STOPS,
            ),
            Self::Hotspots => LayerSpec::scalar(
                "Mantle hotspots",
                OVERLAY_LINE_WIDTH,
                |world| &world.hotspots.cell_intensities,
                &HOTSPOT_COLOR_STOPS,
            ),
            Self::OceanicPeaks => LayerSpec::custom(
                "Seamount / abyssal peaks",
                OVERLAY_LINE_WIDTH,
                |world, radius| oceanic_peak_asset(&world.voronoi, &world.oceanic_peaks, radius),
            ),
            Self::VolcanicArcs => {
                LayerSpec::custom("Volcanic arcs", OVERLAY_LINE_WIDTH, |world, radius| {
                    volcanic_arc_asset(&world.voronoi, &world.volcanic_arcs, radius)
                })
            }
            Self::Cratons => LayerSpec::scalar(
                "Craton strength",
                OVERLAY_LINE_WIDTH,
                |world| &world.cratons.cell_strengths,
                &CRATON_COLOR_STOPS,
            ),
            Self::Basins => {
                LayerSpec::custom("Sedimentary basins", OVERLAY_LINE_WIDTH, |world, radius| {
                    basin_asset(&world.voronoi, &world.basins.cell_basins, radius)
                })
            }
            Self::Boundaries => LayerSpec::custom("Boundary classes", 4.0, |world, radius| {
                boundary_asset(&world.voronoi, &world.boundaries, radius)
            }),
            Self::Motion => LayerSpec::custom("Plate motion", 2.6, |world, radius| {
                motion_asset(&world.voronoi, &world.plates, &world.kinematics, radius)
            }),
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
