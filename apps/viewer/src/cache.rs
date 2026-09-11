//! Viewer-owned persistence for the last successfully generated domain model.

use crate::model::{
    ClimateSettings, ClimateWorld, CompleteWorld, GeneratedWorld, GenerationTimings,
    GeologySettings, GeologyWorld, TectonicsSettings, TectonicsWorld,
};
use bevy::prelude::Resource;
use procgen_climate::*;
use procgen_core::Vec3;
use procgen_cubesphere::{CubeFace, CubeField};
use procgen_geology::*;
use procgen_planet::{Orbit, Planet, Star};
use procgen_sphere::FibonacciConfig;
use procgen_sphere_mesh::{CellCorner, SphereMesh, VoronoiEdge};
use procgen_tectonics::*;
use procgen_terrain::{
    TerrainCellControls, TerrainControlConfig, TerrainControls, TerrainStampInput, TerrainStampKind,
};
use std::{
    env, fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAGIC: &[u8; 8] = b"PRCGENW\0";
const MAX_SNAPSHOT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_COLLECTION_ITEMS: usize = 32 * 1024 * 1024;
const GENERATOR_BUILD_ID: &str = env!("PROCGEN_GENERATOR_BUILD_ID");
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub enum CacheError {
    Io(io::Error),
    Invalid(String),
}

impl CacheError {
    fn invalid(message: impl fmt::Display) -> Self {
        Self::Invalid(message.to_string())
    }
}

impl fmt::Display for CacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for CacheError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Invalid(_) => None,
        }
    }
}

impl From<io::Error> for CacheError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Debug, Resource)]
pub struct WorldCache {
    path: PathBuf,
}

impl Default for WorldCache {
    fn default() -> Self {
        Self::new(platform_cache_path())
    }
}

impl WorldCache {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<GeneratedWorld, CacheError> {
        let metadata = fs::metadata(&self.path)?;
        if metadata.len() > MAX_SNAPSHOT_BYTES {
            return Err(CacheError::invalid("snapshot exceeds the size limit"));
        }
        let bytes = fs::read(&self.path)?;
        decode_snapshot(&bytes)
    }

    pub fn is_missing(error: &CacheError) -> bool {
        matches!(error, CacheError::Io(error) if error.kind() == io::ErrorKind::NotFound)
    }

    pub fn store(&self, world: CompleteWorld<'_>) -> Result<(), CacheError> {
        let bytes = encode_snapshot(world);
        if bytes.len() as u64 > MAX_SNAPSHOT_BYTES {
            return Err(CacheError::invalid("snapshot exceeds the size limit"));
        }
        atomic_replace(&self.path, &bytes)
    }

    pub fn clear(&self) -> Result<bool, CacheError> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}

fn platform_cache_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    let base = env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches"));
    #[cfg(target_os = "windows")]
    let base = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")));

    base.unwrap_or_else(env::temp_dir)
        .join("procgen-viewer")
        .join("last-world.bin")
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), CacheError> {
    let mut replacement = AtomicReplacement::begin(path)?;
    replacement.file.write_all(bytes)?;
    replacement.commit()
}

struct AtomicReplacement {
    destination: PathBuf,
    temp: PathBuf,
    file: File,
    committed: bool,
}

impl AtomicReplacement {
    fn begin(destination: &Path) -> Result<Self, CacheError> {
        let parent = destination
            .parent()
            .ok_or_else(|| CacheError::invalid("cache path has no parent directory"))?;
        fs::create_dir_all(parent)?;
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(".last-world-{}-{sequence}.tmp", std::process::id()));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        Ok(Self {
            destination: destination.to_owned(),
            temp,
            file,
            committed: false,
        })
    }

    fn commit(mut self) -> Result<(), CacheError> {
        fs::rename(&self.temp, &self.destination)?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for AtomicReplacement {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.temp);
        }
    }
}

fn encode_snapshot(world: CompleteWorld<'_>) -> Vec<u8> {
    let mut encoder = Encoder::default();
    encoder.bytes.extend_from_slice(MAGIC);
    GENERATOR_BUILD_ID.to_owned().encode(&mut encoder);
    world.tectonics.encode(&mut encoder);
    world.geology.encode(&mut encoder);
    world.climate.encode(&mut encoder);
    encoder.bytes
}

fn decode_snapshot(bytes: &[u8]) -> Result<GeneratedWorld, CacheError> {
    let mut decoder = Decoder { bytes, offset: 0 };
    if decoder.read_exact(MAGIC.len())? != MAGIC {
        return Err(CacheError::invalid("snapshot magic does not match"));
    }
    if String::decode(&mut decoder)? != GENERATOR_BUILD_ID {
        return Err(CacheError::invalid(
            "generator build identity does not match",
        ));
    }
    let tectonics = TectonicsWorld::decode(&mut decoder)?;
    let geology = GeologyWorld::decode(&mut decoder)?;
    let climate = ClimateWorld::decode(&mut decoder)?;
    if decoder.offset != bytes.len() {
        return Err(CacheError::invalid("snapshot contains trailing data"));
    }
    CompleteWorld {
        tectonics: &tectonics,
        geology: &geology,
        climate: &climate,
    }
    .validate()
    .map_err(CacheError::invalid)?;
    Ok(GeneratedWorld::from_phases(tectonics, geology, climate))
}

#[derive(Default)]
struct Encoder {
    bytes: Vec<u8>,
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
    fn read_exact(&mut self, length: usize) -> Result<&'a [u8], CacheError> {
        let end = self
            .offset
            .checked_add(length)
            .filter(|&end| end <= self.bytes.len())
            .ok_or_else(|| CacheError::invalid("snapshot ended unexpectedly"))?;
        let result = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(result)
    }
}

trait CacheCodec: Sized {
    fn encode(&self, encoder: &mut Encoder);
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError>;
}

macro_rules! number_codec {
    ($($ty:ty),+ $(,)?) => {$(
        impl CacheCodec for $ty {
            fn encode(&self, encoder: &mut Encoder) {
                encoder.bytes.extend_from_slice(&self.to_le_bytes());
            }
            fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
                Ok(Self::from_le_bytes(decoder.read_exact(size_of::<Self>())?.try_into().unwrap()))
            }
        }
    )+};
}

number_codec!(i32, u32, u64, f32, f64);

impl CacheCodec for usize {
    fn encode(&self, encoder: &mut Encoder) {
        (*self as u64).encode(encoder);
    }
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
        u64::decode(decoder)?
            .try_into()
            .map_err(|_| CacheError::invalid("snapshot integer exceeds this platform"))
    }
}

impl<T: CacheCodec> CacheCodec for Vec<T> {
    fn encode(&self, encoder: &mut Encoder) {
        encode_slice(self, encoder);
    }
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
        let length = usize::decode(decoder)?;
        if length > MAX_COLLECTION_ITEMS {
            return Err(CacheError::invalid("snapshot collection is too large"));
        }
        (0..length).map(|_| T::decode(decoder)).collect()
    }
}

fn encode_slice<T: CacheCodec>(values: &[T], encoder: &mut Encoder) {
    values.len().encode(encoder);
    for value in values {
        value.encode(encoder);
    }
}

impl<T: CacheCodec> CacheCodec for Option<T> {
    fn encode(&self, encoder: &mut Encoder) {
        match self {
            Some(value) => {
                1_u32.encode(encoder);
                value.encode(encoder);
            }
            None => 0_u32.encode(encoder),
        }
    }
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
        match u32::decode(decoder)? {
            0 => Ok(None),
            1 => Ok(Some(T::decode(decoder)?)),
            _ => Err(CacheError::invalid("snapshot option tag is invalid")),
        }
    }
}

impl<T: CacheCodec, const N: usize> CacheCodec for [T; N] {
    fn encode(&self, encoder: &mut Encoder) {
        for value in self {
            value.encode(encoder);
        }
    }
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
        let values: Vec<_> = (0..N)
            .map(|_| T::decode(decoder))
            .collect::<Result<_, _>>()?;
        values
            .try_into()
            .map_err(|_| CacheError::invalid("snapshot array length is invalid"))
    }
}

impl<A: CacheCodec, B: CacheCodec> CacheCodec for (A, B) {
    fn encode(&self, encoder: &mut Encoder) {
        self.0.encode(encoder);
        self.1.encode(encoder);
    }
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
        Ok((A::decode(decoder)?, B::decode(decoder)?))
    }
}

impl CacheCodec for String {
    fn encode(&self, encoder: &mut Encoder) {
        self.as_bytes().to_vec().encode(encoder);
    }
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
        String::from_utf8(Vec::<u8>::decode(decoder)?)
            .map_err(|_| CacheError::invalid("snapshot string is not UTF-8"))
    }
}

impl CacheCodec for u8 {
    fn encode(&self, encoder: &mut Encoder) {
        encoder.bytes.push(*self);
    }
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
        Ok(decoder.read_exact(1)?[0])
    }
}

impl CacheCodec for Vec3 {
    fn encode(&self, encoder: &mut Encoder) {
        self.x.encode(encoder);
        self.y.encode(encoder);
        self.z.encode(encoder);
    }
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
        Ok(Self::new(
            f32::decode(decoder)?,
            f32::decode(decoder)?,
            f32::decode(decoder)?,
        ))
    }
}

macro_rules! struct_codec {
    ($($ty:ty { $($field:ident),+ $(,)? })+) => {$(
        impl CacheCodec for $ty {
            fn encode(&self, encoder: &mut Encoder) {
                $(self.$field.encode(encoder);)+
            }
            fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
                Ok(Self { $($field: CacheCodec::decode(decoder)?),+ })
            }
        }
    )+};
}

struct_codec! {
    FibonacciConfig { count, jitter, seed }
    Star { luminosity_watts }
    Orbit { semi_major_axis_meters, eccentricity, obliquity_radians, stellar_longitude_at_periapsis_radians }
    Planet { star, orbit, radius_meters, sidereal_rotation_period_seconds, atmospheric_specific_gas_constant_joules_per_kilogram_kelvin, maximum_land_elevation_meters }
    PlatePartitionConfig { arc_count, curvature, subdivided_fraction, piece_fraction, growth_roughness, seed }
    CrustClassificationConfig { continental_fraction, nucleus_count, growth_roughness, seed }
    PlateKinematicsConfig { seed, minimum_angular_speed, maximum_angular_speed, flow_frequency, coherence, oceanic_speed_factor, continental_speed_factor }
    MaterialTransportConfig { gap_radius }
    PoleDriftConfig { axis_drift_rate, speed_drift_rate, speed_drift_limit }
    PlateLifecycleConfig { rift_rate, rift_minimum_area_fraction, rift_curvature, rift_opening_speed, suture_time, suture_minimum_shared_edges }
    PlateEvolutionConfig { seed, step_count, step_duration, transport, deformation, pole_drift, lifecycle }
    CrustBirthPriorConfig { ridge_less_age }
    BaseElevationConfig { seed, continental_base, ridge_elevation, deep_ocean_elevation, cooling_age, dynamic_topography_amplitude, basement_amplitude, basement_frequency, margin_width_hops, margin_edge_elevation }
    BoundaryEffect { offset, depth }
    ContinentalRiftProfile { center_offset, flank_offset, decay_depth }
    BoundaryDeformationConfig { convergent, rift, transform, collision, trench, island_arc, saturation_speed, full_deformation_time, maximum_magnitude }
    CoarseElevationConfig { smoothing_passes, smoothing_weight, sea_level }
    HotspotFieldConfig { hotspot_count, maximum_trail_cells, province_fraction, province_radius_hops, province_rim_hops, seed }
    OceanicPeakFieldConfig { maximum_young_age, seamount_density_scale, abyssal_hill_density_scale, maximum_position_offset, maximum_seamount_height, maximum_abyssal_hill_height, seed }
    VolcanicArcFieldConfig { minimum_boundary_edges, inland_offset_cells, peak_density_divisor, strength_saturation }
    CratonFieldConfig { minimum_boundary_distance, ramp_width }
    SedimentaryBasinFieldConfig { maximum_elevation, minimum_cell_count, maximum_ocean_perimeter_fraction }
    GeologicalElevationConfig { hotspot_uplift, plateau_uplift, volcanic_arc_uplift, craton_flattening, basin_flattening }
    IsostaticAdjustmentConfig { adjustment_strength, continental_support, convergent_support_bonus, divergent_support_penalty, craton_support_bonus, maximum_boundary_distance }
    TerrainControlConfig { base_detail_amplitude, craton_amplitude_delta, volcanic_arc_amplitude, convergent_boundary_amplitude, divergent_boundary_amplitude, transform_boundary_amplitude, basin_amplitude_delta, boundary_strength_saturation, base_ridge_weight, convergent_ridge_weight, volcanic_arc_ridge_weight, base_octave_gain, craton_octave_gain_delta, basin_octave_gain_delta, maximum_abyssal_amplitude, abyssal_age_saturation, hotspot_stamp_strength, volcanic_arc_stamp_scale, oceanic_peak_stamp_scale }
    SolarForcingConfig { orbital_phase, annual_sample_count }
    ClimateAlbedoConfig { land, ocean, snow, ice }
    RadiativeEquilibriumConfig { emissivity }
    SeasonalThermalConfig { land_heat_capacity, ocean_heat_capacity, orbital_period_days }
    AtmosphericCirculationConfig { surface_drag_per_second, terrain_steering, maximum_wind_speed_meters_per_second }
    MoistureTransportConfig { step_count, step_seconds, reference_capacity_kg_per_m2, reference_temperature_kelvin, capacity_temperature_sensitivity_per_kelvin, minimum_capacity_kg_per_m2, maximum_capacity_kg_per_m2, ocean_evaporation_rate_per_second, rainfall_rate_per_second, orographic_coefficient_per_meter, maximum_orographic_fraction_per_step, maximum_transport_fraction_per_step }
    CryosphereConfig { maximum_iterations, closure_tolerance, snowfall_temperature_kelvin, melt_temperature_kelvin, full_snow_cover_kg_per_m2, seasonal_snow_capacity_kg_per_m2, snow_melt_kg_per_m2_per_kelvin_day, land_ice_melt_kg_per_m2_per_kelvin_day, sea_ice_growth_fraction_per_kelvin_day, sea_ice_melt_fraction_per_kelvin_day }
    ClimateCouplingConfig { maximum_iterations, under_relaxation, albedo_tolerance, temperature_tolerance_kelvin, precipitation_tolerance_kg_per_m2_per_day, cover_fraction_tolerance, albedo, radiative_equilibrium, seasonal_thermal, atmospheric_circulation, moisture_transport, cryosphere }
    TectonicsSettings { fibonacci, plates, crust, kinematics, birth_prior, evolution, base_elevation, elevation }
    GeologySettings { hotspots, oceanic_peaks, volcanic_arcs, cratons, basins, geological_elevation, isostasy, terrain_controls }
    ClimateSettings { planet, solar_forcing, coupling }

    VoronoiEdge { vertices, cells }
    CellCorner { vertex, neighbor, edge }
    SphereMesh { radius, cell_centers, cell_offsets, corners, cell_areas, vertices, vertex_cells, vertex_neighbors, edges }
    PlatePartition { cell_plates, plate_count }
    CrustClassificationDiagnostics { continental_fraction, component_count }
    PlateKinematics { angular_velocities }
    BoundaryClassification { edge_classes, edge_normal_speeds, edge_shear }
    PlateEvolutionDiagnostics { active_step_count, owner_change_count, subducted_particle_count, born_particle_count, collided_cell_count, maximum_collision_stack, sampled_cell_count, starting_continental_particle_count, final_continental_particle_count, rift_count, failed_rift_count, suture_count }
    FieldSummary { minimum, maximum, mean }
    CrustBirthPriorDiagnostics { hops, oceanic_cell_count, ridge_cell_count, ridge_plate_count, ridge_less_plate_count, fallback_cell_count }
    SeafloorAgeDiagnostics { summary, oceanic_cell_count }
    SeafloorAge { cell_ages, diagnostics }
    BaseElevationDiagnostics { summary, oceanic, dynamic_topography, basement, margin_depth, oceanic_cell_count, continental_cell_count, margin_cell_count }
    BaseElevation { cell_elevations, diagnostics }
    BoundaryDeformationDiagnostics { summary, source_cell_count, uplifted_cell_count, subsided_cell_count }
    BoundaryDeformation { cell_deformation, diagnostics }
    CoarseElevation { cell_elevations, sea_level, diagnostics }
    HotspotTrailCell { cell, intensity }
    Hotspot { mantle_position, source_cell, plate, trail, province_cell_count }
    HotspotDiagnostics { trail_cell_count, affected_cell_count, overlap_cell_count, stationary_source_count, shortest_trail_cells, longest_trail_cells, province_count, province_cell_count }
    HotspotField { hotspots, cell_intensities, cell_hotspots, cell_plateau, diagnostics }
    OceanicPeak { cell, kind, position, strength, height }
    OceanicPeakDiagnostics { oceanic_cell_count, hotspot_candidate_cell_count, young_seafloor_candidate_cell_count, overlap_cell_count, density, peak_count, seamount_peak_count, abyssal_hill_peak_count, height }
    OceanicPeakField { cell_densities, cell_kinds, peaks, diagnostics }
    VolcanicArcCell { cell, strength }
    VolcanicArcSegment { overriding_plate, boundary_edges, boundary_cells, arc_cells, peaks, inland_depth }
    VolcanicArcDiagnostics { qualifying_edge_count, boundary_cell_count, discarded_short_segment_count, discarded_landlocked_segment_count, arc_cell_count, affected_cell_count, overlap_cell_count, peak_count }
    VolcanicArcField { segments, cell_strengths, cell_segments, diagnostics }
    CratonDiagnostics { boundary_cell_count, continental_land_cell_count, craton_cell_count, full_strength_cell_count, maximum_boundary_distance, strength }
    CratonField { cell_strengths, diagnostics }
    SedimentaryBasin { root_cell, cell_count, ocean_perimeter_fraction, minimum_elevation }
    SedimentaryBasinDiagnostics { candidate_cell_count, component_count, basin_count, basin_cell_count, rejected_small_component_count, rejected_ocean_exposed_component_count, basin_cell_count_range }
    SedimentaryBasinField { cell_basins, basins, diagnostics }
    ElevationEffectDiagnostics { affected_cell_count, total_delta, maximum_absolute_delta }
    SignedEffectDiagnostics { rise, sink }
    GeologicalElevationDiagnostics { elevation, hotspots, volcanic_arcs, cratons, basins }
    GeologicalElevation { cell_elevations, sea_level, diagnostics }
    IsostaticAdjustmentDiagnostics { support, elevation, oceanic_cell_count, preserved_basin_cell_count, adjustment }
    IsostaticAdjustment { cell_support, cell_elevations, sea_level, diagnostics }
    AreaWeightedSummary { minimum, maximum, area_weighted_mean }
    SolarForcingDiagnostics { orbital_phase, orbital_distance_meters, stellar_flux_watts_per_square_meter, solar_declination_radians, polar_night_cell_count, polar_day_cell_count, daily_mean, annual_mean }
    SolarForcing { daily_mean_insolation, annual_mean_insolation, diagnostics }
    RadiativeEquilibriumDiagnostics { daily, annual }
    RadiativeEquilibriumTemperature { daily_effective_temperature_kelvin, annual_effective_temperature_kelvin, diagnostics }
    SurfaceThermalDiagnostics { cell_count, selected_area_weighted_mean_kelvin }
    SeasonalThermalDiagnostics { selected_phase, annual_mean, annual_minimum, annual_maximum, annual_amplitude, land, ocean, maximum_periodic_closure_error_kelvin, maximum_fixed_point_iterations }
    SeasonalThermalResponse { selected_temperature_kelvin, annual_temperature_samples_kelvin, annual_sample_count, annual_mean_temperature_kelvin, annual_minimum_temperature_kelvin, annual_maximum_temperature_kelvin, annual_amplitude_kelvin, diagnostics }
    AtmosphericCirculationDiagnostics { wind_speed_meters_per_second, temperature_gradient_kelvin_per_radian, pressure_gradient_acceleration_meters_per_second_squared, coriolis_parameter_per_second, terrain_steering_fraction, calm_cell_count, terrain_steered_cell_count, speed_capped_cell_count, maximum_tangency_error_meters_per_second }
    AtmosphericCirculation { cell_wind_meters_per_second, cell_wind_speed_meters_per_second, cell_temperature_gradient_kelvin_per_radian, cell_pressure_gradient_acceleration_meters_per_second_squared, cell_coriolis_parameter_per_second, cell_terrain_steering_fraction, diagnostics }
    MoistureTransportDiagnostics { humidity_kg_per_m2, moisture_capacity_kg_per_m2, evaporation_kg_per_m2_per_day, precipitation_kg_per_m2_per_day, condensation_kg_per_m2_per_day, orographic_precipitation_kg_per_m2_per_day, simulated_days, ocean_cell_count, precipitating_cell_count, orographic_cell_count, maximum_orographic_fraction_per_step, mass_balance_error_kg_per_m2 }
    MoistureTransport { cell_humidity_kg_per_m2, cell_moisture_capacity_kg_per_m2, cell_evaporation_kg_per_m2_per_day, cell_precipitation_kg_per_m2_per_day, cell_condensation_kg_per_m2_per_day, cell_orographic_precipitation_kg_per_m2_per_day, diagnostics }
    CryosphereDiagnostics { selected_snowfall_kg_per_m2_per_day, selected_melt_kg_per_m2_per_day, selected_snow_cover_fraction, land_ice_cover_fraction, selected_sea_ice_cover_fraction, annual_snowfall_kg_per_m2, annual_snow_melt_kg_per_m2, annual_land_ice_accumulation_kg_per_m2, annual_land_ice_ablation_kg_per_m2, annual_sea_ice_growth_fraction, annual_sea_ice_melt_fraction, land_cell_count, ocean_cell_count, snow_covered_cell_count, land_ice_cell_count, sea_ice_cell_count, maximum_iterations_used, maximum_snow_closure_error_kg_per_m2, maximum_sea_ice_closure_error, snow_mass_balance_error_kg_per_m2, land_ice_mass_balance_kg_per_m2, sea_ice_cover_balance_error }
    Cryosphere { cell_snowfall_kg_per_m2_per_day, cell_melt_kg_per_m2_per_day, cell_snow_cover_fraction, cell_land_ice_cover_fraction, cell_sea_ice_cover_fraction, diagnostics }
    ClimateCouplingDiagnostics { iterations, albedo_residual_rms, temperature_change_rms_kelvin, precipitation_change_rms_kg_per_m2_per_day, cover_fraction_change_rms }
    TerrainCellControls { base_elevation, detail_amplitude, ridge_weight, octave_gain, abyssal_amplitude }
    TerrainStampInput { cell, kind, source_index, position, strength }
    TerrainControls { cells, stamps }
}

macro_rules! enum_codec {
    ($ty:ty { $($value:path = $tag:literal),+ $(,)? }) => {
        impl CacheCodec for $ty {
            fn encode(&self, encoder: &mut Encoder) {
                let tag: u8 = match self { $($value => $tag),+ };
                tag.encode(encoder);
            }
            fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
                match u8::decode(decoder)? {
                    $($tag => Ok($value),)+
                    _ => Err(CacheError::invalid("snapshot enum tag is invalid")),
                }
            }
        }
    };
}

enum_codec!(BoundaryClass {
    BoundaryClass::Interior = 0,
    BoundaryClass::Convergent = 1,
    BoundaryClass::Divergent = 2,
    BoundaryClass::Transform = 3,
});
enum_codec!(OceanicPeakKind {
    OceanicPeakKind::Seamount = 0,
    OceanicPeakKind::AbyssalHill = 1,
});
enum_codec!(TerrainStampKind {
    TerrainStampKind::Hotspot = 0,
    TerrainStampKind::VolcanicArc = 1,
    TerrainStampKind::OceanicSeamount = 2,
    TerrainStampKind::OceanicAbyssalHill = 3,
});

impl<const N: usize> CacheCodec for CubeField<N> {
    fn encode(&self, encoder: &mut Encoder) {
        self.resolution().encode(encoder);
        for face in CubeFace::ALL {
            encode_slice(self.face(face).texels(), encoder);
        }
    }

    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
        let resolution = u32::decode(decoder)?;
        CubeField::from_face_texels(resolution, CacheCodec::decode(decoder)?)
            .map_err(CacheError::invalid)
    }
}

impl CacheCodec for TectonicsWorld {
    fn encode(&self, encoder: &mut Encoder) {
        self.config.encode(encoder);
        self.voronoi.encode(encoder);
        self.plates.encode(encoder);
        self.crust.encode(encoder);
        self.kinematics.encode(encoder);
        self.boundaries.encode(encoder);
        self.cell_birth.encode(encoder);
        self.birth_prior.encode(encoder);
        self.evolution.encode(encoder);
        self.seafloor_age.encode(encoder);
        self.base_elevation.encode(encoder);
        self.deformation.encode(encoder);
        self.elevation.encode(encoder);
    }

    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
        Ok(Self {
            config: CacheCodec::decode(decoder)?,
            voronoi: CacheCodec::decode(decoder)?,
            plates: CacheCodec::decode(decoder)?,
            crust: CacheCodec::decode(decoder)?,
            kinematics: CacheCodec::decode(decoder)?,
            boundaries: CacheCodec::decode(decoder)?,
            cell_birth: CacheCodec::decode(decoder)?,
            birth_prior: CacheCodec::decode(decoder)?,
            evolution: CacheCodec::decode(decoder)?,
            seafloor_age: CacheCodec::decode(decoder)?,
            base_elevation: CacheCodec::decode(decoder)?,
            deformation: CacheCodec::decode(decoder)?,
            elevation: CacheCodec::decode(decoder)?,
            timings: GenerationTimings::default(),
        })
    }
}

impl CacheCodec for GeologyWorld {
    fn encode(&self, encoder: &mut Encoder) {
        self.config.encode(encoder);
        self.hotspots.encode(encoder);
        self.oceanic_peaks.encode(encoder);
        self.volcanic_arcs.encode(encoder);
        self.cratons.encode(encoder);
        self.basins.encode(encoder);
        self.geological_elevation.encode(encoder);
        self.isostasy.encode(encoder);
        self.terrain_controls.encode(encoder);
        self.terrain_control_bake.encode(encoder);
    }

    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
        Ok(Self {
            config: CacheCodec::decode(decoder)?,
            hotspots: CacheCodec::decode(decoder)?,
            oceanic_peaks: CacheCodec::decode(decoder)?,
            volcanic_arcs: CacheCodec::decode(decoder)?,
            cratons: CacheCodec::decode(decoder)?,
            basins: CacheCodec::decode(decoder)?,
            geological_elevation: CacheCodec::decode(decoder)?,
            isostasy: CacheCodec::decode(decoder)?,
            terrain_controls: CacheCodec::decode(decoder)?,
            terrain_control_bake: CacheCodec::decode(decoder)?,
            timings: GenerationTimings::default(),
        })
    }
}

impl CacheCodec for ClimateWorld {
    fn encode(&self, encoder: &mut Encoder) {
        self.config.encode(encoder);
        self.solar_forcing.encode(encoder);
        self.radiative_equilibrium.encode(encoder);
        self.seasonal_thermal.encode(encoder);
        self.atmospheric_circulation.encode(encoder);
        self.moisture_transport.encode(encoder);
        self.cryosphere.encode(encoder);
        self.cell_albedo.encode(encoder);
        self.coupling_diagnostics.encode(encoder);
    }

    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
        Ok(Self {
            config: CacheCodec::decode(decoder)?,
            solar_forcing: CacheCodec::decode(decoder)?,
            radiative_equilibrium: CacheCodec::decode(decoder)?,
            seasonal_thermal: CacheCodec::decode(decoder)?,
            atmospheric_circulation: CacheCodec::decode(decoder)?,
            moisture_transport: CacheCodec::decode(decoder)?,
            cryosphere: CacheCodec::decode(decoder)?,
            cell_albedo: CacheCodec::decode(decoder)?,
            coupling_diagnostics: CacheCodec::decode(decoder)?,
            timings: GenerationTimings::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Fixture, cache as test_cache};

    #[test]
    fn snapshot_round_trip_restores_settings_and_domain_data_without_timings() {
        let fixture = Fixture::new(32, 11);
        let bytes = encode_snapshot(fixture.complete());
        let loaded = decode_snapshot(&bytes).unwrap();
        let complete = loaded.complete().unwrap();

        assert_eq!(complete.settings(), fixture.complete().settings());
        assert_eq!(
            complete.tectonics.voronoi.cell_centers,
            fixture.tectonics.voronoi.cell_centers
        );
        assert_eq!(
            complete.tectonics.voronoi.edges,
            fixture.tectonics.voronoi.edges
        );
        assert_eq!(complete.tectonics.plates, fixture.tectonics.plates);
        assert_eq!(complete.geology.isostasy, fixture.geology.isostasy);
        assert_eq!(complete.climate.cryosphere, fixture.climate.cryosphere);
        assert_eq!(
            complete.geology.terrain_controls,
            fixture.geology.terrain_controls
        );
        assert_eq!(
            complete.geology.terrain_control_bake,
            fixture.geology.terrain_control_bake
        );
        assert!(complete.tectonics.timings.stages().is_empty());
        assert!(complete.geology.timings.stages().is_empty());
        assert!(complete.climate.timings.stages().is_empty());
    }

    fn bake_offset(fixture: &Fixture, snapshot: &[u8]) -> usize {
        let mut encoder = Encoder::default();
        fixture.geology.terrain_control_bake.encode(&mut encoder);
        snapshot
            .windows(encoder.bytes.len())
            .position(|window| window == encoder.bytes)
            .unwrap()
    }

    #[test]
    fn corrupt_bake_dimensions_are_rejected() {
        let fixture = Fixture::new(32, 30);
        let mut bytes = encode_snapshot(fixture.complete());
        let offset = bake_offset(&fixture, &bytes);
        bytes[offset..offset + size_of::<u32>()].copy_from_slice(&3_u32.to_le_bytes());

        assert!(decode_snapshot(&bytes).is_err());
    }

    #[test]
    fn corrupt_bake_data_is_rejected() {
        let fixture = Fixture::new(32, 30);
        let mut bytes = encode_snapshot(fixture.complete());
        let offset = bake_offset(&fixture, &bytes) + size_of::<u32>() + size_of::<u64>();
        bytes[offset..offset + size_of::<f32>()].copy_from_slice(&f32::NAN.to_le_bytes());

        assert!(decode_snapshot(&bytes).is_err());
    }

    #[test]
    fn loaded_snapshot_reuses_cached_bake_data() {
        let (cache_dir, cache) = test_cache("reuse-bake");
        let fixture = Fixture::new(32, 30);
        let mut bytes = encode_snapshot(fixture.complete());
        let offset = bake_offset(&fixture, &bytes) + size_of::<u32>() + size_of::<u64>();
        let cached_value = 123.25_f32;
        bytes[offset..offset + size_of::<f32>()].copy_from_slice(&cached_value.to_le_bytes());
        fs::create_dir_all(cache.path.parent().unwrap()).unwrap();
        fs::write(&cache.path, bytes).unwrap();

        let loaded = cache.load().unwrap();
        let bake = &loaded.geology().unwrap().terrain_control_bake;
        assert_eq!(bake.face(CubeFace::PositiveX).texels()[0][0], cached_value);
        assert_ne!(*bake, fixture.geology.terrain_control_bake);

        fs::remove_dir_all(cache_dir).unwrap();
    }

    #[test]
    fn build_identity_mismatch_invalidates_snapshot() {
        let fixture = Fixture::new(32, 12);

        let mut wrong_build = encode_snapshot(fixture.complete());
        let identity_start = MAGIC.len() + size_of::<u64>();
        wrong_build[identity_start] ^= 1;
        assert!(decode_snapshot(&wrong_build).is_err());
    }

    #[test]
    fn truncated_snapshot_is_nonfatal_corruption() {
        let fixture = Fixture::new(32, 30);
        let mut bytes = encode_snapshot(fixture.complete());
        bytes.truncate(bytes.len() / 2);

        assert!(decode_snapshot(&bytes).is_err());
    }

    #[test]
    fn invalid_topology_sparse_indices_and_field_lengths_are_rejected() {
        let mut fixture = Fixture::new(32, 21);
        let cell_count = fixture.tectonics.voronoi.cell_count();
        fixture.tectonics.voronoi.edges[0].cells[0] = cell_count;
        assert!(decode_snapshot(&encode_snapshot(fixture.complete())).is_err());

        let mut fixture = Fixture::new(32, 26);
        fixture.geology.hotspots.hotspots[0].source_cell = fixture.tectonics.voronoi.cell_count();
        assert!(decode_snapshot(&encode_snapshot(fixture.complete())).is_err());

        let mut fixture = Fixture::new(32, 25);
        fixture.geology.hotspots.hotspots[0].plate = fixture.tectonics.plates.plate_count;
        assert!(decode_snapshot(&encode_snapshot(fixture.complete())).is_err());

        let mut fixture = Fixture::new(32, 17);
        fixture.climate.cryosphere.cell_snow_cover_fraction.pop();
        assert!(decode_snapshot(&encode_snapshot(fixture.complete())).is_err());

        let mut fixture = Fixture::new(32, 18);
        fixture.climate.cell_albedo.pop();
        assert!(decode_snapshot(&encode_snapshot(fixture.complete())).is_err());
    }

    #[test]
    fn successful_store_atomically_replaces_previous_snapshot() {
        let (cache_dir, cache) = test_cache("replace");
        let first = Fixture::new(32, 29);
        let second = Fixture::new(48, 27);
        cache.store(first.complete()).unwrap();
        cache.store(second.complete()).unwrap();

        let loaded = cache.load().unwrap();
        assert_eq!(
            loaded.complete().unwrap().settings(),
            second.complete().settings()
        );
        assert_eq!(loaded.tectonics().unwrap().voronoi.cell_count(), 48);
        let parent_entries: Vec<_> = fs::read_dir(cache.path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(parent_entries, [cache.path.file_name().unwrap()]);

        fs::remove_dir_all(cache_dir).unwrap();
    }

    #[test]
    fn failed_atomic_write_preserves_previous_snapshot() {
        let (cache_dir, cache) = test_cache("failed-replace");
        let fixture = Fixture::new(32, 23);
        cache.store(fixture.complete()).unwrap();
        let original = fs::read(&cache.path).unwrap();

        let mut replacement = AtomicReplacement::begin(&cache.path).unwrap();
        replacement.file.write_all(b"incomplete").unwrap();
        drop(replacement);

        assert_eq!(fs::read(&cache.path).unwrap(), original);
        assert_eq!(
            fs::read_dir(cache.path.parent().unwrap()).unwrap().count(),
            1
        );
        fs::remove_dir_all(cache_dir).unwrap();
    }
}
