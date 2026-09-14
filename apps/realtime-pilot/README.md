# Real-time world pilot

Slices 1–5 of [the pilot design](../../docs/realtime-world-pilot.md). This is a
separate application. It uses `procgen-core` and the CPU gradient-noise primitive
from `procgen-noise`, plus cube-face geometry from `procgen-cubesphere`.
It does not use the existing world pipeline or viewer.

## Physical planet and octave editor

```sh
cargo run -p procgen-realtime-pilot -- --design
```

This mode uses meters and starts with a 2,000 km radius. Edit the radius and
individual noise bands, switch between **Planet** and **Local patch**, and use
**Solo** to inspect a band. **Auto apply** regenerates after edits. The octave
panel shows which bands the current preview resolution can display.

Use **Save controls** or **Copy JSON** to share exact generation settings. Load a
saved design with `--design --design-file PATH`. Use `--design --check` for a
headless report; `--write-design PATH` saves the config. Run `--design --help`
for patch and solo options. With auto apply disabled, press Generate after Load.

This is a coarse height preview, with no height exaggeration. The headless chunk
audit below samples one-meter voxels. A headless travel audit also exercises
bounded density residency. The exploration mode below connects these stages; `--stream`
still runs the earlier radius-4 experiment. See [physical scale and octree LOD](../../docs/realtime-world-planet-scale.md).

## Octree chunk audit

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --design --check-chunks
```

Add `--design-file PATH` to sample your saved settings. Finest chunks have 32³
cells at one-meter spacing. `--chunk-lod 1` selects two-meter spacing, and
`--chunk-point X,Y,Z` selects an integer world position in meters. The report
checks shared face/halo samples and coincident parent samples, and measures live
density payloads. Without a point it samples near the +X surface.

## Octree residency audit

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --design --design-file planet-design.json --check-residency
```

This runs orbit/ground travel, pending-job cancellation, eviction, and revisit
against the saved design. The JSON reports leaf addresses as integer fingerprints,
spacing, job counts, live/peak density payload, and canonical sample agreement.
Defaults allow 512 leaf addresses and two workers, with density allocated within
4 km of the camera. Distant coverage stays as metadata. The density reservation
cap is 84.1 MiB, excluding metadata, worker runtime, and the audit's one reference
volume. Every checked density must match canonical CPU sampling exactly.

This exercises slice 2b's residency owner. The current editor still shows height
previews; the surface audit below exercises slice 2c.
See the [phase document](../../docs/realtime-world-planet-scale.md#slice-2b-camera-driven-residency-implemented).

## Voxel surface and collision audit

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --design --design-file planet-design.json --check-surfaces
```

This builds conforming tetrahedral meshes from the resident chunks, verifies
internal seams, evicts/recreates the location, and checks fine collision. The
report includes geometry counts, intentional outer boundaries, memory payloads,
generation times, and a swept-sphere contact. Mesh potentials retain the full
terrain band stack and unsaturated values; the existing density view still clamps
to +/-4 m. Collision owns a separate 96 m support box with one-meter source cells.

This is a CPU/headless surface baseline. It uses more triangles than the original
QEF mesh and reports mesh storage separately from density residency. The editor
and old `--stream` mode are unchanged; the exploration mode implements slice 3.
See the [phase document](../../docs/realtime-world-planet-scale.md#slice-2c-voxel-meshes-and-nearby-collision-implemented).

## Physical exploration

```sh
cargo run -p procgen-realtime-pilot -- \
  --design --explore --design-file planet-design.json
```

Use **Descend continuously** to travel from orbit or **Go to ground** for a direct
shortcut. **Walk** lands on independent one-meter collision support. W/A/S/D
move, right drag looks around, and E/Q move up/down in flight. Scroll changes
orbit distance or flight speed. The player has a 1.7 m eye height and walks at
4 m/s. The panel distinguishes reference altitude, measured ground clearance,
and the height-field estimate.

A separate 300 km comparison preset preserves the local bands from 65.536 km
through 16 m. Its broadest band is 131.072 km with 4,096 m amplitude; the height
limit remains 12 km. The original `planet-design.json` is unchanged.

```sh
cargo run -p procgen-realtime-pilot -- \
  --design --explore --design-file planet-design-300km.json
```

The viewer loads your saved octave settings and uses GPU exploration by default.
Edit and save in the `--design` editor, then launch exploration again. Neutral,
LOD-color, and normal views are available. Density and mesh generation run on
Metal or Vulkan; Bevy draws the resident GPU buffers directly. Selection and
preparation run on a worker. G5 uses up to 384 GPU height tiles for complete
planet coverage and 125 one-meter voxel chunks around nearby ground. Height
filtering is continuous across tiles; skirts and a 16 m overlap cover the
surface joins. CPU collision remains independent.

Use `--backend cpu` with `--explore` for the canonical CPU visual audit. That mode
retains complete mesh replacement and its 512 KiB per-frame upload limit. There
is no automatic backend fallback.

Record the fixed 90-second native orbit/descent/walk/flight route:

```sh
cargo run -p procgen-realtime-pilot -- \
  --design --explore --design-file planet-design.json --backend gpu \
  --explore-record g5-route.csv
```

This writes frame/stage timings and six adjacent PNG captures, then exits. Live
navigation and movement input are disabled during recorded runs.
The [GPU streaming plan](../../docs/realtime-world-gpu-streaming.md#g5-distant-coverage-filtering-and-final-budgets)
records Metal measurements and remaining limits. The voxel pool is capped at
300 allocations and 512 MiB; each height snapshot is at most 14.31 MiB.
Metal routes peaked at 191 MiB for the large preset and 229 MiB for the small
preset. Frame-time p95 stayed below 9 ms and 14 ms, respectively. Local updates
after initial coverage stayed below 250 ms; initial coverage and height updates
can slightly exceed it. The render overlap is not a watertight mesh export.
Windows/Vulkan validation on RTX 5070 passed all 12 GPU tests and completed both
routes. Peak terrain allocation was 188.9 MiB (large) and 229.3 MiB (small).
Height replacement and frame outliers remain, as do visual overlap and continuous
contact evidence gaps. See the [Windows results](../../docs/windows-gpu-validation.md#windowsvulkan-validation-2026-09-14)
for measurements and their limits.

Run the same closed-coverage, walking, and collision-handoff audit without a GPU:

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --design --design-file planet-design.json --check-explore
```

## GPU generation work

GPU generation and incremental chunk replacement are integrated. See the [GPU streaming plan](../../docs/realtime-world-gpu-streaming.md).
G1 supplies a WGSL density kernel and CPU/GPU agreement checks. G2 adds bounded
uniform-chunk meshing, deterministic scans, and GPU vertex/index/draw buffers.
G3 adds 2:1 transitions and bounded incremental GPU residency. G4 connects these
buffers to the exploration viewer and keeps preparation off the render thread.
G5 adds filtered GPU height tiles, bounded local voxels, and short visual transitions.
It runs on Metal on macOS and Vulkan on Windows/Linux:

```sh
cargo test -p procgen-gpu-tests --test voxel_density_agreement -- --nocapture
cargo test -p procgen-gpu-tests --test voxel_mesh_agreement -- --nocapture
cargo test -p procgen-gpu-tests --test voxel_streaming_agreement -- --nocapture --test-threads=1
cargo test -p procgen-gpu-tests --test height_mesh_agreement -- --nocapture --test-threads=1
```

This checks real GPU chunk batches, shared halo/parent samples, repeated runs,
and CPU agreement using the saved preset and radius/control extremes. Timings
separate pipeline creation, dispatch/completion, CPU sampling, and audit readback.
The test requires a compatible GPU; shader validation alone can run without one
by filtering to `voxel_density_wgsl_validates_without_a_device`.

The meshing audit checks topology, exact shared boundaries, replay order, capacity
failures, and saved-terrain agreement with CPU extraction and collision. It feeds
GPU density directly into extraction without an intermediate readback.

The streaming audit checks mixed-LOD seams, local publication, slot reuse,
cancellation, retirement, overflow, memory limits, and a fixed route through the
saved terrain. Normal generation reads back eight bytes of overflow flags and,
when supported, 32 bytes of stage timestamps per job. Visual samples, vertices,
indices, and draw counts remain on the GPU. The `gpu` feature owns the wgpu
implementation; the inspector enables it, and the audits use the same encoder
as the residency owner. Native-device tests also check deferred submission,
draw leases through slot retirement, and timestamp validity.

## Run the original experiments

```sh
cargo run -p procgen-realtime-pilot -- --seed 42
```

Select a preset, edit its controls, and press **Generate**. Sampling runs on a
worker and uses parallel columns. The inspector keeps showing the previous
volume until the result arrives. A notice identifies unapplied controls.
Move the three slice sliders to inspect the sampled volume without regenerating.

The four panels show base height, an XY density cut, an XZ density cut, and a
ZY density cut. Brown is solid; blue is air. Light pixels are near zero density.
The height panel excludes caves and 3D detail. Colors saturate outside their
display range; the stored values are unchanged.

The diagnostic contains 64 samples per axis, including both ends of a local
box from -1 to +1. These are **local model lengths**, not planet meters. Y is
up only in this Cartesian experiment. The box is a view into the field and
can crop terrain at extreme settings; it is not the spherical voxel
band described below. Surface queries support X/Z and density queries support XYZ within
plus or minus 8 model lengths. Invalid inputs return typed errors.

Three presets cover rounded hills, ridged mountains, and broad basins. Use
seed 42 for the initial comparison. The full seed and resolved parameters are
printed after each interactive generation.

## Headless captures

The CPU library and capture command can run without the engine dependencies:

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --seed 42 --capture /tmp/procgen-pilot
```

This writes one PPM image and one text record per preset. Panel order is height
XZ, density XY, density XZ, density ZY, starting at the top left. Each text
record includes the seed, grid, resolved config, and sampling time. Captures
are local diagnostics; they do not implement terrain-tile export.

## Field decisions

The equations in Murray's slides are the reference. Missing parts are completed
as follows for this experiment:

- Use cubic gradient noise divided by its exported conservative bound. The
  basis and its derivative therefore share the same normalization. Broad relief
  then centers and scales each shape using fixed reference moments; the safety
  bound is not treated as the useful contrast range.
- Use six octaves, frequency `1 / wavelength`, lacunarity 2, and one key across
  octaves. Begin with unit amplitude and damped amplitude; all sums begin at zero.
- Use the unshaped basis derivative in octave coordinates for slope, ridge,
  and perturbation signals. Ridge accumulation mirrors slope accumulation,
  scaled by its own erosion control. These are pilot choices.
- Apply the slope update before adding the damped contribution. Then update
  altitude amplitude and ridge damping, perturbation, and frequency.
- Store the resolved gain once. The displayed `gain + amplification` is one
  value here; the supplied fragments do not establish separate effects.
- Divide the sum by the undamped geometric amplitude envelope, then apply
  `x / sqrt(1 + x*x)` to bound the standardized relief within [-1, 1] without
  hard clipping or normalizing away position-dependent damping.
- Claim analytical derivatives only for the normalized basis and sharpness
  transform. The composed surface and density return scalar values. Their
  feedback vectors are not final gradients.

The surface is the bounded shape function times the height scale. The
[relief calibration report](../../docs/realtime-world-relief.md) describes the
fixed shape moments and the changed terrain. A separate 3D noise
field perturbs the solid boundary. Cave candidates use a 0.5-unit horizontal
grid, a 0.11-unit chamber radius, and a 0.2-unit depth below their own surface
query. Each chamber connects to an inclined capsule that reaches above the
local additive-detail envelope. Only the surrounding nine grid candidates can
affect a column. Cave density is an expected count per square model length.
Chambers can overlap or open through nearby slopes; no global cave network is
promised.

Density magnitude is truncated to 0.125 away from the zero surface. This
preserves the solid boundary and limits cave influence, so omitted distant
candidates cannot change values when a query crosses a candidate-grid boundary.

The seed is folded once by the noise crate. Surface, detail, and caves use
separate named hash domains. A per-column model reuses surface and cave queries
across vertical samples. The direct query path and volume sampler use the same
column evaluator, so cache state and job order cannot change the field.

## Validation and boundaries

```sh
cargo test -p procgen-realtime-pilot --no-default-features
cargo clippy -p procgen-realtime-pilot --all-targets -- -D warnings
cargo fmt -p procgen-realtime-pilot -- --check
```

Tests cover shuffled queries, independent thread schedules, direct/batched
agreement, field envelopes, finite values at control extremes, cave entrances,
agreement with a wider cave search, control effects, analytical gradient checks, invalid inputs, and quantized
initial fingerprints. Exact float bits are not pinned.

Slice 3 adds mixed-detail render meshes and streaming below; slice 4 adds
kinematic contact and population. GPU generation and precision across large
distances remain for later slices. No
planet-scale performance claim follows from these bounded CPU experiments.

## Spherical regions (slice 2)

```sh
cargo run -p procgen-realtime-pilot -- --planet --seed 42
cargo run -p procgen-realtime-pilot --no-default-features -- --planet --check --seed 42
```

The native inspector shows the complete planet. Drag to orbit, scroll to zoom,
and use **Generate** to apply field, sphere, or resolution changes. The previous
mesh remains visible during generation. Switch to **Broad elevation overview**
for the inexpensive height mesh, **Color polygon owners** to inspect the six
face regions, and **Shift render origin** to compare rebased rendering without
changing field queries. The overview and detail mesh use the same broad field;
the overview omits caves and additive density detail.

The initial experiment uses radius 4, height scale 0.5, detail scale 0.04,
rounded-hill noise, and seed 42. Each face has 64 by 64 tangential cells and
16 radial cells. The band extends 0.4 below and 0.3 above broad elevation.
Radial spacing is 0.04375; reference-sphere tangential spacing near a face center
is approximately 0.1. These are model lengths. The default viewport is
1280 by 900 logical pixels. There is one region per face, all resident at the
same resolution. The overview uses 16 by 16 quads per face.

### Field and storage

Broad elevation evaluates the slice-1 shape function at `radius * direction`.
The band radius is `radius + broad_elevation + offset`. Detail evaluates the
3D noise at that planet-centered position. No noise input contains a face ID.
The spherical cave experiment uses candidates from a global 0.5-unit 3D grid:
retain candidates within 0.25 of the reference sphere and project them onto it.
Activation probability is density times 0.5 squared. This is an approximate
surface density, not a promise of a uniform point process.

Cave chambers and vertical shafts follow broad elevation. Their horizontal
metric is reference-sphere chord distance; their vertical metric is band offset.
This is a deliberate spherical adaptation of the local experiment, whose shafts
are inclined. The chamber radius and depth are shared with slice 1. Candidate
projection moves a point by at most 0.25; chamber influence plus density
truncation reaches 0.235. Thus only the surrounding 27 candidate cells can
matter. A wider-search test checks this bound.

Configuration validation requires the lower band to exceed both the additive
detail envelope and the cave depth plus radius, the upper band to exceed the
detail envelope, and the inner radius to remain positive. The upper extent is at most one
reference radius to bound this experiment's query domain. Invalid bands fail;
they are not widened or clipped. The sampled output checks solid lower and
empty upper endpoints as well.

Signed integer cube coordinates identify shared face-grid vertices. All faces
request one canonical sample at a shared edge or corner. Parallel sampling and
cell solves preserve address order. The current sampler assembles the complete
shell; it does not yet generate or evict individual regions. Shared samples
supply the contourer's complete one-cell stencil. Independent region generation
will need the same canonical border cells and polygon-ownership rule.

### Contouring choices and limits

The extractor uses edge roots and gradients from a **trilinear reconstruction
of sampled final density**. Its face-connected cycles choose the interior
connectivity; they do not solve every trilinear interior saddle. Linear
interpolation is an exact, bounded root along each sampled edge. Exactly zero samples are
solid. Normals come from the analytical gradient of this reconstructed final
density in normalized cell coordinates, where the QEF is also fitted. They are
not gradients of the broad height field alone.

The QEF uses 12 fixed Jacobi sweeps on the 3 by 3 normal matrix. It discards
eigenvalues at or below 1e-5 of the largest one and selects minimum displacement
from the intersection mass point in unconstrained directions. If the fitted
point is outside the storage cell, the placement rule explicitly uses the mass
point. It does not clamp coordinates. Zero gradients contribute to the mass
point but add no plane constraint. The accepted point maps back to world space
through the cell's trilinear position map. This constrains storage coordinates;
it does not minimize a world-distance QEF on the curved cell.

Each sign-changing grid edge emits one polygon. Cell-face adjacency orders its
ring; the density sign determines winding. Ordinary rings have four cells;
radial edges at cube corners have three. The lowest incident face address owns
the polygon. A triangle fan splits quads. Region color identifies this owner,
so border polygons can extend slightly into the adjacent face.

Surface-quality slice 2 replaces the original one-vertex-per-cell rule with one
QEF fit per contour cycle. Bilinear face decisions use the same shared samples
on each side; sample IDs resolve an exact saddle tie. Separate face arcs that
connect the same two cell vertices receive distinct shared face vertices.
The hills-42 source now has zero nonmanifold edges or vertices, compared with
467 nonmanifold edges before the repair. Vertex-link checks also detect pinched
sheets that edge counts alone miss. See
[surface quality](../../docs/realtime-world-surface-quality.md#slice-2-extraction-repair)
for scope and measured evidence.

The pure sphere fixture verifies closed two-triangle edge incidence and outward
winding at all seams and corners. Other tests cover planar and sharp QEF data,
near-parallel normals, zero gradients, exact-zero samples, out-of-cell fits,
band rejection, direct/sample agreement, and one-worker/four-worker equality.
The render-origin test reconstructs positions within 0.000002 model lengths
for tested offsets up to 16; it does not claim astronomical f32 precision.

The original POC produced 417,826 density samples, 53,056 contour vertices,
and 106,486 triangles at seed 42. Those counts and its 70–90 ms generation
measurement predate the topology repair. The updated counts and checks are in
the surface-quality report. The overview remains 3,072 triangles; extraction
has no generation kernel or Vulkan agreement claim.

## Streaming and detail (slice 3)

```sh
cargo run -p procgen-realtime-pilot -- --stream --seed 42
cargo run -p procgen-realtime-pilot -- --stream --seed 42 \
  --record /tmp/procgen-stream/seed42.csv
```

Drag to look, use WASD and Q/E to fly, and hold Shift for fast flight. Colors
identify mesh detail: blue is coarse, green is medium, and gold is fine.
The recording command runs a 36-second descent, rapid turns, fast flight,
and retreat, then exits. It writes frame data and a summary. Add `--screenshots` to write
one PNG per phase beside the CSV in a separate visual run; image readback
adds frame stalls. Recording begins after initial coarse GPU coverage is
ready. This flight route bypasses walking; slice 4 adds a separate walking route.

This slice keeps the bounded CPU contour source resident and streams **render
meshes**, not density pages. The source uses the slice-2 64-by-64 face grid and
16 radial cells. This is a deliberate scale limit for the six-region pilot;
it is not a solution for storing an arbitrarily large planet. Density data is
released after extraction. The cheap overview covers the planet during source
preparation and remains until all six initial coarse regions are ready.

Coarse and medium meshes cluster tangential cell vertices in blocks of four
and two respectively. Fine meshes use the refined collision surface described
below. Clustering
keeps radial layers separate, selects the mean position of the source vertices,
and removes triangles collapsed to fewer than three distinct vertices. A cluster
that reverses or flattens a surviving triangle retains its preceding-level
vertices. Medium reduces the base contour; coarse reduces medium, so the coarse mesh cannot
restore triangles already removed by medium. This
check repeats until neighboring changes preserve orientation. Blocks that
contain separate cell parts stay at their preceding level. Complete closed
vertex-link checks reject participating clusters if a collapse creates a
nonmanifold neighborhood.
Every face retains a four-cell fine collar. The collar's sample identities,
positions, and normals are identical at every detail level. Mixed-detail joins
therefore use the same polygons as slice 2. This costs more border geometry
than an adaptive transition mesh, but needs no skirts or overlapping seam
surfaces. Mixed-level audits now require manifold edges and vertex links.

The renderer-independent scheduler admits two region jobs at most. Its queue
has at most one current request per face. Serial tickets reject late results,
including a request that changes from fine to medium and back to fine.
Workers check cancellation before work, during reduction, and during triangle assembly. A cancelled
worker retains its memory reservation until its result is consumed. Fine meshes
are replaced with coarse meshes and released when the camera leaves; every
face retains resident coverage while its replacement is built.

Uploads are split into at most 1,024 triangles per piece. Each frame admits
at most 512 KiB of conservatively counted upload data and stops admitting pieces
after 2 ms of installation work. One piece can cross that time target, so the
recording includes measured installation time. A region becomes ready only
after every piece appears in Bevy's render-world mesh assets. A 0.15-second
complementary ordered-dither transition then replaces the old region. Its
memory reservation remains until the old render assets disappear and the GPU
queue confirms completion of previously submitted work. Further
replacement of that face waits for retirement.

Managed source, job, staging, resident, and retiring-product reservations are
capped at 256 MiB after surface-quality slice 3. Job reservations cover reduction
scratch and the larger fine-region output. Render accounting reserves both CPU
and GPU copies at 156 bytes per expanded triangle per copy. The 512 KiB frame
upload limit stays fixed. Initial source preparation has a separate conservative
1 GiB working reservation and runs once with no concurrent region jobs. These
are application buffer budgets, not a cap on Bevy, the driver, thread stacks,
window buffers, or diagnostic screenshot readback. Process peak RSS is measured
separately in the route report.

The baseline remains radius 4 with about 0.1 tangential sample spacing near
face centers and 0.04375 radial spacing. Manual flight speeds are 0.5 and 4
model lengths per second. The recorded descent moves at 0.7125. The turn phase ends with two seconds of
deliberate 0.12-second revolutions to outrun uploads and test cancellation. Fast flight
circles at radius 5.5 and speed 4. The viewport is 1280 by 900 logical pixels.
Detail uses altitude and viewing direction, with distance hysteresis. Coarse
coverage remains behind the camera. Late detail retains that coarse coverage;
waiting frames and cancelled/rejected work are recorded rather than hidden.

Tests weld mixed-detail face products by their canonical identities and check
for open edges, unbalanced winding, and reversed surviving triangles. They also check readiness of every
upload piece, rejected obsolete results, request reversal, memory admission,
and delayed retirement. GPU visual and timing evidence is recorded separately;
headless scheduler tests do not establish GPU performance.

## Usable terrain (slice 4)

```sh
cargo run -p procgen-realtime-pilot -- --stream --seed 42
cargo run -p procgen-realtime-pilot -- --stream --seed 42 \
  --record /tmp/procgen-usable/walk.csv --walk-route
```

Press **G** to land on the exterior surface below the camera direction, or to
return to flight. Landing rejects steep or obstructed support. WASD walks at
0.18 model lengths/s; drag looks around with radial up. Q/E, scroll, and Shift
remain flight controls. The walker is a sphere of radius 0.035; the camera is
0.12 above its radial foot datum. Rocks are brown and landmarks are red posts.
The sidebar reports the nearest accepted landmark within a chord distance of
3, or no result. Decorations and posts do not have collision.

### Fine geometry refinement

Surface-quality slice 3 adds one conforming edge split inside fine regions.
A shared edge is split only when both incident triangles lie outside the fixed
four-cell collar. Triangles with one or two split edges form the transition to
the unchanged collar; there are no hanging vertices, skirts, or overlapping
patches. Each fully refined triangle becomes four triangles.

New edge vertices search for a density root along the averaged endpoint normals,
within one quarter of the edge length. Ten bisections refine a valid sign
bracket. An absent bracket or zero normal retains the midpoint. Projections
that reverse or flatten a child triangle are rejected on all incident triangles.
The result retains the contour's connectivity; it does not discover new sheets
below the original sampling resolution.

The canonical refined CPU mesh is prepared once across the bounded planet for
stable collision and placement. Fine render products copy its triangles only
on nearby faces. Coarse and medium use the original contour reductions. This
retains more CPU geometry rather than switching collision underneath the walker;
it is not demand-paged high-resolution density. See the
[slice 3 record](../../docs/realtime-world-surface-quality.md#slice-3-fine-geometry)
for counts, validation, and memory costs.

### Collision and movement

Collision uses the repaired and refined fine source triangles. Render reduction, upload,
dither, and retirement cannot change the collision surface. The query contract
remains a **two-sided triangle query surface**. The topology repair adds no
inside/outside classification or solid-volume physics behavior. It supports a kinematic sphere, not a
capsule, jumping, stepping, rigid bodies, or guaranteed camera-head clearance
inside caves. Walking requests fine detail on nearby faces regardless of view direction.
Before those uploads are ready, coarse render surfaces can differ from the
fine contact surface; collision remains ready and stable.

A bounding-volume hierarchy indexes canonical source triangles. A nearby patch
holds at most 32,768 triangle IDs in a cube with half-width 0.6. Patches rebuild
before the walker leaves an inner margin. Sphere sweeps require the complete
start/end swept bounds to fit the patch; missing coverage or an oversized patch
stops movement and increments a visible failure count. Triangle distances drive
conservative advancement; density magnitude is never used as a distance bound.
Nearest-distance queries traverse the existing BVH near-first and prune nodes
that cannot improve the result, while retaining the patch's exact candidate IDs.
This avoids scanning the whole refined patch for every sweep iteration.
Each sweep has at most 64 iterations; exhaustion rejects the sweep, retains
its safe start, and reports a failure. A substep has at most four slide contacts.

The contact skin is 0.0001 model lengths. A 0.005 downward support query keeps
contact at triangle joins. A support normal must have dot product at least 0.65
with radial up; walkable support has static friction. Gravity is 1.5 model
lengths/s². Simulation substeps are at most 1/120 second, with at most 0.1 second
simulated per rendered frame. Longer frames discard excess simulation time and
increment the delayed-frame count; they are not claimed as uninterrupted travel.

### Population and readiness

Rocks use an 8-by-8 candidate grid per face; landmarks use 2-by-2. Candidate
identities are kind, face, x, and y. Independent seed streams jitter candidates
within the middle half of each cell. Noise selects clustered rock candidates.
Fine-triangle exterior queries supply position and slope; both kinds reject
support below the walkable slope threshold. There is no water or material field
in this pilot, so those eligibility rules are not claimed.

The fixed six-face experiment has at most 408 candidates. Its accepted catalog
stays on the CPU, while render instances enter and leave a radius of 1.8 around
the camera. At most eight new instances install in a frame; all use two shared
meshes and materials. Seed 42 accepts 199 rocks and 24 landmarks. The local
instances and bounded landmark search read the same accepted catalog. Revisit
and eviction change neither identities nor contact positions. This is bounded
population residency, not world-scale placement paging.

Preparation sends the fine source to the render scheduler first. Collision
indexing and population resolution then run on the preparation worker; coarse
render installation can proceed during that work. Walking waits for collision
readiness. A 32 MiB reservation covers the larger collision index, nearby IDs,
placement data, and population buffers within the 256 MiB managed cap.
Source preparation retains its separate 1 GiB working reservation until contour
extraction and refinement end. Engine and driver allocations
remain outside these application buffer reservations.

The walking recording starts at the first accepted candidate with clear
walkable support. It walks toward projected world X for 12 seconds, turns in
place for six seconds, walks toward projected -X for 12 seconds, then rests for
six seconds. This is a repeatable input route, not a promise to retrace an exact
path around obstacles. CSV output adds walking/population time, nearest triangle
clearance, grounded state, delayed frames, and collision failures. Add
`--screenshots` for a separate image run. The original flight recording remains
available without `--walk-route` and now includes population work.

Tests cover two-sided fast sweeps, triangle face/edge/vertex distances, missing
coverage, resting contact through all render levels, walking clearance, seeded
placement recreation and eviction, accepted integer IDs, and bounded landmark
searches. Native measurements and platform limits are in
[the slice 4 report](../../docs/realtime-world-usable-results.md).

## Variety, captures, and seed checks (slice 5)

```sh
cargo run -p procgen-realtime-pilot -- --stream --preset ridges --seed 42
cargo run -p procgen-realtime-pilot -- --stream --preset basins --seed 42 \
  --record /tmp/procgen-variety/basins-walk.csv --walk-route --screenshots
cargo run -p procgen-realtime-pilot -- --stream \
  --replay /tmp/procgen-variety/basins-walk.replay.json \
  --record /tmp/procgen-variety/basins-replay.csv --screenshots
```

Spherical presets are `hills`, `ridges`, and `basins`. Hills preserves the
slice-4 defaults. Ridges and basins adapt the corresponding local noise controls
to this radius-4 shell: height scales 0.7/0.5, detail scales 0.055/0.025, and
cave densities 2.5/1.5. The grid and radial band remain unchanged. These are
pilot choices, not claimed source-game parameters. The sidebar identifies the
preset and expands **Resolved parameters** to show every input and the build ID.

Each recording now saves its resolved `.case.json` before generation, a frame
CSV, a `.summary.txt`, and a `.replay.json`. Summaries identify the source build,
Rust toolchain, OS, and any replay's originating build. A build ID hashes the
pilot source, core/noise/cube-sphere sources, pilot manifest, and workspace lock
file. It identifies source inputs, not a bit-identical executable across targets.

A case has format version 1, source build, and a scenario containing the full
`u64` seed, resolved planet configuration, and `flight` or `walk` route. Use
`--stream --case FILE` to inspect one, or add `--record CSV` to run its route.
Edit a copy of the case to inspect custom parameters. Unknown JSON fields,
invalid terrain parameters, and radii other than 4 fail explicitly. Case/replay
inputs cannot be combined with seed, preset, or route overrides. The local and
static planet inspectors keep their existing controls and defaults.

Replay uses the actual recorded camera position and forward vector preceding
each playback time; it does not invent intermediate rotations during rapid
turns. It preserves the recorded route's walking detail demand and radial up.
This is **camera replay**, not physics, job-order, upload-timing, or frame-time
reproduction. Replays can compare source builds; the originating and current
builds remain visible. A replay starts at zero, has strictly increasing times,
and contains 2–16,384 finite poses within the 36-second route. Input artifacts
are limited to 8 MiB. A recording that reaches its pose cap fails explicitly.

### Automated seed sweep

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --sweep /tmp/procgen-variety/sweep --samples 2 --sample-seed 20260913
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --sweep /tmp/procgen-variety/rerun --case /tmp/procgen-variety/sweep/hills-42.case.json
```

Each preset uses fixed seeds 0, 42, and 4,294,967,338, plus up to 32 sampled
seeds. The default is two sampled seeds per preset. Omit `--sample-seed` to take
a fresh host-time seed; the suite records that seed and every resolved world
seed before evaluation. Supply it again to reproduce the same sample. An output
directory must be empty so a later run cannot overwrite a prior report.

Cases run sequentially. Each checks parameter validity, fine and three mixed
mesh arrangements, population recreation, and a 36-second walking route at
1/120-second simulation steps. Native and headless walking use the same landing
selection and movement-input function. The CPU audit checks contact clearance
and stationary support; it records quantized source/route fingerprints, shape
ranges, triangle/placement counts, source/index bytes, and phase timings. It
retains only six mixed-detail products at once. It does not emulate GPU jobs
or prove rendered frame budgets.

A case is **failed** on invalid geometry, nonmanifold edges or vertex links,
or contact. Fine and mixed-level products must pass. A valid case is **expensive**
when source/collision/population preparation exceeds 1,000 ms or a measured
walking step exceeds 4 ms. These are CPU triage thresholds, not GPU frame-budget
claims; scheduling noise can produce an outlier. Mesh-audit and whole-route
costs are also recorded. Re-measure expensive cases before attributing a cause.

The suite writes a case JSON for every seed, flushes one result per case to
`results.jsonl`, and produces a summary. Contact failures also save a static
camera replay at the failure location, with stage and simulation tick in the
result. That view helps inspection; it does not replay the failed physics.
Failures return a nonzero exit status after the remaining cases finish. Expensive
cases remain valid and are reported separately. Parameter failures have no
surface location to capture. Use the saved case with the native inspector for
flight/ground images; the CPU sweep itself does not produce GPU screenshots.
Each successful result also records `broad_height` (minimum, 5th percentile,
median, 95th percentile, maximum) over 65,536 area-distributed directions,
before caves and local detail. This distinguishes broad relief from mesh extrema.

Results and remaining pilot limits are in
[the variety report](../../docs/realtime-world-variety-results.md).

## Neutral surface inspection

The streaming inspector now starts in neutral gray with fixed lighting and
restrained procedural normal and roughness detail. Select **Plain** under
**Surface detail** to compare with the untextured material. Select
averaged mesh, triangle-face, or final-density normals without regenerating the
world. LOD colors remain available, alongside normal-direction and
mesh/density-agreement overlays. **Triangle edges** can be combined with any
view. Magenta identifies an undefined normal.

```sh
cargo run -p procgen-realtime-pilot -- --stream --preset ridges --seed 42 \
  --normals triangle --wireframe
```

Use `--normals averaged|triangle|density` and
`--surface neutral|lod|normals|agreement`, plus
`--surface-detail plain|textured` (default `textured`), with saved camera replays
for matched comparisons. Surface detail applies only to neutral shading;
diagnostic overlays retain the base surface. The pattern uses planet coordinates
at fixed scales and fades below pixel size. It adds no geometry or collision.
Recordings save a separate `.view.json`; generation cases and
camera paths keep their existing formats. See
[surface quality](../../docs/realtime-world-surface-quality.md) for the normal
contract, memory cost, comparison procedure, and findings.
