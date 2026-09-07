//! Viewer-owned persistence for the last successfully generated domain model.

use crate::model::{GeneratedWorld, GenerationSettings, GenerationTimings};
use bevy::prelude::Resource;
use procgen_climate::*;
use procgen_core::Vec3;
use procgen_geology::*;
use procgen_planet::{Orbit, Planet, Star};
use procgen_sphere::FibonacciConfig;
use procgen_sphere_mesh::{CellCorner, SphereMesh, VoronoiEdge};
use procgen_tectonics::*;
use std::{
    env, fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAGIC: &[u8; 8] = b"PRCGENW\0";
const FORMAT_VERSION: u32 = 1;
const MAX_SNAPSHOT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_COLLECTION_ITEMS: usize = 32 * 1024 * 1024;
const GENERATOR_BUILD_ID: &str = env!("PROCGEN_GENERATOR_BUILD_ID");
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub enum CacheError {
    Io(io::Error),
    Invalid(&'static str),
    InvalidOwned(String),
}

impl fmt::Display for CacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Invalid(message) => formatter.write_str(message),
            Self::InvalidOwned(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for CacheError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Invalid(_) | Self::InvalidOwned(_) => None,
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

    pub fn load(&self) -> Result<(GenerationSettings, GeneratedWorld), CacheError> {
        let metadata = fs::metadata(&self.path)?;
        if metadata.len() > MAX_SNAPSHOT_BYTES {
            return Err(CacheError::Invalid("snapshot exceeds the size limit"));
        }
        let bytes = fs::read(&self.path)?;
        decode_snapshot(&bytes)
    }

    pub fn is_missing(error: &CacheError) -> bool {
        matches!(error, CacheError::Io(error) if error.kind() == io::ErrorKind::NotFound)
    }

    pub fn store(
        &self,
        settings: &GenerationSettings,
        world: &GeneratedWorld,
    ) -> Result<(), CacheError> {
        let bytes = encode_snapshot(settings, world);
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
        .join("last-world-v1.bin")
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), CacheError> {
    atomic_replace_with(path, |file| file.write_all(bytes))
}

fn atomic_replace_with(
    path: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<(), CacheError> {
    let parent = path
        .parent()
        .ok_or(CacheError::Invalid("cache path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".last-world-{}-{sequence}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        write(&mut file)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        sync_directory(parent)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_: &Path) -> io::Result<()> {
    Ok(())
}

fn encode_snapshot(settings: &GenerationSettings, world: &GeneratedWorld) -> Vec<u8> {
    let mut encoder = Encoder::default();
    encoder.bytes.extend_from_slice(MAGIC);
    FORMAT_VERSION.encode(&mut encoder);
    GENERATOR_BUILD_ID.to_owned().encode(&mut encoder);
    settings.encode(&mut encoder);
    world.encode(&mut encoder);
    encoder.bytes
}

fn decode_snapshot(bytes: &[u8]) -> Result<(GenerationSettings, GeneratedWorld), CacheError> {
    let mut decoder = Decoder { bytes, offset: 0 };
    if decoder.read_exact(MAGIC.len())? != MAGIC {
        return Err(CacheError::Invalid("snapshot magic does not match"));
    }
    if u32::decode(&mut decoder)? != FORMAT_VERSION {
        return Err(CacheError::Invalid(
            "snapshot format version does not match",
        ));
    }
    if String::decode(&mut decoder)? != GENERATOR_BUILD_ID {
        return Err(CacheError::Invalid(
            "generator build identity does not match",
        ));
    }
    let settings = GenerationSettings::decode(&mut decoder)?;
    let mut world = GeneratedWorld::decode(&mut decoder)?;
    if decoder.offset != bytes.len() {
        return Err(CacheError::Invalid("snapshot contains trailing data"));
    }
    world.config = settings;
    validate_world(&world)?;
    Ok((settings, world))
}

fn validate_world(world: &GeneratedWorld) -> Result<(), CacheError> {
    let mesh = &world.voronoi;
    let cells = mesh.cell_count();
    let vertices = mesh.vertex_count();
    let edges = mesh.edge_count();

    if world.config.fibonacci.count != cells
        || world.config.plates.plate_count() != world.plates.plate_count
        || world.config.hotspots.hotspot_count != world.hotspots.hotspots.len()
        || world.config.solar_forcing.annual_sample_count
            != world.seasonal_thermal.annual_sample_count
        || cells < 4
        || vertices != cells.saturating_mul(2).saturating_sub(4)
        || edges != cells.saturating_mul(3).saturating_sub(6)
        || mesh.corners.len() != edges.saturating_mul(2)
        || !mesh.radius.is_finite()
        || mesh.radius <= 0.0
        || mesh
            .cell_centers
            .iter()
            .chain(&mesh.vertices)
            .any(|point| !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite())
        || mesh.cell_offsets.len() != cells + 1
        || mesh.cell_offsets.first() != Some(&0)
        || mesh.cell_offsets.last() != Some(&mesh.corners.len())
        || mesh.cell_offsets.windows(2).any(|pair| pair[0] > pair[1])
        || mesh
            .cell_offsets
            .windows(2)
            .any(|pair| pair[1] - pair[0] < 3)
        || mesh.cell_areas.len() != cells
        || mesh
            .cell_areas
            .iter()
            .any(|area| !area.is_finite() || *area <= 0.0)
        || mesh.vertex_cells.len() != vertices
        || mesh.vertex_neighbors.len() != vertices
        || mesh.corners.iter().any(|corner| {
            corner.vertex >= vertices || corner.neighbor >= cells || corner.edge >= edges
        })
        || mesh.edges.iter().any(|edge| {
            edge.vertices.iter().any(|&vertex| vertex >= vertices)
                || edge.cells.iter().any(|&cell| cell >= cells)
        })
        || mesh
            .vertex_cells
            .iter()
            .flatten()
            .any(|&cell| cell >= cells)
        || mesh
            .vertex_neighbors
            .iter()
            .flatten()
            .any(|&vertex| vertex >= vertices)
    {
        return Err(CacheError::Invalid("snapshot mesh topology is invalid"));
    }

    let mut edge_incidence = vec![0_u8; edges];
    for (cell, offsets) in mesh.cell_offsets.windows(2).enumerate() {
        for corner in &mesh.corners[offsets[0]..offsets[1]] {
            let edge = mesh.edges[corner.edge];
            let incident_cells = mesh.vertex_cells[corner.vertex];
            if !edge.cells.contains(&cell)
                || !edge.cells.contains(&corner.neighbor)
                || !incident_cells.contains(&cell)
                || !incident_cells.contains(&corner.neighbor)
                || cell == corner.neighbor
            {
                return Err(CacheError::Invalid("snapshot cell rings are invalid"));
            }
            edge_incidence[corner.edge] = edge_incidence[corner.edge].saturating_add(1);
        }
    }
    if edge_incidence.into_iter().any(|incidence| incidence != 2) {
        return Err(CacheError::Invalid("snapshot edge incidence is invalid"));
    }

    world
        .plates
        .validate(mesh)
        .and_then(|_| world.crust.validate(&world.plates))
        .and_then(|_| world.kinematics.validate(&world.plates))
        .and_then(|_| world.boundaries.validate(mesh))
        .and_then(|_| world.seafloor_age.validate(mesh))
        .and_then(|_| world.elevation.validate(mesh))
        .map_err(|error| {
            CacheError::InvalidOwned(format!("snapshot stage data is invalid: {error}"))
        })?;

    let cell_lengths = [
        world.base_elevation.cell_elevations.len(),
        world.deformation.cell_deformation.len(),
        world.hotspots.cell_intensities.len(),
        world.hotspots.cell_hotspots.len(),
        world.oceanic_peaks.cell_densities.len(),
        world.oceanic_peaks.cell_kinds.len(),
        world.volcanic_arcs.cell_strengths.len(),
        world.volcanic_arcs.cell_segments.len(),
        world.cratons.cell_strengths.len(),
        world.basins.cell_basins.len(),
        world.geological_elevation.cell_elevations.len(),
        world.isostasy.cell_support.len(),
        world.isostasy.cell_elevations.len(),
        world.solar_forcing.daily_mean_insolation.len(),
        world.solar_forcing.annual_mean_insolation.len(),
        world
            .radiative_equilibrium
            .daily_effective_temperature_kelvin
            .len(),
        world
            .radiative_equilibrium
            .annual_effective_temperature_kelvin
            .len(),
        world.seasonal_thermal.selected_temperature_kelvin.len(),
        world.seasonal_thermal.annual_mean_temperature_kelvin.len(),
        world
            .seasonal_thermal
            .annual_minimum_temperature_kelvin
            .len(),
        world
            .seasonal_thermal
            .annual_maximum_temperature_kelvin
            .len(),
        world.seasonal_thermal.annual_amplitude_kelvin.len(),
        world
            .atmospheric_circulation
            .cell_wind_meters_per_second
            .len(),
        world
            .atmospheric_circulation
            .cell_wind_speed_meters_per_second
            .len(),
        world
            .atmospheric_circulation
            .cell_temperature_gradient_kelvin_per_radian
            .len(),
        world
            .atmospheric_circulation
            .cell_pressure_gradient_acceleration_meters_per_second_squared
            .len(),
        world
            .atmospheric_circulation
            .cell_coriolis_parameter_per_second
            .len(),
        world
            .atmospheric_circulation
            .cell_terrain_steering_fraction
            .len(),
        world.moisture_transport.cell_humidity_kg_per_m2.len(),
        world
            .moisture_transport
            .cell_moisture_capacity_kg_per_m2
            .len(),
        world
            .moisture_transport
            .cell_evaporation_kg_per_m2_per_day
            .len(),
        world
            .moisture_transport
            .cell_precipitation_kg_per_m2_per_day
            .len(),
        world
            .moisture_transport
            .cell_condensation_kg_per_m2_per_day
            .len(),
        world
            .moisture_transport
            .cell_orographic_precipitation_kg_per_m2_per_day
            .len(),
        world.cryosphere.cell_snowfall_kg_per_m2_per_day.len(),
        world.cryosphere.cell_melt_kg_per_m2_per_day.len(),
        world.cryosphere.cell_snow_cover_fraction.len(),
        world.cryosphere.cell_land_ice_cover_fraction.len(),
        world.cryosphere.cell_sea_ice_cover_fraction.len(),
        world.cell_albedo.len(),
    ];
    if cell_lengths.into_iter().any(|length| length != cells)
        || world.seasonal_thermal.annual_sample_count == 0
        || world
            .seasonal_thermal
            .annual_temperature_samples_kelvin
            .len()
            != cells.saturating_mul(world.seasonal_thermal.annual_sample_count)
    {
        return Err(CacheError::Invalid("snapshot field lengths are invalid"));
    }
    if world
        .hotspots
        .cell_hotspots
        .iter()
        .flatten()
        .any(|&hotspot| hotspot >= world.hotspots.hotspots.len())
        || world.hotspots.hotspots.iter().any(|hotspot| {
            hotspot.source_cell >= cells
                || hotspot.plate >= world.plates.plate_count
                || hotspot.trail.iter().any(|trail| trail.cell >= cells)
        })
        || world
            .oceanic_peaks
            .peaks
            .iter()
            .any(|peak| peak.cell >= cells)
        || world
            .volcanic_arcs
            .cell_segments
            .iter()
            .flatten()
            .any(|&segment| segment >= world.volcanic_arcs.segments.len())
        || world.volcanic_arcs.segments.iter().any(|segment| {
            segment.overriding_plate >= world.plates.plate_count
                || segment.boundary_edges.iter().any(|&edge| edge >= edges)
                || segment.boundary_cells.iter().any(|&cell| cell >= cells)
                || segment.arc_cells.iter().any(|arc| arc.cell >= cells)
                || segment.peaks.iter().any(|&cell| cell >= cells)
        })
    {
        return Err(CacheError::Invalid(
            "snapshot sparse field indices are invalid",
        ));
    }
    world
        .hotspots
        .validate(mesh)
        .map_err(|error| CacheError::InvalidOwned(error.to_string()))?;
    world
        .volcanic_arcs
        .validate(mesh)
        .map_err(|error| CacheError::InvalidOwned(error.to_string()))?;
    world
        .cratons
        .validate(mesh)
        .map_err(|error| CacheError::InvalidOwned(error.to_string()))?;
    world
        .basins
        .validate(mesh)
        .map_err(|error| CacheError::InvalidOwned(error.to_string()))?;
    world
        .geological_elevation
        .validate(mesh)
        .map_err(|error| CacheError::InvalidOwned(error.to_string()))?;
    world
        .solar_forcing
        .validate(mesh)
        .map_err(|error| CacheError::InvalidOwned(error.to_string()))?;
    Ok(())
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
            .ok_or(CacheError::Invalid("snapshot ended unexpectedly"))?;
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

number_codec!(u32, u64, f32, f64);

impl CacheCodec for usize {
    fn encode(&self, encoder: &mut Encoder) {
        (*self as u64).encode(encoder);
    }
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
        u64::decode(decoder)?
            .try_into()
            .map_err(|_| CacheError::Invalid("snapshot integer exceeds this platform"))
    }
}

impl<T: CacheCodec> CacheCodec for Vec<T> {
    fn encode(&self, encoder: &mut Encoder) {
        self.len().encode(encoder);
        for value in self {
            value.encode(encoder);
        }
    }
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
        let length = usize::decode(decoder)?;
        if length > MAX_COLLECTION_ITEMS {
            return Err(CacheError::Invalid("snapshot collection is too large"));
        }
        (0..length).map(|_| T::decode(decoder)).collect()
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
            _ => Err(CacheError::Invalid("snapshot option tag is invalid")),
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
            .map_err(|_| CacheError::Invalid("snapshot array length is invalid"))
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
            .map_err(|_| CacheError::Invalid("snapshot string is not UTF-8"))
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
    PlatePartitionConfig { major_plate_count, minor_plate_count, major_head_start_rounds, growth_roughness, seed }
    CrustClassificationConfig { target_ocean_fraction, seed }
    PlateKinematicsConfig { seed, minimum_angular_speed, maximum_angular_speed }
    PlateMigrationConfig { minimum_convergence }
    PlateEvolutionConfig { step_count, migration }
    SeafloorAgeConfig { ridge_less_age }
    BaseElevationConfig { continental_base, ridge_elevation, deep_ocean_elevation, cooling_age }
    BoundaryEffect { offset, depth }
    ContinentalRiftProfile { center_offset, flank_offset, decay_depth }
    BoundaryDeformationConfig { convergent, rift, transform, collision, trench, saturation_speed }
    CoarseElevationConfig { smoothing_passes, smoothing_weight }
    HotspotFieldConfig { hotspot_count, maximum_trail_cells, seed }
    OceanicPeakFieldConfig { maximum_young_age, seamount_density_scale, abyssal_hill_density_scale, maximum_position_offset, maximum_seamount_height, maximum_abyssal_hill_height, seed }
    VolcanicArcFieldConfig { minimum_boundary_edges, inland_offset_cells, peak_density_divisor, strength_saturation }
    CratonFieldConfig { minimum_boundary_distance, ramp_width }
    SedimentaryBasinFieldConfig { maximum_elevation, minimum_cell_count, maximum_ocean_perimeter_fraction }
    GeologicalElevationConfig { hotspot_uplift, volcanic_arc_uplift, craton_flattening, basin_flattening }
    IsostaticAdjustmentConfig { adjustment_strength, continental_support, convergent_support_bonus, divergent_support_penalty, craton_support_bonus, maximum_boundary_distance }
    SolarForcingConfig { orbital_phase, annual_sample_count }
    ClimateAlbedoConfig { land, ocean, snow, ice }
    RadiativeEquilibriumConfig { emissivity }
    SeasonalThermalConfig { land_heat_capacity, ocean_heat_capacity, orbital_period_days }
    AtmosphericCirculationConfig { surface_drag_per_second, terrain_steering, maximum_wind_speed_meters_per_second }
    MoistureTransportConfig { step_count, step_seconds, reference_capacity_kg_per_m2, reference_temperature_kelvin, capacity_temperature_sensitivity_per_kelvin, minimum_capacity_kg_per_m2, maximum_capacity_kg_per_m2, ocean_evaporation_rate_per_second, rainfall_rate_per_second, orographic_coefficient_per_meter, maximum_orographic_fraction_per_step, maximum_transport_fraction_per_step }
    CryosphereConfig { maximum_iterations, closure_tolerance, snowfall_temperature_kelvin, melt_temperature_kelvin, full_snow_cover_kg_per_m2, seasonal_snow_capacity_kg_per_m2, snow_melt_kg_per_m2_per_kelvin_day, land_ice_melt_kg_per_m2_per_kelvin_day, sea_ice_growth_fraction_per_kelvin_day, sea_ice_melt_fraction_per_kelvin_day }
    ClimateCouplingConfig { maximum_iterations, under_relaxation, albedo_tolerance, temperature_tolerance_kelvin, precipitation_tolerance_kg_per_m2_per_day, cover_fraction_tolerance, albedo, radiative_equilibrium, seasonal_thermal, atmospheric_circulation, moisture_transport, cryosphere }
    GenerationSettings { fibonacci, plates, crust, kinematics, evolution, seafloor_age, base_elevation, deformation, elevation, hotspots, oceanic_peaks, volcanic_arcs, cratons, basins, geological_elevation, isostasy, planet, solar_forcing, climate_coupling }

    VoronoiEdge { vertices, cells }
    CellCorner { vertex, neighbor, edge }
    SphereMesh { radius, cell_centers, cell_offsets, corners, cell_areas, vertices, vertex_cells, vertex_neighbors, edges }
    PlatePartition { cell_plates, plate_count }
    CrustClassification { plate_classes }
    PlateKinematics { angular_velocities }
    BoundaryClassification { edge_classes, edge_normal_speeds, edge_shear }
    PlateEvolutionDiagnostics { active_step_count, proposal_count, contested_cell_count, migrated_cell_count, maximum_convergence }
    FieldSummary { minimum, maximum, mean }
    SeafloorAgeDiagnostics { summary, oceanic_cell_count, ridge_cell_count, ridge_plate_count, ridge_less_plate_count, fallback_cell_count }
    SeafloorAge { cell_ages, diagnostics }
    BaseElevationDiagnostics { summary, oceanic, oceanic_cell_count, continental_cell_count }
    BaseElevation { cell_elevations, diagnostics }
    BoundaryDeformationDiagnostics { summary, source_cell_count, uplifted_cell_count, subsided_cell_count }
    BoundaryDeformation { cell_deformation, diagnostics }
    CoarseElevation { cell_elevations, diagnostics }
    HotspotTrailCell { cell, intensity }
    Hotspot { mantle_position, source_cell, plate, trail }
    HotspotDiagnostics { trail_cell_count, affected_cell_count, overlap_cell_count, stationary_source_count, shortest_trail_cells, longest_trail_cells }
    HotspotField { hotspots, cell_intensities, cell_hotspots, diagnostics }
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
    GeologicalElevation { cell_elevations, diagnostics }
    IsostaticAdjustmentDiagnostics { support, elevation, oceanic_cell_count, preserved_basin_cell_count, adjustment }
    IsostaticAdjustment { cell_support, cell_elevations, diagnostics }
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
                    _ => Err(CacheError::Invalid("snapshot enum tag is invalid")),
                }
            }
        }
    };
}

enum_codec!(CrustClass { CrustClass::Oceanic = 0, CrustClass::Continental = 1 });
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

impl CacheCodec for GeneratedWorld {
    fn encode(&self, encoder: &mut Encoder) {
        self.voronoi.encode(encoder);
        self.plates.encode(encoder);
        self.crust.encode(encoder);
        self.kinematics.encode(encoder);
        self.boundaries.encode(encoder);
        self.evolution.encode(encoder);
        self.seafloor_age.encode(encoder);
        self.base_elevation.encode(encoder);
        self.deformation.encode(encoder);
        self.elevation.encode(encoder);
        self.hotspots.encode(encoder);
        self.oceanic_peaks.encode(encoder);
        self.volcanic_arcs.encode(encoder);
        self.cratons.encode(encoder);
        self.basins.encode(encoder);
        self.geological_elevation.encode(encoder);
        self.isostasy.encode(encoder);
        self.solar_forcing.encode(encoder);
        self.radiative_equilibrium.encode(encoder);
        self.seasonal_thermal.encode(encoder);
        self.atmospheric_circulation.encode(encoder);
        self.moisture_transport.encode(encoder);
        self.cryosphere.encode(encoder);
        self.cell_albedo.encode(encoder);
        self.climate_coupling_diagnostics.encode(encoder);
    }

    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CacheError> {
        Ok(Self {
            voronoi: CacheCodec::decode(decoder)?,
            plates: CacheCodec::decode(decoder)?,
            crust: CacheCodec::decode(decoder)?,
            kinematics: CacheCodec::decode(decoder)?,
            boundaries: CacheCodec::decode(decoder)?,
            evolution: CacheCodec::decode(decoder)?,
            seafloor_age: CacheCodec::decode(decoder)?,
            base_elevation: CacheCodec::decode(decoder)?,
            deformation: CacheCodec::decode(decoder)?,
            elevation: CacheCodec::decode(decoder)?,
            hotspots: CacheCodec::decode(decoder)?,
            oceanic_peaks: CacheCodec::decode(decoder)?,
            volcanic_arcs: CacheCodec::decode(decoder)?,
            cratons: CacheCodec::decode(decoder)?,
            basins: CacheCodec::decode(decoder)?,
            geological_elevation: CacheCodec::decode(decoder)?,
            isostasy: CacheCodec::decode(decoder)?,
            solar_forcing: CacheCodec::decode(decoder)?,
            radiative_equilibrium: CacheCodec::decode(decoder)?,
            seasonal_thermal: CacheCodec::decode(decoder)?,
            atmospheric_circulation: CacheCodec::decode(decoder)?,
            moisture_transport: CacheCodec::decode(decoder)?,
            cryosphere: CacheCodec::decode(decoder)?,
            cell_albedo: CacheCodec::decode(decoder)?,
            climate_coupling_diagnostics: CacheCodec::decode(decoder)?,
            timings: GenerationTimings::default(),
            config: GenerationSettings::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn settings(cell_count: usize, seed: u64) -> GenerationSettings {
        GenerationSettings {
            fibonacci: FibonacciConfig {
                jitter: 0.25,
                seed,
                ..FibonacciConfig::new(cell_count)
            },
            plates: PlatePartitionConfig {
                seed,
                ..PlatePartitionConfig::new(2, 2)
            },
            crust: CrustClassificationConfig::new(seed),
            kinematics: PlateKinematicsConfig::new(seed),
            evolution: PlateEvolutionConfig {
                step_count: 4,
                ..Default::default()
            },
            hotspots: HotspotFieldConfig {
                hotspot_count: 3,
                maximum_trail_cells: 4,
                seed,
            },
            oceanic_peaks: OceanicPeakFieldConfig::new(seed),
            ..GenerationSettings::default()
        }
    }

    fn fixture(cell_count: usize, seed: u64) -> (GenerationSettings, GeneratedWorld) {
        let settings = settings(cell_count, seed);
        let world = GeneratedWorld::generate(settings).unwrap();
        (settings, world)
    }

    fn temp_cache(name: &str) -> WorldCache {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        WorldCache::new(env::temp_dir().join(format!(
            "procgen-viewer-{name}-{}-{unique}/world.bin",
            std::process::id()
        )))
    }

    #[test]
    fn snapshot_round_trip_restores_settings_and_domain_data_without_timings() {
        let (settings, world) = fixture(32, 11);
        let bytes = encode_snapshot(&settings, &world);
        let (loaded_settings, loaded) = decode_snapshot(&bytes).unwrap();

        assert_eq!(loaded_settings, settings);
        assert_eq!(loaded.config, settings);
        assert_eq!(loaded.voronoi.cell_centers, world.voronoi.cell_centers);
        assert_eq!(loaded.voronoi.edges, world.voronoi.edges);
        assert_eq!(loaded.plates, world.plates);
        assert_eq!(loaded.isostasy, world.isostasy);
        assert_eq!(loaded.cryosphere, world.cryosphere);
        assert!(loaded.timings.stages().is_empty());
    }

    #[test]
    fn format_and_build_identity_mismatches_invalidate_snapshot() {
        let (settings, world) = fixture(32, 12);

        let mut wrong_version = encode_snapshot(&settings, &world);
        wrong_version[MAGIC.len()] ^= 1;
        assert!(decode_snapshot(&wrong_version).is_err());

        let mut wrong_build = encode_snapshot(&settings, &world);
        let identity_start = MAGIC.len() + size_of::<u32>() + size_of::<u64>();
        wrong_build[identity_start] ^= 1;
        assert!(decode_snapshot(&wrong_build).is_err());
    }

    #[test]
    fn truncated_snapshot_is_nonfatal_corruption() {
        let (settings, world) = fixture(32, 13);
        let mut bytes = encode_snapshot(&settings, &world);
        bytes.truncate(bytes.len() / 2);

        assert!(decode_snapshot(&bytes).is_err());
    }

    #[test]
    fn invalid_topology_and_field_lengths_are_rejected() {
        let (settings, mut world) = fixture(32, 14);
        world.voronoi.edges[0].cells[0] = world.voronoi.cell_count();
        assert!(decode_snapshot(&encode_snapshot(&settings, &world)).is_err());

        let (settings, mut world) = fixture(32, 15);
        world.cell_albedo.pop();
        assert!(decode_snapshot(&encode_snapshot(&settings, &world)).is_err());
    }

    #[test]
    fn successful_store_atomically_replaces_previous_snapshot() {
        let cache = temp_cache("replace");
        let (first_settings, first) = fixture(32, 16);
        let (second_settings, second) = fixture(48, 17);
        cache.store(&first_settings, &first).unwrap();
        cache.store(&second_settings, &second).unwrap();

        let (loaded_settings, loaded) = cache.load().unwrap();
        assert_eq!(loaded_settings, second_settings);
        assert_eq!(loaded.voronoi.cell_count(), 48);
        let parent_entries: Vec<_> = fs::read_dir(cache.path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(parent_entries, [cache.path.file_name().unwrap()]);

        fs::remove_dir_all(cache.path.parent().unwrap()).unwrap();
    }

    #[test]
    fn failed_atomic_write_preserves_previous_snapshot() {
        let cache = temp_cache("failed-replace");
        let (settings, world) = fixture(32, 18);
        cache.store(&settings, &world).unwrap();
        let original = fs::read(&cache.path).unwrap();

        let result = atomic_replace_with(&cache.path, |file| {
            file.write_all(b"incomplete")?;
            Err(io::Error::other("injected write failure"))
        });

        assert!(result.is_err());
        assert_eq!(fs::read(&cache.path).unwrap(), original);
        assert_eq!(
            fs::read_dir(cache.path.parent().unwrap()).unwrap().count(),
            1
        );
        fs::remove_dir_all(cache.path.parent().unwrap()).unwrap();
    }
}
