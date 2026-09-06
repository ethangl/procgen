use crate::{camera::ViewerCamera, model::GeneratedWorld};
use bevy::{camera::visibility::RenderLayers, gizmos::config::GizmoLineConfig, prelude::*};
use procgen_climate::{CALM_WIND_SPEED_METERS_PER_SECOND, SolarForcing};
use procgen_core::Vec3 as SphereVec3;
use procgen_geology::{OceanicPeakField, OceanicPeakKind, VolcanicArcField};
use procgen_sphere_mesh::{SphereMesh, VoronoiEdge};
use procgen_tectonics::{
    BoundaryClass, BoundaryClassification, CrustClass, CrustClassification, PlateKinematics,
    PlatePartition, SEA_LEVEL, SeafloorAge,
};

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
const MAXIMUM_VECTOR_COUNT: usize = 256;
const DEFORMATION_COLOR_STOPS: [(f32, Vec3); 3] = [
    (-0.5, Vec3::new(0.08, 0.35, 0.95)),
    (0.0, Vec3::new(0.12, 0.12, 0.16)),
    (0.5, Vec3::new(1.0, 0.38, 0.08)),
];
const SEAFLOOR_AGE_COLOR_STOPS: [(f32, Vec3); 3] = [
    (0.0, Vec3::new(0.35, 0.95, 1.0)),
    (0.5, Vec3::new(0.08, 0.4, 0.8)),
    (1.0, Vec3::new(0.015, 0.05, 0.2)),
];
const ELEVATION_COLOR_STOPS: [(f32, Vec3); 5] = [
    (0.0, Vec3::new(0.02, 0.08, 0.3)),
    (SEA_LEVEL, Vec3::new(0.08, 0.65, 0.85)),
    // Duplicate sea-level stop deliberately separates water from land.
    (SEA_LEVEL, Vec3::new(0.16, 0.55, 0.18)),
    (0.75, Vec3::new(0.55, 0.38, 0.16)),
    (1.0, Vec3::new(0.96, 0.96, 0.94)),
];
const INSOLATION_COLOR_STOPS: [(f32, Vec3); 5] = [
    (0.0, Vec3::new(0.015, 0.02, 0.08)),
    (0.2, Vec3::new(0.08, 0.18, 0.5)),
    (0.45, Vec3::new(0.12, 0.65, 0.82)),
    (0.7, Vec3::new(1.0, 0.72, 0.12)),
    (1.0, Vec3::new(1.0, 0.98, 0.78)),
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
const WIND_SPEED_COLOR_STOPS: [(f32, Vec3); 5] = [
    (0.0, Vec3::new(0.03, 0.05, 0.1)),
    (10.0, Vec3::new(0.08, 0.38, 0.72)),
    (30.0, Vec3::new(0.12, 0.75, 0.72)),
    (60.0, Vec3::new(1.0, 0.72, 0.12)),
    (100.0, Vec3::new(0.9, 0.1, 0.04)),
];
const HOTSPOT_COLOR_STOPS: [(f32, Vec3); 4] = [
    (0.0, Vec3::new(0.08, 0.06, 0.12)),
    (0.25, Vec3::new(0.55, 0.08, 0.3)),
    (0.65, Vec3::new(1.0, 0.25, 0.05)),
    (1.0, Vec3::new(1.0, 0.95, 0.25)),
];
const OCEANIC_PEAK_COLOR_STOPS: [(f32, Vec3); 4] = [
    (0.0, Vec3::new(0.02, 0.06, 0.12)),
    (0.25, Vec3::new(0.05, 0.35, 0.52)),
    (0.65, Vec3::new(0.18, 0.78, 0.72)),
    (1.0, Vec3::new(0.95, 0.9, 0.42)),
];
const SEAMOUNT_PEAK_COLOR: Vec3 = Vec3::new(1.0, 0.42, 0.08);
const ABYSSAL_HILL_PEAK_COLOR: Vec3 = Vec3::new(0.55, 0.92, 1.0);
const CELL_MARKER_SCALE: f32 = 0.32;
const MINIMUM_CELL_MARKER_SIZE: f32 = 0.003;
const MAXIMUM_CELL_MARKER_SIZE: f32 = 0.012;
const VOLCANIC_ARC_COLOR_STOPS: [(f32, Vec3); 4] = [
    (0.0, Vec3::new(0.08, 0.055, 0.04)),
    (0.25, Vec3::new(0.55, 0.12, 0.02)),
    (0.65, Vec3::new(1.0, 0.42, 0.03)),
    (1.0, Vec3::new(1.0, 0.95, 0.28)),
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
    const COUNT: usize = Self::ALL.len();

    const fn index(self) -> usize {
        self as usize
    }

    const fn render_layer(self) -> usize {
        self.index() + 1
    }

    fn radius(self) -> f32 {
        DRAW_RADIUS_BASE + self.index() as f32 * DRAW_RADIUS_STEP
    }

    pub fn label(self) -> &'static str {
        self.spec().label
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

fn insolation_asset(mesh: &SphereMesh, forcing: &SolarForcing, radius: f32) -> GizmoAsset {
    let maximum = forcing.diagnostics.daily_mean.maximum;
    let reciprocal = if maximum > 0.0 { maximum.recip() } else { 0.0 };
    let normalized = forcing
        .daily_mean_insolation
        .iter()
        .map(|value| value * reciprocal)
        .collect::<Vec<_>>();
    scalar_field_asset(mesh, &normalized, &INSOLATION_COLOR_STOPS, radius)
}

fn oceanic_peak_asset(mesh: &SphereMesh, field: &OceanicPeakField, radius: f32) -> GizmoAsset {
    let mut asset = scalar_field_asset(
        mesh,
        &field.cell_densities,
        &OCEANIC_PEAK_COLOR_STOPS,
        radius,
    );
    let base_size = cell_marker_size(mesh);
    for peak in &field.peaks {
        let color = match peak.kind {
            OceanicPeakKind::Seamount => SEAMOUNT_PEAK_COLOR,
            OceanicPeakKind::AbyssalHill => ABYSSAL_HILL_PEAK_COLOR,
        };
        let position = to_bevy(peak.position.normalized()) * radius;
        add_cross_marker(
            &mut asset,
            position,
            base_size * (0.5 + peak.height.clamp(0.0, 1.5)),
            opaque_color(color),
        );
    }
    asset
}

fn basin_asset(mesh: &SphereMesh, cell_basins: &[Option<usize>], radius: f32) -> GizmoAsset {
    voronoi_edge_asset(mesh, |_, edge| {
        let basins = edge.cells.map(|cell| cell_basins[cell]);
        let color = match basins {
            [None, None] => Color::srgba(0.045, 0.065, 0.075, 0.7),
            [Some(id), _] | [None, Some(id)] => id_color(id),
        };
        Some((radius, color))
    })
}

fn volcanic_arc_asset(mesh: &SphereMesh, field: &VolcanicArcField, radius: f32) -> GizmoAsset {
    let mut asset = scalar_field_asset(
        mesh,
        &field.cell_strengths,
        &VOLCANIC_ARC_COLOR_STOPS,
        radius,
    );
    let marker_size = cell_marker_size(mesh);
    let marker_color = VOLCANIC_ARC_COLOR_STOPS[VOLCANIC_ARC_COLOR_STOPS.len() - 1].1;
    for &peak_cell in field.segments.iter().flat_map(|segment| &segment.peaks) {
        let position = to_bevy(mesh.cell_centers[peak_cell].normalized()) * radius;
        add_cross_marker(
            &mut asset,
            position,
            marker_size,
            opaque_color(marker_color),
        );
    }
    asset
}

fn seafloor_age_asset(mesh: &SphereMesh, age: &SeafloorAge, radius: f32) -> GizmoAsset {
    let maximum_age = age.diagnostics.summary.maximum.max(1.0);
    voronoi_edge_asset(mesh, |_, edge| {
        let ages = edge.cells.map(|cell| age.cell_ages[cell]);
        let color = match ages {
            [None, None] => Color::srgba(0.18, 0.16, 0.14, 0.75),
            [Some(_), None] | [None, Some(_)] => Color::srgba(0.96, 0.96, 1.0, 1.0),
            [Some(left), Some(right)] => scalar_edge_color(
                [left as f32 / maximum_age, right as f32 / maximum_age],
                &SEAFLOOR_AGE_COLOR_STOPS,
            ),
        };
        Some((radius, color))
    })
}

#[derive(Resource)]
pub struct LayerSettings {
    visible: [bool; DiagnosticLayer::COUNT],
}

impl LayerSettings {
    pub fn is_visible(&self, layer: DiagnosticLayer) -> bool {
        self.visible[layer.index()]
    }

    pub fn set_visible(&mut self, layer: DiagnosticLayer, visible: bool) {
        self.visible[layer.index()] = visible;
    }
}

impl Default for LayerSettings {
    fn default() -> Self {
        let mut visible = [false; DiagnosticLayer::COUNT];
        visible[DiagnosticLayer::IsostaticElevation.index()] = true;
        Self { visible }
    }
}

#[derive(Resource)]
struct DiagnosticAssets([Handle<GizmoAsset>; DiagnosticLayer::COUNT]);

pub struct DiagnosticRenderPlugin;

impl Plugin for DiagnosticRenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LayerSettings>()
            .add_systems(Startup, setup_scene)
            .add_systems(
                Update,
                (
                    rebuild_diagnostic_assets.run_if(resource_changed::<GeneratedWorld>),
                    sync_visible_layers.run_if(resource_changed::<LayerSettings>),
                ),
            );
    }
}

fn setup_scene(
    mut commands: Commands,
    mut gizmo_assets: ResMut<Assets<GizmoAsset>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(0.985).mesh().ico(5).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.025, 0.035, 0.055),
            perceptual_roughness: 1.0,
            unlit: true,
            ..default()
        })),
    ));

    let diagnostic_assets = std::array::from_fn(|index| {
        let layer = DiagnosticLayer::ALL[index];
        let handle = gizmo_assets.add(GizmoAsset::new());
        spawn_layer(&mut commands, handle.clone(), layer);
        handle
    });
    spawn_axes(&mut commands, &mut gizmo_assets);

    commands.insert_resource(DiagnosticAssets(diagnostic_assets));
}

fn spawn_layer(commands: &mut Commands, handle: Handle<GizmoAsset>, layer: DiagnosticLayer) {
    let spec = layer.spec();
    commands.spawn((
        Gizmo {
            handle,
            line_config: GizmoLineConfig {
                width: spec.line_width,
                perspective: false,
                ..default()
            },
            depth_bias: -0.0005,
        },
        RenderLayers::layer(layer.render_layer()),
    ));
}

fn spawn_axes(commands: &mut Commands, assets: &mut Assets<GizmoAsset>) {
    let mut axes = GizmoAsset::new();
    axes.line(
        Vec3::X * 1.08,
        Vec3::X * 1.35,
        Color::srgb(0.95, 0.25, 0.25),
    );
    axes.line(Vec3::Y * 1.08, Vec3::Y * 1.35, Color::srgb(0.3, 0.9, 0.4));
    axes.line(Vec3::Z * 1.08, Vec3::Z * 1.35, Color::srgb(0.3, 0.55, 1.0));
    commands.spawn(Gizmo {
        handle: assets.add(axes),
        line_config: GizmoLineConfig {
            width: 3.0,
            ..default()
        },
        ..default()
    });
}

fn rebuild_diagnostic_assets(
    world: Res<GeneratedWorld>,
    assets: Res<DiagnosticAssets>,
    mut gizmo_assets: ResMut<Assets<GizmoAsset>>,
) {
    for &layer in DiagnosticLayer::ALL {
        *gizmo_assets.get_mut(&assets.0[layer.index()]).unwrap() =
            layer.spec().build(&world, layer.radius());
    }
}

fn sync_visible_layers(
    settings: Res<LayerSettings>,
    mut camera_layers: Single<&mut RenderLayers, With<ViewerCamera>>,
) {
    // Retained gizmos ignore `Visibility`, so filtering must happen on the camera's render layers.
    let mut layers = vec![0];
    layers.extend(
        DiagnosticLayer::ALL
            .iter()
            .filter(|&&layer| settings.is_visible(layer))
            .map(|&layer| layer.render_layer()),
    );
    **camera_layers = RenderLayers::from_layers(&layers);
}

fn point_asset(mesh: &SphereMesh, radius: f32) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let points = &mesh.cell_centers;
    let size = (0.018 / (points.len() as f32).sqrt().max(8.0)).max(0.001);
    for (index, &point) in points.iter().enumerate() {
        let point = to_bevy(point) * radius;
        add_cross_marker(&mut asset, point, size, id_color(index));
    }
    asset
}

fn add_cross_marker(asset: &mut GizmoAsset, position: Vec3, size: f32, color: Color) {
    let reference = if position.y.abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let tangent = position.cross(reference).normalize() * size;
    let bitangent = position.cross(tangent).normalize() * size;
    asset.line(position - tangent, position + tangent, color);
    asset.line(position - bitangent, position + bitangent, color);
}

fn cell_marker_size(mesh: &SphereMesh) -> f32 {
    (CELL_MARKER_SCALE / (mesh.cell_count() as f32).sqrt())
        .clamp(MINIMUM_CELL_MARKER_SIZE, MAXIMUM_CELL_MARKER_SIZE)
}

fn opaque_color(color: Vec3) -> Color {
    Color::srgba(color.x, color.y, color.z, 1.0)
}

fn delaunay_asset(mesh: &SphereMesh, radius: f32) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let color = Color::srgba(0.35, 0.5, 0.72, 0.9);
    for edge in &mesh.edges {
        add_surface_edge(
            &mut asset,
            to_bevy(mesh.cell_centers[edge.cells[0]]),
            to_bevy(mesh.cell_centers[edge.cells[1]]),
            radius,
            color,
        );
    }
    asset
}

fn voronoi_asset(mesh: &SphereMesh, radius: f32) -> GizmoAsset {
    voronoi_edge_asset(mesh, |_, edge| Some((radius, id_color(edge.cells[0]))))
}

fn plate_asset(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    radius: f32,
    border_radius: f32,
) -> GizmoAsset {
    voronoi_edge_asset(mesh, |_, edge| {
        let left_plate = plates.cell_plates[edge.cells[0]];
        let right_plate = plates.cell_plates[edge.cells[1]];
        // White outlines keep this layer useful on its own; the boundary-class
        // layer deliberately overlays them at a slightly larger radius.
        if left_plate == right_plate {
            Some((radius, id_color(left_plate)))
        } else {
            Some((border_radius, Color::srgba(0.95, 0.95, 1.0, 0.98)))
        }
    })
}

fn crust_asset(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    crust: &CrustClassification,
    radius: f32,
) -> GizmoAsset {
    voronoi_edge_asset(mesh, |_, edge| {
        let left = crust.cell_class(plates, edge.cells[0]);
        let right = crust.cell_class(plates, edge.cells[1]);
        let color = if left != right {
            Color::srgba(0.96, 0.96, 1.0, 1.0)
        } else {
            match left {
                CrustClass::Oceanic => Color::srgba(0.12, 0.48, 0.95, 0.98),
                CrustClass::Continental => Color::srgba(0.92, 0.62, 0.2, 0.98),
            }
        };
        Some((radius, color))
    })
}

fn boundary_asset(
    mesh: &SphereMesh,
    boundaries: &BoundaryClassification,
    radius: f32,
) -> GizmoAsset {
    voronoi_edge_asset(mesh, |edge_index, _| {
        let color = match boundaries.edge_classes[edge_index] {
            BoundaryClass::Interior => return None,
            BoundaryClass::Convergent => Color::srgba(1.0, 0.25, 0.18, 1.0),
            BoundaryClass::Divergent => Color::srgba(0.15, 0.6, 1.0, 1.0),
            BoundaryClass::Transform => Color::srgba(1.0, 0.78, 0.12, 1.0),
        };
        Some((radius, color))
    })
}

fn scalar_field_asset(
    mesh: &SphereMesh,
    values: &[f32],
    stops: &[(f32, Vec3)],
    radius: f32,
) -> GizmoAsset {
    voronoi_edge_asset(mesh, |_, edge| {
        Some((
            radius,
            scalar_edge_color(edge.cells.map(|cell| values[cell]), stops),
        ))
    })
}

fn scalar_edge_color(values: [f32; 2], stops: &[(f32, Vec3)]) -> Color {
    let color = piecewise_lerp((values[0] + values[1]) * 0.5, stops);
    Color::srgba(color.x, color.y, color.z, 0.98)
}

fn piecewise_lerp(value: f32, stops: &[(f32, Vec3)]) -> Vec3 {
    let value = value.clamp(stops[0].0, stops[stops.len() - 1].0);
    for pair in stops.windows(2) {
        let (low_value, low) = pair[0];
        let (high_value, high) = pair[1];
        if value < high_value {
            let t = (value - low_value) / (high_value - low_value);
            return low.lerp(high, t);
        }
    }
    stops[stops.len() - 1].1
}

fn motion_asset(
    mesh: &SphereMesh,
    plates: &PlatePartition,
    kinematics: &PlateKinematics,
    radius: f32,
) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let stride = (mesh.cell_count() / MAXIMUM_VECTOR_COUNT).max(1);
    for cell in (0..mesh.cell_count()).step_by(stride) {
        let plate = plates.cell_plates[cell];
        let position = mesh.cell_centers[cell];
        let start = to_bevy(position.normalized()) * radius;
        let velocity = to_bevy(kinematics.velocity_at(plate, position));
        if velocity.length_squared() > 1.0e-12 {
            asset.arrow(start, start + velocity * 0.09, id_color(plate));
        }
    }
    asset
}

fn wind_asset(world: &GeneratedWorld, radius: f32) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    let mesh = &world.voronoi;
    let circulation = &world.atmospheric_circulation;
    let stride = (mesh.cell_count() / MAXIMUM_VECTOR_COUNT).max(1);
    let maximum_speed = circulation
        .diagnostics
        .wind_speed_meters_per_second
        .maximum
        .max(CALM_WIND_SPEED_METERS_PER_SECOND);
    for cell in (0..mesh.cell_count()).step_by(stride) {
        let wind = circulation.cell_wind_meters_per_second[cell];
        let speed = circulation.cell_wind_speed_meters_per_second[cell];
        if speed <= CALM_WIND_SPEED_METERS_PER_SECOND {
            continue;
        }
        let start = to_bevy(mesh.cell_centers[cell].normalized()) * radius;
        let direction = to_bevy(wind) / speed;
        let length = 0.025 + 0.075 * (speed / maximum_speed);
        let color = opaque_color(piecewise_lerp(speed, &WIND_SPEED_COLOR_STOPS));
        asset.arrow(start, start + direction * length, color);
    }
    asset
}

fn voronoi_edge_asset(
    mesh: &SphereMesh,
    mut style: impl FnMut(usize, &VoronoiEdge) -> Option<(f32, Color)>,
) -> GizmoAsset {
    let mut asset = GizmoAsset::new();
    for (edge_index, edge) in mesh.edges.iter().enumerate() {
        let Some((radius, color)) = style(edge_index, edge) else {
            continue;
        };
        add_surface_edge(
            &mut asset,
            to_bevy(mesh.vertices[edge.vertices[0]]),
            to_bevy(mesh.vertices[edge.vertices[1]]),
            radius,
            color,
        );
    }
    asset
}

fn add_surface_edge(asset: &mut GizmoAsset, start: Vec3, end: Vec3, radius: f32, color: Color) {
    let start = start.normalize();
    let end = end.normalize();
    let mut previous = start * radius;
    for segment in 1..=3 {
        let t = segment as f32 / 3.0;
        let current = start.lerp(end, t).normalize() * radius;
        asset.line(previous, current, color);
        previous = current;
    }
}

fn to_bevy(point: SphereVec3) -> Vec3 {
    Vec3::new(point.x, point.y, point.z)
}

fn id_color(id: usize) -> Color {
    let hue = (id as f32 * 137.508) % 360.0;
    Color::hsla(hue, 0.62, 0.62, 0.95)
}
