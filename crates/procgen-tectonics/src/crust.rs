//! Continental crust as a per-cell property of the mesh, independent of the
//! plate partition.
//!
//! `core_count` cores are hashed onto the sphere at least a minimum chord
//! apart, and the rest of the `nucleus_count` nuclei are satellites scattered
//! around them, most around the first core and fewest around the last, so the
//! continents come out clustered toward one hemisphere and ranked in size.
//! The partition's shortest-arrival growth spreads them over per-edge integer
//! costs until the settled area reaches `continental_fraction` of the sphere.
//! Each nucleus grows at its own hashed pace, a low-frequency field makes some
//! parts of the sphere slower than others, and satellites start late, so a
//! cluster grows together into one lobed continent rather than a ring of
//! discs. Growth is bounded by nothing: it crosses plate boundaries freely,
//! so a continent's edge falls wherever it falls relative to them. An edge
//! inside a plate is a passive margin and one on a boundary is an active
//! margin, and both arise without a rule for either.
//!
//! Cells growth reached are continental and every other cell is oceanic. That
//! is the initial condition alone: from step zero onward a cell's crust is
//! read from the step its crust was created, through [`CellCrust`], so a
//! rifting continent grows an oceanic margin and an overridden cell takes the
//! overriding material's class without anything storing a second answer.
//! Plates have no crust class; the readers that want a plate-level number
//! take [`CrustClassification::plate_continental_fraction`].
//!
//! Determinism: the nuclei are hashes, half-angle rotations, and point
//! location on the same floats; the arrival costs are integers scaled by
//! hashed integer ratios and by a polynomial noise field quantized once; and
//! the area budget is an f64 sum compared against an f64 target. So the mask
//! is bit-identical across machines, as the crack walk is.

use crate::{
    MAX_GROWTH_ROUGHNESS, PlatePartition, StageInputError,
    partition::{
        BASE_GROWTH_COST, COST_SCALE_ONE, GrowthBounds, GrowthCosts, GrowthScales, PlateGrowth,
    },
};
use procgen_core::{
    RandomStream, Vec3,
    random_streams::{CRUST_COST_FIELD, CRUST_GROWTH_COST, CRUST_NUCLEUS, CRUST_NUCLEUS_COST},
};
use procgen_noise::{
    OctaveConfig, OctaveGain, Validated, amplitude_sum, fbm_3d, fold_seed_u64_to_u32,
};
use procgen_sphere_mesh::{SphereMesh, connected_components, default_hop_length, hops};
use std::{cmp::Ordering, collections::VecDeque, fmt};

/// Chord length on the unit sphere two cores must be apart, about 46 degrees,
/// so five cores cannot all crowd into one quarter of the sphere. Each core
/// is drawn until it clears every earlier core by this, or the farthest of
/// its draws is taken.
const MIN_CORE_SEPARATION: f32 = 0.78;
/// Draws per core before the farthest draw so far is taken as it is.
const CORE_PLACEMENT_DRAWS: u64 = 64;
/// Draws per satellite before the nearest free cell to its last draw is
/// taken; a spread of zero lands every draw on the core itself.
const SATELLITE_PLACEMENT_DRAWS: u64 = 8;
/// Every nucleus's edge costs are scaled by a hashed ratio in one up to this,
/// so a slow nucleus grows a small continent and a fast one a large one.
const MAX_NUCLEUS_COST_RATIO: u64 = 3;
/// The cost field's lattice frequency on the unit sphere: about one feature
/// per continent, so it bends outlines rather than roughening coasts.
const COST_FIELD_FREQUENCY: f32 = 1.0;
const COST_FIELD_OCTAVES: u32 = 3;
const COST_FIELD_LACUNARITY: f32 = 2.0;
const COST_FIELD_OCTAVE_GAIN: f32 = 0.5;
/// The cost field scales costs by one plus or minus up to this: a quarter
/// pace in the slowest places, one and three quarters in the fastest.
const COST_FIELD_RELIEF: f32 = 0.75;

/// How far the cores have grown when their satellites start, as a model
/// length on the unit sphere. Late enough that a satellite adds a lobe to its
/// core rather than meeting it as an equal, early enough that it still adds
/// one; the stage converts it to hops and hops to growth cost.
fn satellite_head_start() -> f32 {
    5.0 * default_hop_length()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CrustClass {
    Oceanic,
    Continental,
}

impl CrustClass {
    pub const ALL: [Self; 2] = [Self::Oceanic, Self::Continental];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrustClassificationConfig {
    /// Target fraction of the sphere's surface covered by continental crust.
    ///
    /// Not the land fraction: base elevation's margin taper leaves the
    /// outermost continental cells below the default sea level, so the land a
    /// world shows is smaller than this. The default is the value that leaves
    /// the viewer's land count where it stood before the taper existed; see
    /// `docs/land-and-ocean.md` for the measurement.
    pub continental_fraction: f32,
    /// Continental nuclei growth starts from, the order of Earth's cratons:
    /// `core_count` cores and the rest satellites clustered around them. Must
    /// be at least `core_count` and at most the mesh's cell count.
    pub nucleus_count: usize,
    /// Cores the satellites cluster around, each the start of a continent of
    /// its own. A satellite joins core `r` with weight `1 / (r + 1)`, so the
    /// first core gathers the largest continent and the last the smallest.
    /// Must be at least one and at most `nucleus_count`.
    pub core_count: usize,
    /// Mean angle on the unit sphere, in radians, from a satellite to its
    /// core. Each draw is the sum of two uniform variates, so satellites land
    /// anywhere from the core out to twice this; the rotation is applied
    /// through its half-angle tangent, which is within one percent of the
    /// angle at the default. Must be finite and not negative.
    pub cluster_spread: f32,
    /// Maximum percentage that an edge's deterministic traversal cost varies
    /// above or below the baseline, in the partition's units and bounded by
    /// `MAX_GROWTH_ROUGHNESS`. Zero grows smooth coasts.
    pub growth_roughness: u32,
    pub seed: u64,
}

impl CrustClassificationConfig {
    pub const fn new(seed: u64) -> Self {
        Self {
            continental_fraction: 0.347,
            nucleus_count: 24,
            core_count: 5,
            cluster_spread: 0.35,
            growth_roughness: MAX_GROWTH_ROUGHNESS,
            seed,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CrustClassificationDiagnostics {
    /// Continental share of the sphere's area the growth achieved, which
    /// exceeds the target by at most one cell's area.
    pub continental_fraction: f32,
    /// Connected components of continental crust, which is fewer than
    /// `nucleus_count` when two nuclei grew together.
    pub component_count: usize,
    /// Share of the sphere's area the largest continent covers.
    pub largest_component_fraction: f32,
}

/// Per-cell crust class before step zero.
#[derive(Clone, Debug, PartialEq)]
pub struct CrustClassification {
    pub cell_classes: Vec<CrustClass>,
    pub diagnostics: CrustClassificationDiagnostics,
}

impl CrustClassification {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), StageInputError> {
        if self.cell_classes.len() != mesh.cell_count() {
            return Err(StageInputError::CrustClasses);
        }
        Ok(())
    }

    pub fn class(&self, cell: usize) -> CrustClass {
        self.cell_classes[cell]
    }

    /// Area-weighted continental share of each plate, for the readers that
    /// want a plate-level number out of a per-cell fact. Every plate owns a
    /// cell, so every share is defined.
    pub fn plate_continental_fraction(
        &self,
        mesh: &SphereMesh,
        partition: &PlatePartition,
    ) -> Vec<f64> {
        let mut continental = vec![0.0; partition.plate_count];
        for (cell, &plate) in partition.cell_plates.iter().enumerate() {
            if self.cell_classes[cell] == CrustClass::Continental {
                continental[plate] += f64::from(mesh.cell_areas[cell]);
            }
        }
        for (share, area) in continental.iter_mut().zip(partition.plate_areas(mesh)) {
            *share /= area;
        }
        continental
    }
}

/// Per-cell crust class after evolution, read from the model time at which
/// each cell's crust was created: crust with a birth time is oceanic, crust
/// with none is original continental crust that has never been re-made.
///
/// This is the only per-cell answer from step zero onward.
/// [`CrustClassification`] is the initial condition it starts from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellCrust<'a> {
    pub cell_birth: &'a [Option<f32>],
}

impl CellCrust<'_> {
    pub fn validate(&self, mesh: &SphereMesh) -> Result<(), StageInputError> {
        if self.cell_birth.len() != mesh.cell_count() {
            return Err(StageInputError::CrustBirth);
        }
        Ok(())
    }

    pub fn class(&self, cell: usize) -> CrustClass {
        match self.cell_birth[cell] {
            Some(_) => CrustClass::Oceanic,
            None => CrustClass::Continental,
        }
    }

    /// [`material_order`] over the crust two cells carry.
    pub fn order(&self, left: usize, right: usize) -> Ordering {
        material_order(self.cell_birth[left], self.cell_birth[right])
    }

    /// Cells of each crust class, in [`CrustClass::ALL`] order. Both counts
    /// come from one scan, because a consumer showing either usually shows
    /// both.
    pub fn cell_counts(&self) -> [usize; CrustClass::ALL.len()] {
        let oceanic = self
            .cell_birth
            .iter()
            .filter(|birth| birth.is_some())
            .count();
        [oceanic, self.cell_birth.len() - oceanic]
    }
}

/// Which of two parcels of crust covers the other where they meet:
/// continental over oceanic, and among oceanic the younger over the older.
/// `Equal` is two continents, or two floors of one age, and means no
/// polarity.
///
/// The arguments are birth times, as [`CellCrust`] stores them: `None` is
/// continental crust that was never re-made and outranks any ocean floor.
/// Transport reads this to decide which parcel owns a contested cell,
/// deformation to decide which side of a convergent edge gets the trench, and
/// the volcanic arcs to decide which plate the arc sits on, so the three
/// stages cannot disagree about who is on top.
pub fn material_order(left: Option<f32>, right: Option<f32>) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => left.total_cmp(&right),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrustClassificationError {
    InvalidContinentalFraction,
    InvalidNucleusCount,
    InvalidCoreCount,
    InvalidClusterSpread,
    InvalidGrowthRoughness,
}

impl fmt::Display for CrustClassificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidContinentalFraction => {
                formatter.write_str("continental fraction must be finite and between 0 and 1")
            }
            Self::InvalidNucleusCount => formatter.write_str(
                "nucleus count must be at least the core count and at most the mesh cell count",
            ),
            Self::InvalidCoreCount => {
                formatter.write_str("core count must be at least one and at most the nucleus count")
            }
            Self::InvalidClusterSpread => {
                formatter.write_str("cluster spread must be finite and not negative")
            }
            Self::InvalidGrowthRoughness => write!(
                formatter,
                "crust growth roughness cannot exceed {MAX_GROWTH_ROUGHNESS}%"
            ),
        }
    }
}

impl std::error::Error for CrustClassificationError {}

/// Grows continental nuclei to a target share of the sphere and calls every
/// cell they reached continental.
///
/// The nuclei are `core_count` hashed cores and satellites clustered around
/// them, placed without reference to plates. Growth settles the cheapest
/// arrival until the settled area passes the target, so the achieved area
/// overshoots by at most one cell.
pub fn classify_crust(
    mesh: &SphereMesh,
    config: CrustClassificationConfig,
) -> Result<CrustClassification, CrustClassificationError> {
    if !config.continental_fraction.is_finite()
        || !(0.0..=1.0).contains(&config.continental_fraction)
    {
        return Err(CrustClassificationError::InvalidContinentalFraction);
    }
    if config.core_count == 0 || config.core_count > config.nucleus_count {
        return Err(CrustClassificationError::InvalidCoreCount);
    }
    if config.nucleus_count > mesh.cell_count() {
        return Err(CrustClassificationError::InvalidNucleusCount);
    }
    if !config.cluster_spread.is_finite() || config.cluster_spread < 0.0 {
        return Err(CrustClassificationError::InvalidClusterSpread);
    }
    if config.growth_roughness > MAX_GROWTH_ROUGHNESS {
        return Err(CrustClassificationError::InvalidGrowthRoughness);
    }

    let nuclei = crust_nuclei(mesh, config);
    let mut growth = PlateGrowth::new(
        mesh,
        GrowthBounds::WholeSphere,
        GrowthCosts {
            roughness: config.growth_roughness,
            stream: RandomStream::new(config.seed, CRUST_GROWTH_COST),
            scales: GrowthScales {
                region: nucleus_cost_ratios(config),
                cell: cost_field(mesh, config.seed),
            },
        },
    );
    let head_start = hops(mesh.cell_count(), satellite_head_start()) as u64 * BASE_GROWTH_COST;
    for (nucleus, &cell) in nuclei.iter().enumerate() {
        let start = if nucleus < config.core_count {
            0
        } else {
            head_start
        };
        growth.seed(cell, nucleus, start);
    }
    let continental_area =
        growth.grow_to_area(f64::from(config.continental_fraction) * mesh.total_area());

    let cell_classes: Vec<_> = (0..mesh.cell_count())
        .map(|cell| match growth.reached(cell) {
            true => CrustClass::Continental,
            false => CrustClass::Oceanic,
        })
        .collect();
    let components = connected_components(
        mesh,
        |cell| cell_classes[cell] == CrustClass::Continental,
        |_, _| true,
    );
    let largest_component_area = components
        .iter()
        .map(|component| {
            component
                .iter()
                .map(|&cell| f64::from(mesh.cell_areas[cell]))
                .sum::<f64>()
        })
        .fold(0.0, f64::max);
    Ok(CrustClassification {
        cell_classes,
        diagnostics: CrustClassificationDiagnostics {
            continental_fraction: (continental_area / mesh.total_area()) as f32,
            component_count: components.len(),
            largest_component_fraction: (largest_component_area / mesh.total_area()) as f32,
        },
    })
}

/// The cells the continents grow from: the first `core_count` are cores and
/// the rest satellites, in seeding order. Every nucleus is a distinct cell,
/// which `nucleus_count` at most the cell count makes possible.
///
/// A core is drawn at random until it clears every earlier core by
/// `MIN_CORE_SEPARATION`, or the farthest of `CORE_PLACEMENT_DRAWS` draws is
/// taken. A satellite picks its core by the `1 / (r + 1)` weights, a tangent
/// direction from three signed hashes, and an angle of `cluster_spread`
/// times the sum of two unit hashes, then lands on the cell under the rotated
/// core direction. A draw that lands on a taken cell is redrawn, and after
/// `SATELLITE_PLACEMENT_DRAWS` draws the nearest free cell to the last one is
/// taken instead.
pub(crate) fn crust_nuclei(mesh: &SphereMesh, config: CrustClassificationConfig) -> Vec<usize> {
    let stream = RandomStream::new(config.seed, CRUST_NUCLEUS);
    let cell_count = mesh.cell_count();
    let direction = |cell: usize| mesh.cell_centers[cell].normalized();
    let mut nuclei = Vec::with_capacity(config.nucleus_count);
    let mut taken = vec![false; cell_count];

    let min_separation_squared = MIN_CORE_SEPARATION * MIN_CORE_SEPARATION;
    for core in 0..config.core_count {
        let item = core as u64;
        let mut best = (usize::MAX, f32::MIN);
        for draw in 0..CORE_PLACEMENT_DRAWS {
            let cell = (stream.sample_u64(item, draw) % cell_count as u64) as usize;
            let position = direction(cell);
            let separation = nuclei
                .iter()
                .map(|&other| direction(other).distance_squared(position))
                .fold(f32::MAX, f32::min);
            if separation > best.1 {
                best = (cell, separation);
            }
            if separation >= min_separation_squared {
                break;
            }
        }
        // A draw on a taken cell has zero separation, so it is the best draw
        // only when every draw landed on a taken cell; then its nearest free
        // neighbor stands in.
        let cell = nearest_free_cell(mesh, &taken, best.0);
        taken[cell] = true;
        nuclei.push(cell);
    }

    let weights: Vec<f32> = (0..config.core_count)
        .map(|rank| 1.0 / (rank as f32 + 1.0))
        .collect();
    let weight_sum: f32 = weights.iter().sum();
    let mut triangle_hint = 0;
    for satellite in config.core_count..config.nucleus_count {
        let item = satellite as u64;
        let mut pick = stream.unit_f32(item, 0) * weight_sum;
        let mut core = 0;
        while core + 1 < config.core_count && pick >= weights[core] {
            pick -= weights[core];
            core += 1;
        }
        let center = direction(nuclei[core]);
        let mut last = nuclei[core];
        let mut cell = None;
        for draw in 0..SATELLITE_PLACEMENT_DRAWS {
            let sample = 1 + draw * 5;
            let random = Vec3::new(
                stream.signed_f32(item, sample),
                stream.signed_f32(item, sample + 1),
                stream.signed_f32(item, sample + 2),
            );
            let tangent = (random - center * random.dot(center)).normalized();
            let angle = config.cluster_spread
                * (stream.unit_f32(item, sample + 3) + stream.unit_f32(item, sample + 4));
            let position = center.rotated_toward(tangent, 0.5 * angle);
            let location = mesh.locate_delaunay(position, triangle_hint);
            triangle_hint = location.triangle;
            last = location.nearest_cell(mesh, position);
            if !taken[last] {
                cell = Some(last);
                break;
            }
        }
        let cell = cell.unwrap_or_else(|| nearest_free_cell(mesh, &taken, last));
        taken[cell] = true;
        nuclei.push(cell);
    }
    nuclei
}

/// The nearest cell to `start` in hops that no nucleus holds, `start` itself
/// included; ties go to the first found in breadth-first corner order. There
/// is one because there are more cells than nuclei.
fn nearest_free_cell(mesh: &SphereMesh, taken: &[bool], start: usize) -> usize {
    let mut visited = vec![false; taken.len()];
    let mut queue = VecDeque::from([start]);
    visited[start] = true;
    while let Some(cell) = queue.pop_front() {
        if !taken[cell] {
            return cell;
        }
        for corner in mesh.cell_corners(cell) {
            if !visited[corner.neighbor] {
                visited[corner.neighbor] = true;
                queue.push_back(corner.neighbor);
            }
        }
    }
    unreachable!("more cells than nuclei")
}

/// Each nucleus's cost scale in units of [`COST_SCALE_ONE`], hashed uniformly
/// from one up to `MAX_NUCLEUS_COST_RATIO`.
fn nucleus_cost_ratios(config: CrustClassificationConfig) -> Vec<u64> {
    let stream = RandomStream::new(config.seed, CRUST_NUCLEUS_COST);
    (0..config.nucleus_count)
        .map(|nucleus| {
            COST_SCALE_ONE
                + stream.sample_u64(nucleus as u64, 0)
                    % (COST_SCALE_ONE * (MAX_NUCLEUS_COST_RATIO - 1))
        })
        .collect()
}

/// Each cell's cost scale in units of [`COST_SCALE_ONE`]: one plus the
/// low-frequency octave sum, normalized to `COST_FIELD_RELIEF` at the
/// amplitude sum and clamped there. Quantized to the integer scale once, so
/// growth never compares the floats again.
fn cost_field(mesh: &SphereMesh, seed: u64) -> Vec<u64> {
    let key = fold_seed_u64_to_u32(RandomStream::new(seed, CRUST_COST_FIELD).sample_u64(0, 0));
    let octaves = cost_field_octaves();
    let gain =
        OctaveGain::new(COST_FIELD_OCTAVE_GAIN).expect("a halving gain is in the unit interval");
    let normalization = COST_FIELD_RELIEF / amplitude_sum(octaves, gain);
    mesh.cell_centers
        .iter()
        .map(|&center| {
            let relief = (fbm_3d(key, center.normalized(), octaves, gain).value * normalization)
                .clamp(-COST_FIELD_RELIEF, COST_FIELD_RELIEF);
            (COST_SCALE_ONE as f32 * (1.0 + relief)) as u64
        })
        .collect()
}

/// The cost field's octave progression, from constants chosen within the
/// noise crate's limits, so validation cannot fail.
fn cost_field_octaves() -> Validated<OctaveConfig> {
    OctaveConfig {
        octaves: COST_FIELD_OCTAVES,
        frequency: COST_FIELD_FREQUENCY,
        lacunarity: COST_FIELD_LACUNARITY,
    }
    .validate()
    .expect("the cost field's octaves are within the noise crate's limits")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{fingerprint, mesh, reference_partition};
    use procgen_sphere_mesh::mean_cell_width;

    fn class_fingerprint(crust: &CrustClassification) -> u64 {
        fingerprint(crust.cell_classes.iter().map(|&class| class as u8 as u64))
    }

    #[test]
    fn classification_is_deterministic_and_seeded() {
        let mesh = mesh(512);
        let config = CrustClassificationConfig::new(17);
        let first = classify_crust(&mesh, config).unwrap();

        assert_eq!(first, classify_crust(&mesh, config).unwrap());
        assert_ne!(
            first.cell_classes,
            classify_crust(&mesh, CrustClassificationConfig { seed: 18, ..config })
                .unwrap()
                .cell_classes
        );
    }

    #[test]
    fn reference_classification_has_stable_fingerprint() {
        let mesh = mesh(512);
        let crust = classify_crust(&mesh, CrustClassificationConfig::new(17)).unwrap();

        // Hashes, integer arrival costs, and f64 area sums only, so this value
        // is expected to match on both the macOS and the Windows development
        // machine.
        assert_eq!(class_fingerprint(&crust), 14_641_724_092_776_327_602);
        assert_eq!(crust.diagnostics.component_count, 2);
    }

    #[test]
    fn growth_stops_within_one_cell_of_the_target_area() {
        let mesh = mesh(512);
        // One cell's area is about a five-hundredth of the sphere, so the
        // largest cell bounds the overshoot at every fraction.
        let widest_cell =
            f64::from(mesh.cell_areas.iter().copied().fold(f32::MIN, f32::max)) / mesh.total_area();
        for fraction in [0.1, 0.3, 0.6] {
            let crust = classify_crust(
                &mesh,
                CrustClassificationConfig {
                    continental_fraction: fraction,
                    ..CrustClassificationConfig::new(17)
                },
            )
            .unwrap();
            let achieved = f64::from(crust.diagnostics.continental_fraction);
            assert!(
                achieved >= f64::from(fraction) && achieved - f64::from(fraction) <= widest_cell,
                "fraction {fraction} achieved {achieved}"
            );
        }
    }

    #[test]
    fn every_continental_cell_grew_from_a_nucleus() {
        let mesh = mesh(512);
        let config = CrustClassificationConfig::new(17);
        let crust = classify_crust(&mesh, config).unwrap();
        let nuclei = crust_nuclei(&mesh, config);

        let components = connected_components(
            &mesh,
            |cell| crust.class(cell) == CrustClass::Continental,
            |_, _| true,
        );
        assert_eq!(components.len(), crust.diagnostics.component_count);
        assert!(crust.diagnostics.component_count >= 1);
        assert!(crust.diagnostics.component_count <= config.nucleus_count);
        // Every component holds a nucleus and every continental cell is in a
        // component, so each grew from a nucleus through continental crust
        // alone. Two nuclei in one component are two continents that met.
        for component in &components {
            assert!(
                component.iter().any(|cell| nuclei.contains(cell)),
                "a continent with no nucleus"
            );
        }
        assert_eq!(
            components.iter().map(Vec::len).sum::<usize>(),
            crust
                .cell_classes
                .iter()
                .filter(|&&class| class == CrustClass::Continental)
                .count()
        );
        assert!(
            nuclei
                .iter()
                .all(|&cell| crust.class(cell) == CrustClass::Continental)
        );
    }

    #[test]
    fn nuclei_are_distinct_and_satellites_sit_near_a_core() {
        let mesh = mesh(512);
        let config = CrustClassificationConfig::new(17);
        // Any rotation is bounded by its angle, and the nearest cell to a
        // point is within a cell's width of it, so a satellite is never
        // farther from its core than the widest draw plus one hop.
        let reach = 2.0 * config.cluster_spread + mean_cell_width(1.0, mesh.cell_count());
        // A zero spread lands every draw on the core itself, so every
        // satellite takes the fallback and they must still be distinct.
        for config in [
            config,
            CrustClassificationConfig {
                cluster_spread: 0.0,
                ..config
            },
        ] {
            let nuclei = crust_nuclei(&mesh, config);
            assert_eq!(nuclei.len(), config.nucleus_count);
            let mut sorted = nuclei.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), nuclei.len(), "a cell holds two nuclei");

            let (cores, satellites) = nuclei.split_at(config.core_count);
            let direction = |cell: usize| mesh.cell_centers[cell].normalized();
            let farthest_chord = satellites
                .iter()
                .map(|&satellite| {
                    cores
                        .iter()
                        .map(|&core| direction(core).distance_squared(direction(satellite)))
                        .fold(f32::MAX, f32::min)
                })
                .fold(0.0, f32::max)
                .sqrt();
            assert!(
                farthest_chord <= reach,
                "a satellite {farthest_chord} from every core, reach {reach}, spread {}",
                config.cluster_spread
            );
        }
    }

    #[test]
    fn plate_shares_average_to_the_achieved_fraction() {
        let (mesh, partition) = reference_partition();
        let crust = classify_crust(&mesh, CrustClassificationConfig::new(17)).unwrap();

        let shares = crust.plate_continental_fraction(&mesh, &partition);
        assert_eq!(shares.len(), partition.plate_count);
        assert!(shares.iter().all(|share| (0.0..=1.0).contains(share)));
        // A continent that ends inside a plate leaves that plate partly
        // continental, which is the passive margin the stage exists for.
        assert!(
            shares.iter().any(|&share| share > 0.0 && share < 1.0),
            "no plate carries both crusts"
        );
        let weighted: f64 = shares
            .iter()
            .zip(partition.plate_areas(&mesh))
            .map(|(share, area)| share * area)
            .sum();
        assert!(
            (weighted / mesh.total_area() - f64::from(crust.diagnostics.continental_fraction))
                .abs()
                < 1.0e-6,
            "plate shares do not sum to the achieved fraction"
        );
    }

    #[test]
    fn cell_crust_reads_the_birth_time_and_not_plate_ownership() {
        let mesh = mesh(32);
        let cell_birth: Vec<_> = (0..mesh.cell_count())
            .map(|cell| (cell % 3 != 0).then_some(-(cell as f32)))
            .collect();
        let crust = CellCrust {
            cell_birth: &cell_birth,
        };

        assert_eq!(crust.validate(&mesh), Ok(()));
        assert_eq!(crust.class(0), CrustClass::Continental);
        assert_eq!(crust.class(1), CrustClass::Oceanic);
        // Two cells in three are oceanic, so the counts also pin their order.
        let [oceanic, continental] = crust.cell_counts();
        assert_eq!(oceanic + continental, mesh.cell_count());
        assert!(oceanic > continental);
        assert_eq!(
            CellCrust {
                cell_birth: &cell_birth[1..]
            }
            .validate(&mesh),
            Err(StageInputError::CrustBirth)
        );
    }

    #[test]
    fn fraction_extremes_classify_every_cell() {
        let mesh = mesh(512);
        let config = CrustClassificationConfig::new(1);
        let continental = classify_crust(
            &mesh,
            CrustClassificationConfig {
                continental_fraction: 1.0,
                ..config
            },
        )
        .unwrap();
        let oceanic = classify_crust(
            &mesh,
            CrustClassificationConfig {
                continental_fraction: 0.0,
                ..config
            },
        )
        .unwrap();

        assert!(
            continental
                .cell_classes
                .iter()
                .all(|&class| class == CrustClass::Continental)
        );
        assert_eq!(continental.diagnostics.component_count, 1);
        // The nuclei themselves are settled before the budget is consulted, so
        // a zero fraction leaves exactly them continental.
        assert_eq!(
            oceanic
                .cell_classes
                .iter()
                .filter(|&&class| class == CrustClass::Continental)
                .count(),
            config.nucleus_count
        );
    }

    #[test]
    fn rejects_invalid_configurations() {
        let mesh = mesh(64);
        let valid = CrustClassificationConfig::new(0);
        for (config, expected) in [
            (
                CrustClassificationConfig {
                    continental_fraction: -0.1,
                    ..valid
                },
                CrustClassificationError::InvalidContinentalFraction,
            ),
            (
                CrustClassificationConfig {
                    continental_fraction: 1.1,
                    ..valid
                },
                CrustClassificationError::InvalidContinentalFraction,
            ),
            (
                CrustClassificationConfig {
                    continental_fraction: f32::NAN,
                    ..valid
                },
                CrustClassificationError::InvalidContinentalFraction,
            ),
            (
                CrustClassificationConfig {
                    nucleus_count: mesh.cell_count() + 1,
                    ..valid
                },
                CrustClassificationError::InvalidNucleusCount,
            ),
            (
                CrustClassificationConfig {
                    core_count: 0,
                    ..valid
                },
                CrustClassificationError::InvalidCoreCount,
            ),
            (
                CrustClassificationConfig {
                    core_count: valid.nucleus_count + 1,
                    ..valid
                },
                CrustClassificationError::InvalidCoreCount,
            ),
            (
                CrustClassificationConfig {
                    cluster_spread: -0.1,
                    ..valid
                },
                CrustClassificationError::InvalidClusterSpread,
            ),
            (
                CrustClassificationConfig {
                    cluster_spread: f32::NAN,
                    ..valid
                },
                CrustClassificationError::InvalidClusterSpread,
            ),
            (
                CrustClassificationConfig {
                    growth_roughness: MAX_GROWTH_ROUGHNESS + 1,
                    ..valid
                },
                CrustClassificationError::InvalidGrowthRoughness,
            ),
        ] {
            assert_eq!(classify_crust(&mesh, config), Err(expected));
        }

        let crust = classify_crust(&mesh, valid).unwrap();
        assert_eq!(crust.validate(&mesh), Ok(()));
        assert_eq!(
            CrustClassification {
                cell_classes: crust.cell_classes[1..].to_vec(),
                ..crust
            }
            .validate(&mesh),
            Err(StageInputError::CrustClasses)
        );
    }
}
