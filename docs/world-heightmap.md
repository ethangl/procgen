# Earth-like world heightmap

## Goal

Generate a deterministic, Earth-scale spherical world and derive elevation
products from it:

- Global world-generation heightmap: roughly 15 arc seconds / 512 m per sample.
- Final detailed elevation product: roughly 32 m per sample, comparable in role
  to a global DEM such as COP-DEM-GLO-30.
- Raster dimensions, tile dimensions, and refinement ratios must be powers of
  two, forming a regular resolution pyramid.

The 32 m product is 16 times finer on each axis than the 512 m base and therefore
contains 256 times as many samples. It must be generated and processed in tiles
or chunks rather than as one in-memory global raster.

## Model boundary

The authoritative model lives on a sphere. Equirectangular maps, cube faces,
images, and raw height buffers are projections or export formats—not the core
world representation. Keep global structure separate from local detail
refinement so a coarse result can drive independently generated high-resolution
tiles.

Before implementing the large raster stages, settle the projection, tile size,
and pyramid level used for each product. The power-of-two grid is authoritative;
15 arc seconds, 512 m, and 32 m are nominal scale targets rather than exact grid
spacings. Exact 15-arc-second global equirectangular dimensions would be 86,400
by 43,200 and are intentionally not used.

## Existing reference pipeline

The C# reference currently performs:

1. Fibonacci sphere sampling.
2. Convex hull / spherical Delaunay and Voronoi construction.
3. Plate assignment, motion, boundaries, and coarse elevation.
4. Hotspots, volcanic arcs, cratons, basins, seamounts, and isostasy.
5. Wind and precipitation.
6. Dense spherical terrain and optional subdivision.
7. Equirectangular rasterization.
8. Wrapped blur, global/coastal detail, sharpening, and export.

Do not preserve this decomposition automatically. Use fixed seeds and captured
stage outputs to determine behavior, then give each Rust component the smallest
cohesive API justified by real reuse.

## Port sequence

Port and validate one boundary at a time:

1. Sphere coordinates and deterministic Fibonacci sampling.
2. Spherical topology and mesh invariants.
3. Plate and coarse-elevation state.
4. Geological modifiers and climate, individually.
5. Dense terrain/refinement.
6. Projection and tiled rasterization.
7. Raster detail, filtering, previews, and exports.

Completed foundations:

- `procgen-core`: dependency-free vector math and counter-addressable random
  streams shared by CPU and future GPU implementations.
- `procgen-sphere`: deterministic Y-up Fibonacci sampling.
- `procgen-sphere-mesh`: spherical Delaunay/Voronoi topology, GPU-friendly CSR
  cell-corner rings, and cell areas.
- `procgen-tectonics`: deterministic major/minor plate partitioning, rigid plate
  angular velocities, local spherical tangent motion, static convergent,
  divergent, and transform boundary classification, and static per-plate crust
  classification with area-weighted ocean coverage. It repeatedly reclassifies
  current boundaries and applies deterministic, simultaneous ownership migration
  for a configured step count. The result retains final ownership and boundaries
  plus aggregate evolution diagnostics, without step history. Per-plate crust
  classes remain fixed while cell crust follows current ownership. A separate
  post-evolution stage derives signed per-cell deformation from current-owner
  crust and final boundary classes and strengths, with deterministic overlap
  resolution and bounded within-plate propagation. Continental divergent
  boundaries use a configurable graben profile: strength-scaled central
  subsidence, a steep transition to weaker negative flanks, then bounded decay
  to zero. Oceanic ridges remain solely owned by bathymetry. A separate
  seafloor-age stage derives oceanic-cell hop distance from final divergent
  boundaries; propagation stays within final plate ownership and ridge-less
  oceanic plates receive a deterministic configured fallback age. Base
  elevation then keeps continental cells at their configured base and maps
  oceanic age through a configurable square-root ridge-to-deep cooling curve.
  Tectonic elevation composes that base with boundary deformation once before
  simultaneous smoothing and clamping. None of these stages accumulates state
  during evolution.
- `procgen-geology`: deterministic present-day geological fields derived from
  completed tectonic state without feeding changes back into tectonics. Seeded
  mantle hotspots have bounded decaying trails opposite final-owner plate
  motion, constrained to final plate ownership with stable max-intensity overlap
  resolution. Volcanic arcs group final mixed-crust convergent boundaries by
  overriding continental plate, walk a bounded distance inland, and retain
  strength-ranked peak candidates with stable segment, peak, and overlap
  ordering. Craton strength applies only to above-sea-level continental cells
  and ramps with graph distance from final plate boundaries. It reads but never
  mutates tectonic elevation and does not infer plate-continuity history.
  Sedimentary basins are compact, stable connected-component IDs for low-lying
  continental land, filtered by size and ocean-facing perimeter. The stage
  records component summaries but does not flatten or otherwise modify tectonic
  elevation. Oceanic peak density uses deterministic max-wins overlap between
  hotspot intensity and young nonzero seafloor age. An independent seeded pass
  emits stable sparse seamount and abyssal-hill candidates with positions
  bounded inside their source cells and strength-scaled diagnostic heights; it
  does not stamp cones, mutate elevation, or produce a dense terrain field. A
  separate geological elevation stage copies coarse tectonic elevation, then
  applies hotspot uplift, volcanic-arc uplift, craton flattening toward the
  configured continental base, and basin flattening toward each component's
  original deterministic minimum, in that stable order. It reports the
  aggregate actual delta of each effect and does not consume the sparse oceanic
  peaks or mutate any input field. A final deterministic isostatic stage derives
  per-cell support from current-owner continental crust, convergent and divergent
  boundary proximity, craton strength, basin membership, and geological
  elevation. It produces a separate clamped elevation field, keeps oceanic cells
  unchanged, preserves basin floors, and reports aggregate rise and sink without
  mutating its inputs.
- `procgen-viewer`: diagnostic GPU viewer with retained topology, tectonic
  plate, crust, motion, and boundary layers, orbit controls, deterministic
  regeneration, evolution, deformation, bathymetry, and tectonic-elevation
  controls, signed deformation, seafloor-age, base-elevation,
  tectonic-elevation, and geological-elevation layers, hotspot and volcanic-arc
  field visualization, craton distance/ramp controls and strength
  visualization, sedimentary-basin controls and stable-ID visualization,
  seamount/abyssal density and sparse-peak controls and visualization,
  geological-elevation effect controls and final layer, per-effect aggregate
  diagnostics, isostatic support and adjusted-elevation controls and layers,
  aggregate rise/sink diagnostics, and stage timings.
- `procgen-planet`: minimal SI-unit stellar, orbital, size, rotation, atmosphere,
  and physical land-elevation-scale inputs, with Earth-like values isolated in a
  preset.
- `procgen-climate`: deterministic top-of-atmosphere daily-mean insolation for
  a selected orbital phase plus a bounded-sample, elapsed-time-weighted annual
  mean over spherical cells. A separate stage converts those daily and annual
  fields to effective radiative-equilibrium temperatures using explicit uniform
  albedo, emissivity, and Stefan-Boltzmann radiation. Earthlike values exist only
  as a caller-selected preset. A third stage solves an isolated local seasonal
  surface-energy balance over a periodic orbit using explicit land and ocean
  heat capacities selected from final elevation and sea level. It reports the
  selected phase plus annual mean, extrema, amplitude, and convergence
  diagnostics. A fourth independent stage derives coarse finite tangent winds
  from selected seasonal-temperature gradients, explicit planetary rotation
  and atmospheric gas properties, linear surface drag, and bounded terrain
  steering. It contains no hard-coded latitude bands. These stages model no
  coupled climate feedbacks. A fifth pure fixed-step stage starts with an empty
  atmospheric column, evaporates water from exposed ocean cells according to
  selected temperature and bounded moisture capacity, conservatively transports
  it over mesh edges with the wind field, and removes capacity condensation,
  background rainfall, and bounded terrain-ascent orographic precipitation. It
  reports a spherical-area-weighted mass-balance residual and retains no state
  between runs. A sixth pure stage consumes the sampled annual temperature
  cycle, precipitation climatology, and final elevation/ocean mask to solve
  bounded periodic snow and sea-ice reservoirs. It derives selected-phase snow
  and sea ice plus equilibrium land-ice cover from perennial accumulation and
  ablation potential, with no latitude assignment or coupled feedback. See
  `docs/solar-forcing.md`,
  `docs/radiative-equilibrium-temperature.md`, and
  `docs/seasonal-thermal-response.md`, and
  `docs/coarse-atmospheric-circulation.md`, and
  `docs/moisture-transport.md`, and
  `docs/cryosphere.md`.

The viewer should gain new diagnostic layers as later pipeline attributes are
added. Accelerate a generation stage only when its workload and data layout
justify CPU parallelism or GPU dispatch.
