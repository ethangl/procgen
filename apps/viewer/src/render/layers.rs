use super::assets::*;
use crate::model::GeneratedWorld;
use bevy::prelude::{GizmoAsset, Vec3};

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

pub(super) struct LayerSpec {
    label: &'static str,
    pub(super) line_width: f32,
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

    pub(super) fn build(self, world: &GeneratedWorld, radius: f32) -> GizmoAsset {
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

    pub(super) fn radius(self) -> f32 {
        DRAW_RADIUS_BASE + self.index() as f32 * DRAW_RADIUS_STEP
    }

    pub fn label(self) -> &'static str {
        self.spec().label
    }

    pub(super) fn spec(self) -> LayerSpec {
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
                wind_asset(world, radius)
            }),
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
