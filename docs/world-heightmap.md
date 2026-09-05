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
  Coarse elevation composes that base with boundary deformation once before
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
  mutates elevation and does not infer plate-continuity history. Sedimentary
  basins are compact, stable connected-component IDs for low-lying continental
  land, filtered by size and ocean-facing perimeter. Per-basin floor elevation
  is metadata only; the stage does not flatten or otherwise modify elevation.
- `procgen-viewer`: diagnostic GPU viewer with retained topology, tectonic
  plate, crust, motion, and boundary layers, orbit controls, deterministic
  regeneration, evolution, deformation, bathymetry, and coarse-elevation
  controls, signed deformation, seafloor-age, base-elevation, and composed
  elevation layers, hotspot and volcanic-arc field visualization, craton
  distance/ramp controls and strength visualization, sedimentary-basin controls
  and stable-ID visualization, aggregate diagnostics, and stage timings.

The viewer should gain new diagnostic layers as later pipeline attributes are
added. Accelerate a generation stage only when its workload and data layout
justify CPU parallelism or GPU dispatch.
