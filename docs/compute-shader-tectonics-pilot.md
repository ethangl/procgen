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

## Measured so far

Partition through evolution, in a release build on the development MacBook Pro
(Apple M1 Max via Metal) on 2026-09-08, with the default configuration of six
major and 111 minor plates at 99 percent growth roughness, seed 0, a 0.75
target ocean fraction, and nine evolution steps. Each stage is timed from
submission to device idle. The border and migration columns are the whole
evolution: ten classifications and nine migration steps.

| Resolution | Init | Seeds | Growth | Crust | Borders | Migration | Total   |
| ---------- | ---- | ----- | ------ | ----- | ------- | --------- | ------- |
| 128        | 1 ms | 9 ms  | 20 ms  | 1 ms  | 13 ms   | 12 ms     | 56 ms   |
| 256        | 1 ms | 12 ms | 54 ms  | 3 ms  | 28 ms   | 17 ms     | 115 ms  |
| 512        | 1 ms | 22 ms | 197 ms | 1 ms  | 59 ms   | 16 ms     | 297 ms  |
| 1024       | 1 ms | 68 ms | 940 ms | 3 ms  | 198 ms  | 26 ms     | 1236 ms |

Frontier passes are what the longer of the two growth relaxations consumed out
of its budget; the frontier's order varies run to run, so the count moves by a
pass or two while the labels it settles on do not. It measured 115 of 768 at
128 texels per face, 230 of 1536 at 256, 435 of 3072 at 512, and 923 of 6144 at
1024. Pass counts grow with the face resolution rather than with its square,
and the budget clears the measured need by six times.

Growth is dominated by the frontier's latency rather than its arithmetic: a
pass carries a few thousand cells at 1024 texels per face, which occupies a
small fraction of the device. Seeding re-evaluates every texel direction once
per plate, which is why it grows faster than the cell count; folding the
distance-field update into the farthest-point reduction removed one of its two
full passes per plate, and widening that reduction past 256 workgroups costs
more in its serial final step than it recovers. Neither stage is addressed
further yet; the interactivity slice has the measurements it needs to choose.

Evolution is cheap by comparison and scales with the cell count as a
full-raster map should: at 1024 texels per face one boundary classification is
about 20 ms and one migration step about 3 ms, so the nineteen dispatch rounds
together cost about a fifth of growth. The crust reduction is negligible
because a workgroup accumulates its own plate areas before touching the shared
counters.

Boundaries at 256 texels per face do not read as grid-aligned at the default
roughness, and plate areas keep the same distribution across resolutions
because the head start is configured as an arc. The evolution reaches the
requested ocean fraction to within a thousandth at every resolution and moves
roughly four percent of the raster's cells over its nine steps.

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
function of texel coordinates: the lower of the two cells owns the border and
the id is four times that cell plus its own direction toward the other. A seam
can rotate which direction that is, so the owner's direction is resolved
rather than assumed opposite.

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
rather than reused, because their meaning changes. The partition's one such
field, the major plates' head start, is configured as a great-circle arc and
converted to link-length cost against the face resolution, so plate sizes stay
comparable when the resolution changes.

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
pipeline's former two-phase semantics expressed without phases. Minor seeds
are chosen among cells still unclaimed at the head-start cost, which is a
reduction over the growth state, not a readback. Evolution runs the migration
step `step_count` times, reclassifying boundaries between steps.

The mesh pipeline has since moved on: it walks a crack pattern first and uses
growth only to subdivide the resulting faces, while the pilot's growth and its
head start are unchanged, and the fixed-point argument below still covers both
the pilot's growth and that subdivision step.

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

Stage outputs are GPU buffers indexed by cell: ownership, arrival cost,
boundary class, age, deformation, elevation. Per-border results are stored per
cell rather than per edge, because `wgpu`'s default limits cap one storage
binding at 128 MiB and a per-edge array at 1024 texels per face is four times
the cell count. A cell's four boundary classes are two bits each in one word,
and both cells of a border classify it independently and agree, so no reader
resolves the owning edge. Normal speeds are not stored at all: they follow from
the plate angular velocities and the two cells' directions, which are already
resident, and the migration gather recomputes them through the same function
that classified the border.

The kernels share one bind group of one uniform block and eight storage
buffers, which is `wgpu`'s default storage-binding limit exactly. A stage that
needs another buffer consolidates two existing ones rather than adding a ninth;
the per-plate record is already one such consolidation, and so is the ownership
word that carries a cell's current and pending plate together. Outputs stay
resident between runs, and a settings change reruns the pipeline from the first
dirty stage.

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
  comparisons, evaluated in a fixed expression order. The test vector that was
  to verify fused multiply-add contraction disabled instead measured it
  enabled: on 2026-09-08 Metal through `wgpu` contracted every vector in the
  table, and `wgpu` exposes no control over it. Contraction is a property of
  one backend's shader compiler, so it is deterministic within a backend and
  leaves run-to-run and schedule invariance intact; what it puts in question is
  cross-backend agreement, which is now measured rather than assumed. The test
  records which of the two roundings a backend chose and fails only on a third,
  which would mean reassociation or reduced precision. If a class or winner
  flips between the two machines, convergence moves to fixed-point `i32`. That
  is decided in slice 5, by measurement.

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
  `wgpu`, not Bevy, and on `procgen-tectonics` for shared config types such as
  `CoarseElevationConfig`, which carries the sea-level datum. Generation logic
  lives here, never in the app.
- `procgen-gpu-tests` gains `tectonics_pipeline.rs`.
- `apps/raster-viewer` is the pilot application and the intended successor to
  the current viewer. It passes Bevy's device to the pipeline and renders six
  face grids displaced by the elevation buffer and colored by per-cell
  lookups; no fan mesh, no CPU-side vertex rebuild. Grid density is a display
  setting independent of texel resolution, since the grids sample the
  buffers. It exposes the tectonic settings, a face-resolution switch, stage
  timings, and diagnostics read back asynchronously one frame late. Until the
  interactivity slice it submits the pipeline stage by stage on Bevy's device
  and waits, which is what makes per-stage timings measurable without timestamp
  queries; the render-graph node pattern in the current viewer's
  `terrain_tiles/compute.rs` arrives with the preview and the asynchronous
  readback.

The current viewer is frozen: it takes no new features and keeps working on
the Voronoi path until the successor covers what it shows. It accepts the
second-consumer rule, because the alternative is a second copy of everything a
viewer needs, and it has accepted phase-scoped generation, because iterating
tectonic settings on the Voronoi path otherwise pays for geology and climate on
every run. Shared viewer support is lifted into
`procgen-viewer-support` as the pilot app reaches for it, and the orbit
controls and the identity colour ramp are there already.

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
- Ownership and arrival cost share one `u32` per cell: nine bits of plate id
  below twenty-three bits of cost, so the numeric order of the word is the
  lexicographic order of the label and one `atomicMin` is the whole tie rule.
  The reserved plate id makes the unclaimed label `u32::MAX`, and the cost
  field holds the costliest shortest path 1024 texels per face can produce.
- A head start large enough to claim every cell leaves its minor plates
  seedless rather than failing the run. Every cell is still owned; those plates
  simply own none, and the plate buffer records which.
- A border edge's canonical id is four times its lower cell plus that cell's
  own direction toward the other, so both cells name it identically. It orders
  migration ties; it indexes nothing.
- Boundary classes are stored per cell, two bits per border, because both cells
  of a border reach the same class from the same inputs. Normal speeds are
  derived where they are needed rather than stored.
- Plate areas are integers: each cell's solid angle is quantized against a
  fixed scale and summed with atomics, so the crust classification is exact
  integer arithmetic on an order-independent reduction. The solid angle itself
  is a midpoint rule over the gnomonic area element, evaluated with the
  polynomial tangent, because the exact spherical quadrilateral needs an arc
  tangent.
- Crust visits plates in a seeded order the host computes and uploads, since
  the mesh pipeline's order comes from a 64-bit stream no kernel may see.
- Per-plate motion is computed on the host by the mesh pipeline's own
  kinematics function and quantized to a power-of-two grid before upload.
- The kernels sit at `wgpu`'s default limit of eight storage buffers per stage.
  New per-cell state consolidates into an existing buffer rather than adding a
  binding.
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
