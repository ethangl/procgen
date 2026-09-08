# Compute-shader tectonics pilot

## Goal

Build tectonics as a GPU-resident pipeline on a cube-sphere raster: plate
partition through tectonic elevation as compute shaders, with results visible
in a viewer as settings change. The resolution target is an eightfold linear
increase over the current mesh, from roughly 80 km cells to 1024 texels per
face at about 10 km, which is 64 times as many cells. The pilot decides
whether geology, climate, and later erosion follow the same path, and it
settles the raster's contracts before those stages depend on them.

The pilot answers four questions, each with a measurable result:

1. Is it deterministic? Integer outputs must be bit-identical run to run,
   under any dispatch schedule, and across the Metal and Vulkan backends.
   Float outputs must agree across backends within recorded tolerances.
2. Is it interactive? Releasing a tectonic slider must update the rendered
   surface within about 250 ms at 1024 texels per face, with a coarser preview
   while dragging.
3. Does it look acceptable? Plate boundaries must not read as grid-aligned and
   distance-driven fields must not show diamond-shaped contours at 1024
   texels per face.
4. Does it produce the same kind of world? Plate count and size distribution,
   ocean fraction, boundary class proportions, and elevation range must be
   comparable to the mesh pipeline's diagnostics. Comparable, not equal; the
   two pipelines share definitions, not results.

A negative answer to any of these is a valid outcome. The pilot exists to find
out cheaply.

## Why tectonics

Tectonics is the root of the pipeline, so everything downstream can consume
raster data once it is there. It is the smallest coherent group of stages. It
is also where the one real unknown lives: whether plate growth on a grid looks
acceptable. Proving or killing that early is worth more than moving the most
expensive stage first.

The stage definitions carry over from the mesh pipeline because they are
already fixed points rather than sequential procedures, which is what makes a
parallel implementation well-defined regardless of how work is scheduled:

- Plate growth is shortest-arrival search over deterministic integer link
  costs. Once the tie rule is order-independent, every schedule converges to
  the same ownership.
- Plate evolution applies one simultaneous migration per step. That is a
  Jacobi update, which is exactly what a compute pass is. Its tie rule is
  explicit: strongest convergence, then lower plate id, then lower edge id.
- Seafloor age is a multi-source distance field. Boundary deformation is
  bounded distance propagation with an explicit tie rule: larger magnitude,
  then lower source cell. Both have unique results.
- Boundary classification, base elevation, and elevation composition are
  per-edge or per-cell maps plus a simultaneous smoothing stencil.

Configs whose meaning is unchanged are reused from `procgen-tectonics` as data
contracts: plate counts, roughness and seed, migration threshold, kinematics,
the cooling curve values, smoothing. The CPU stage functions are not used.

## Current state

The coarse pipeline runs on a 65,536-cell Voronoi mesh on the CPU. Measured
in a release build on the development MacBook Pro:

| Stage group                          | Time    |
| ------------------------------------ | ------- |
| Sampling, Delaunay, Voronoi          | 122 ms  |
| Tectonics (partition to elevation)   | 93 ms   |
| Geology                              | 40 ms   |
| Terrain controls and control bake    | 45 ms   |
| Solar forcing                        | 96 ms   |
| Coupled climate                      | 1048 ms |
| Total                                | 1444 ms |

The cost is not the problem at this cell count. The problem is that
resolution and CPU time are coupled: halving the cell size quadruples the
count, and the graph stages cannot be parallelized on the CPU beyond a modest
factor. The mesh caps the coarse world at roughly 88 km cells, and the control
bake projects that onto 512-texel cube faces at about 20 km, which cannot add
information the mesh does not have.

What exists to build on:

- `procgen-cubesphere` owns the equi-angular mapping, integer tile addressing
  with cross-face edge neighbors, and checked-in WGSL for the mapping and a
  cross-face field sampler. Face resolution `R` corresponds to tile level `L`
  through `R = 64 * 2^L`.
- `procgen-core` owns the 32-bit four-word hash that noise already uses in
  WGSL.
- The viewer dispatches a WGSL compute pass from a Bevy render-graph node for
  terrain tiles and keeps results GPU-resident. It has no readback path yet.
- `procgen-gpu-tests` owns the wgpu device and readback harness used by the
  noise and terrain agreement tests.
- Bevy 0.18 re-exports its wgpu, so a crate built on plain `wgpu` at the same
  version shares the viewer's device.

## The raster

A cube-sphere face resolution `R` defines `6 * R^2` cells. Each texel is a
cell whose center is the equi-angular direction of its texel center and whose
area is its solid angle. Cells are ordered face-major in raster scan order, so
every cell id is a pure function of face and texel coordinates.

| Face resolution | Tile level | Cells     | Spacing at Earth radius |
| --------------- | ---------- | --------- | ----------------------- |
| 128             | 1          | 98,304    | 78 km                   |
| 256             | 2          | 393,216   | 39 km                   |
| 512             | 3          | 1,572,864 | 20 km                   |
| 1024            | 4          | 6,291,456 | 9.8 km                  |

The current mesh corresponds to 128 texels per face. The pilot's default is
1024, the eightfold target. At that resolution a mountain belt a few hundred
kilometers wide spans tens of texels, so deformation profiles become
cross-sections rather than one-cell terraces.

### Why not an equirectangular raster

An equirectangular raster is the simplest possible domain: longitude wraps by
modulo, indexing is trivial, and it is already the export format. It would
work for tectonics. It is not used because of the poles. Texel width shrinks
with the cosine of latitude, so texels are two to one at 60 degrees and
degenerate in the pole rows, and one third of the rows cover 13 percent of the
sphere. Growth fronts and distance contours stretch east to west at high
latitude unless every link cost is corrected for latitude, which turns the
integer cost model into a position-dependent one, and polar plates come out
wrong. It also misses the target: 2048 by 1024 is about 20 km at the equator,
a fourfold improvement, and 4096 by 2048 reaches 10 km with a third more
texels than 1024 texels per face.

The cube-sphere's price is twelve edges and eight corners of special
adjacency, paid once in the texel-neighbor rule, and its mapping, tile
addressing, and WGSL sampler already exist because the terrain tiles live
there. Fields produced on it are consumed by those tiles and by later erosion
on the same pyramid without another projection. Equirectangular remains the
export format, produced by the resampling step the heightmap plan already
names.

### Borders and links

Two kinds of adjacency serve two kinds of stage.

Border edges are the four shared borders of each quad, each with two cells
and a midpoint. Stages that act across a physical boundary use them: boundary
classification, migration proposals, and later moisture flux. Border edges
are ordered per cell in a fixed direction order, so every edge id is a pure
function of texel coordinates.

Links are the eight neighbors a cell can traverse, each with an integer
length. Stages that grow, flood, or measure distance use links: plate growth,
seafloor age, deformation, and later craton ramps and basins. Hop counts on
irregular Voronoi cells are roughly isotropic, but on a 4-connected grid they
are Manhattan distance and every distance-driven field would grow
diamond-shaped contours. The links therefore use chamfer lengths of 5 for axis
neighbors and 7 for diagonal neighbors, which is near-isotropic and stays in
integers, so distance fields are bit-exact by construction. Growth costs scale
with link length before roughness is applied. Configs the mesh pipeline
measures in hops are measured here in link-length units, with defaults derived
from the same physical distances; those fields are owned by the pilot crate
rather than reused, because their meaning changes.

One texel-neighbor rule, in WGSL for the kernels and in Rust for tests,
resolves both borders and links across the twelve cube edges and at the eight
corners, where a cell has seven links instead of eight. Nothing else in the
pipeline knows a face boundary exists.

## Pipeline

Every stage runs on the GPU. Per-plate stages run as single-workgroup kernels
over plate-count entries. The only host work is uploading configs and reading
back diagnostics.

| Stage                   | Formulation                                       |
| ----------------------- | ------------------------------------------------- |
| Plate seeds             | Farthest-point argmax reductions, one per seed    |
| Plate growth            | Frontier shortest-arrival relaxation              |
| Crust classification    | Per-plate area from an integer reduction          |
| Plate kinematics        | Per plate from the seeded stream                  |
| Boundary classification | Per border edge from plate motion                 |
| Migration step          | Per-edge proposals, per-cell winner gather        |
| Seafloor age            | Frontier multi-source distance relaxation         |
| Base elevation          | Per cell                                          |
| Boundary deformation    | Bounded frontier relaxation with source ids       |
| Tectonic elevation      | Compose, simultaneous smoothing, clamp            |

Growth is one shortest-path problem with per-seed start offsets: major seeds
start at zero and minor seeds at the head-start cost. That is the mesh
pipeline's two-phase semantics expressed without phases. Minor seeds are
chosen among cells still unclaimed at the head-start cost, which is a
reduction over the growth state, not a readback. Evolution runs the migration
step `step_count` times, reclassifying boundaries between steps.

Growth and seafloor age are label-correcting relaxations, and at 1024 texels
per face they need on the order of the sphere's diameter in links, a few
thousand passes. A pass over every cell would cost seconds. Instead each pass
touches only the frontier: cells whose label changed in the previous pass,
compacted into a list that drives the next pass through indirect dispatch.
Total work is bounded by the number of relaxations, not passes times cells,
and an empty frontier makes the remaining passes zero-sized, so a fixed upper
bound of passes finishes without a readback. Both relaxations share one kernel
shape: a label of cost and origin, a lexicographic minimum, and a frontier.
Deformation is the same relaxation bounded by its depth, and the smoothing
stencil is a fixed small number of full passes.

Stage outputs are GPU buffers indexed by cell or border edge: ownership,
arrival cost, boundary class and normal speeds, age, deformation, elevation.
At 1024 texels per face they total roughly half a gigabyte, dominated by the
per-edge arrays, which fits both development machines. They stay resident
between runs, and a settings change reruns the pipeline from the first dirty
stage.

## Determinism

There is no CPU reference. Determinism is a property of the kernels
themselves, established by construction and verified by test:

- Every integer result is the fixed point of a lexicographic minimum or a
  per-cell gather in fixed neighbor order. Frontier compaction may order the
  frontier differently on every run, and the result does not depend on that
  order. Integer diagnostics use atomic counters, which are order-independent;
  float diagnostics use fixed-order tree reductions, never float atomics.
- Growth ties break on cost then lower plate id. The mesh pipeline's arrival
  heap breaks ties on insertion order, which has no parallel meaning, so this
  is the one definition the pilot changes.
- Link costs come from the 32-bit four-word hash keyed by canonical link id.
  Costs and path sums are `u32`; the base cost is 100 and roughness is below
  100, so sums stay far from overflow.
- No transcendental function sits on a path that decides an integer. The
  equi-angular mapping needs a tangent, and hardware tangents differ between
  backends; the pipeline evaluates it with a fixed polynomial using add and
  multiply only, in Rust and WGSL alike, so texel directions are bit-identical
  everywhere. The mesh pipeline's measured terrain divergence was attributed
  to exactly this, so the polynomial is useful beyond the pilot.
- Per-plate inputs computed on the host, such as angular velocities, are
  quantized to a fixed grid before upload so platform math libraries cannot
  leak ulp differences into the pipeline.
- Boundary class and migration winners are integer outcomes of float
  comparisons. With identical inputs, add-multiply-compare arithmetic in a
  fixed expression order, and FMA contraction verified disabled by a test
  vector on each backend, they are expected to be bit-identical across Metal
  and Vulkan. The tests assert it. If a flip appears, convergence moves to
  fixed-point `i32`. That is decided by measurement, not in advance.

Float tolerances for deformation and elevation follow the workspace rule: ten
times the measured maximum divergence between the two development GPUs,
recorded beside the constants with adapters and date.

Tests live in `procgen-gpu-tests` and read results back:

- Run-to-run and schedule invariance: the same seed produces bit-identical
  integer buffers across two runs and across different workgroup sizes and
  frontier chunk sizes.
- Fixed-point correctness at small resolution: expected ownership and
  distances computed in test code by a plain shortest-path search over the
  same link rule. That is test scaffolding, not a CPU path.
- Structural invariants at full resolution: every cell owned, every plate
  connected, boundary classes consistent with ownership, age zero exactly at
  divergent oceanic borders, deformation zero beyond its depth, elevation
  clamped.
- Integer fingerprints pinned per seed, which must match on both backends.

## Execution and layout

WGSL through wgpu is the only backend, as `AGENTS.md` requires. It reaches
Metal on macOS and Vulkan on Windows, and every pipeline must run on both.

- `procgen-cubesphere` gains the texel-neighbor rule for borders and links in
  Rust and WGSL, and the deterministic polynomial tangent in both.
- `procgen-raster-tectonics` is new and owns the pipeline: checked-in WGSL
  kernels, buffer layouts, config packing, dispatch sequencing against a
  `wgpu::Device`, and the pilot-owned distance-unit configs. It depends on
  `wgpu`, not Bevy, and on `procgen-tectonics` for shared config types and
  constants such as `SEA_LEVEL`. Generation logic lives here, never in the
  app.
- `procgen-gpu-tests` gains `tectonics_pipeline.rs`.
- `apps/raster-viewer` is the pilot application and the intended successor to
  the current viewer. It passes Bevy's device to the pipeline and renders six
  face grids displaced by the elevation buffer and colored by per-cell
  lookups; no fan mesh, no CPU-side vertex rebuild. Grid density is a display
  setting independent of texel resolution, since the grids sample the
  buffers. It exposes the tectonic settings, a face-resolution switch, stage
  timings, and diagnostics read back asynchronously one frame late. Dispatch
  follows the render-graph node pattern in the current viewer's
  `terrain_tiles/compute.rs`.

The current viewer is frozen. It is not modified, and it keeps working on the
Voronoi path until the successor covers what it shows. Shared viewer support
such as the orbit camera, lighting, and palette is lifted into a small crate
when the second application needs it, which is the second-consumer rule.

The pilot follows the code-quality rules in `AGENTS.md`: stage outputs own
their validation, configs are public data, no speculative surface, one
failure convention per crate, and diagnostics computed from results rather
than hot-loop bookkeeping.

## Success criteria

Decided before writing kernels:

- Integer buffers are bit-identical run to run, across workgroup and frontier
  chunk sizes, and between the MacBook Pro and the RTX 5070, for at least four
  seeds at 256 and 1024 texels per face. Float tolerances are recorded with
  the measured maximum, adapters, and date.
- Slider release to updated surface is under about 250 ms at 1024 texels per
  face on the MacBook Pro, with a coarser preview during drags. The RTX 5070
  number is recorded alongside.
- Side by side with the current viewer at 1024 texels per face: boundaries do
  not read as grid-aligned with the existing growth roughness, and age and
  deformation contours do not read as diamonds. Screenshots and seeds are
  recorded with the evaluation.
- Diagnostics are of the same character as the mesh pipeline's: plate count
  and area distribution, ocean fraction, boundary class proportions, and
  elevation range.

## Risks

- Grid artifacts are the reason the pilot exists. Mitigations in order: the
  existing roughness, 8-connected chamfer links, then higher roughness or
  jittered seed costs. If none suffice, that is the pilot's answer.
- Cross-backend flips in float-decided integers, handled as described under
  Determinism. The RTX 5070 is the second backend, so this is measured only
  when both machines have run the same build; the pilot schedules that
  explicitly rather than discovering it late.
- Growth cost at 1024 texels per face. Frontier passes are the plan; if the
  few thousand dependent dispatches still dominate, growth is defined
  hierarchically: coarse growth, then fine relaxation in a band around coarse
  boundaries seeded from settled interior labels.
- No CPU path. The pilot cannot run on a machine without a GPU, and
  reproducible export depends on backend agreement rather than a canonical
  CPU result. This is workspace policy for raster stages; the pilot is its
  first test.
- wgpu version coupling. The pipeline crate must track the wgpu version Bevy
  re-exports, so Bevy upgrades and pipeline changes move together.
- Scope creep into geology because the results invite it. The pilot ends at
  tectonic elevation.

## Decisions

Settled before implementation:

- The pipeline is GPU-only, as `AGENTS.md` requires for raster stages. No
  CPU implementation of any raster stage is written.
- The raster's own adjacency is the mesh. No `SphereMesh` is built from it and
  `procgen-sphere-mesh` is not modified.
- Border edges for boundaries and migration; 8-connected links with chamfer
  lengths 5 and 7 for growth and distance.
- Growth ties break on cost then plate id. Link costs are 32-bit. Every other
  definition is the mesh pipeline's.
- Texel directions come from a polynomial tangent with add and multiply
  only, identical in Rust and WGSL. Host-computed per-plate inputs are
  quantized before upload.
- Growth and distance relaxations are frontier-based with indirect dispatch.
  No mid-pipeline readback exists.
- wgpu and WGSL are the only GPU backend.
- The pilot app is `apps/raster-viewer`, the successor to the viewer, not a
  permanent second application. The current viewer is frozen.
- Default face resolution is 1024; 128, 256, and 512 remain selectable.

## Non-goals

The pilot adds no geology, climate, erosion, terrain-tile integration, tile
export, CUDA, or CPU implementation of raster stages. It does not modify
`procgen-sphere-mesh`, `procgen-tectonics` beyond exposing shared constants,
the current viewer, or the control bake, and does not decide the fate of
`docs/terrain-detail-refinement.md` slice 16. Those decisions are recorded in
the evaluation.

## Slices

Slices are ordered so that GPU-generated plates are on screen after the second
slice and every later slice adds a visible layer.

### Raster contracts.

1. Add the texel-neighbor rule for borders and links and the deterministic
   polynomial tangent to `procgen-cubesphere`, in Rust and WGSL, with seam and
   corner tests through `procgen-gpu-tests`—without kernels or an application.

### Plates on screen.

2. Add `procgen-raster-tectonics` with seed and growth kernels, and
   `apps/raster-viewer` rendering six face grids colored by ownership at a
   selectable resolution with orbit camera and stage timings. Add run-to-run
   and schedule-invariance tests and the small-resolution shortest-path check.
   First viewable milestone: grid artifacts and growth cost are visible
   here—without evolution or elevation.

### Evolution.

3. Add crust, kinematics, boundary classification, and migration kernels;
   evolution steps; crust and boundary layers; the FMA test vector; and
   structural invariant tests—without elevation.

### Bathymetry.

4. Add seafloor age and base elevation kernels with age and base-elevation
   layers and their invariants—without deformation.

### Relief.

5. Add deformation and elevation kernels, displaced rendering, deformation and
   elevation layers, and the first cross-backend float tolerances, recorded
   once both machines have run the build—without geology.

### Interactivity.

6. Add dirty-stage rerun, a coarser preview during drags, and asynchronous
   readback for diagnostics and timings; 1024 becomes the default—without
   geology or climate.

### Evaluation.

7. Record measurements, screenshots, seeds, and cross-backend results against
   the success criteria; decide go or no-go for geology and climate, the
   future of the Voronoi path, and the terrain plan's export slices; update
   `AGENTS.md` and the terrain-detail plan accordingly.

Each slice should explicitly exclude later consumers and backends.
