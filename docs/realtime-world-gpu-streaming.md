# Real-time pilot: GPU generation and incremental streaming

Status: G1 through G5 are implemented. Physical exploration uses GPU height tiles
and local voxel geometry by default; `--backend cpu` selects the CPU visual audit.
Metal validation and route measurements are recorded below. Windows/Vulkan
results are in [Windows GPU validation](windows-gpu-validation.md#windowsvulkan-validation-2026-09-14).
Both hosts have completed the original G5 routes; visual and measurement gaps
remain. The [surface-join follow-up](#surface-join-follow-up) replaces pixel
discard transitions with smooth composition and has separate validation.
This replaces the CPU-only visual generation plan for the physical-planet pilot.
The original saved terrain settings remain unchanged. G5 adds a separate 300 km
comparison preset. The earlier world pipeline is unrelated.

## Required architecture

GPU density generation and meshing through wgpu are required on macOS/Metal and
Windows/Vulkan. Visual density, extraction scratch, vertices, indices, and draw
counts stay in bounded GPU buffers. Normal exploration does not read visual
geometry back to the CPU. There is no CUDA requirement and no automatic CPU
fallback. The CPU path remains an explicitly selected reference and audit mode;
independent nearby CPU collision continues to support walking.

The octree and scheduling policy remain on the CPU. Stable chunk addresses and
generation tickets identify GPU slots. Reuse unchanged chunks. Rebuild only
changed chunks and the boundary geometry affected by their neighbors. Retain
old coverage until a complete local replacement group is ready. Retired slots
cannot be reused until GPU completion; obsolete jobs cannot publish into reused
slots. Never wait for the entire selected planet to regenerate during movement.

The current baseline rebuilds 5,510,386 triangles in about 37 seconds on M1 Max,
before CPU packing and upload. It holds 83.7 MiB of source density and 320 MiB of
CPU mesh vector capacity, excluding scratch and renderer copies. Faster kernels
and smaller updates are both necessary. CPU sampling is already multithreaded;
this work does not assume a speedup without measurement.

## G1: GPU density and CPU agreement

Implement the full physical octave stack and compensated radial potential in
WGSL. Reuse the existing core hash and gradient-noise shader. Upload validated
field parameters and integer chunk origins; reuse the canonical narrowed seed.
Output the same 35-cubed halo grid as CPU chunks, including unsaturated potentials
for coarse edge interpolation. Keep dispatch size and buffers bounded.

Acceptance: execute real compute dispatches on Metal, provide the same command
for Vulkan, and measure CPU agreement near the surface and at coarse levels.
Check shared faces/edges/corners and coincident parent samples within one backend,
repeated runs, reordered batches, disabled bands, both seed halves, supported
radius extremes, and the actual saved preset. Audit readback is permitted here.
Report compilation and transfer overhead separately from steady dispatch work.
This slice does not claim an exploration speedup or modify visual meshing.

### G1 implementation

The pilot owns `voxel_gpu.rs` and `voxel_density.wgsl`. It exports a validated
field packing layout, integer chunk records, and the assembled compute shader,
with no wgpu or Bevy dependency in those modules. A 64-thread workgroup writes
chunk-major unsaturated f32 potentials into storage. No CPU noise evaluations or
per-sample coordinate uploads occur in the GPU dispatch. The existing GPU test
crate owns device creation and audit readback. G1 kept the viewer on its CPU
reference; G4 connects these kernels to exploration.

The initial direct translation differed from CPU by up to 1.52 m on steep saved
terrain. Shader reassociation changed compensated radial residuals and noise
coordinates. Core WGSL arithmetic helpers now accept fixed runtime identities
(one and zero), so explicit FMA calls retain CPU add/multiply boundaries without
adding quantization. The shared noise shader uses the same implementation with
these identities; its ordinary entry point passes literals and retains normal
optimization. The pilot supplies the identities in its private uniform layout.

The radial square root uses the same fixed polynomial iteration and residual
correction in Rust and WGSL before selecting noise lattice cells. This settles
the numerical sampling contract without changing octave settings. The shader
also corrects reciprocal/division residuals. No f64, 64-bit integer arithmetic,
GPU-specific source fork, or CPU fallback is required.

Run on macOS/Metal or Windows/Vulkan:

```sh
cargo test -p procgen-gpu-tests --test voxel_density_agreement -- --nocapture
```

The GPU test selects Metal on macOS and Vulkan elsewhere and requires a device.
For parsing/validation without a device, filter to
`voxel_density_wgsl_validates_without_a_device`.

CPU agreement uses an absolute 0.02 m potential tolerance plus four f32 epsilon
units relative to the unsaturated potential. The absolute part is one fiftieth
of a finest voxel; the relative part accounts for coarse samples millions of
meters from the surface. Exact equality is required for shared GPU samples,
repeated dispatches, and changed batch order. The suite includes the checked-in
saved preset, fractional radius, 100 km and 8,000 km planets, both seed halves,
24 bands, disabled and zero-amplitude bands, strong feedback, and a flat sphere
including its center. Timing output separates pipeline creation, dispatch and
completion, CPU sampling, and audit readback. It does not measure GPU timestamp
intervals or end-to-end rendering latency.

Metal validation on Apple M1 Max covers 1,071,875 CPU/GPU sample comparisons.
Maximum error within 32 m of the surface is 0.00245 m; far, unsaturated samples
differ by up to 1 m at planet-scale magnitudes and satisfy the relative bound.
All 78,174 coincident GPU halo/parent comparisons are exact, as are repeat and
batch-order comparisons. The 11-chunk saved-preset batch writes 1,886,500 bytes
of potentials. Warm dispatch plus completion takes about 13 ms; CPU sampling
measured 217–318 ms across local runs. Audit readback adds about 2–3.4 ms.
Shader/pipeline creation measured 134 ms initially and about 2 ms with the driver
cache warm. These are local observations with other work running, not guaranteed
latency bounds. See the [Windows/Vulkan results](windows-gpu-validation.md#windowsvulkan-validation-2026-09-14).

Validation also passes the existing pilot suites with and without inspector
features, an independent f64 oracle test for the fixed square root, shared
noise/terrain GPU agreement tests, Clippy for both pilot feature configurations
and the GPU test crate, the native build, and formatting. The saved-preset CPU
exploration audit retains 5,510,386 ground triangles, a 0.079 mm collision handoff
difference, and zero idle drift. The saved design file has no changes in G1.

## G2: GPU chunk meshing

Implement bounded GPU classification, deterministic output allocation, and mesh
emission for uniform chunks. Keep output in storage/vertex/index buffers. Use
integer anchors and chunk-local positions. Maintain a canonical CPU extraction
reference for the chosen topology; do not port the global tree-based extractor
by sending all of its scratch data to the GPU.

Acceptance: planes, spheres, exact-zero samples, steep saved terrain, empty/full
chunks, shared uniform boundaries, output capacity failures, and repeated work
must produce valid meshes. Check surface/CPU collision agreement with a stated
geometric tolerance. Counts must come from deterministic scans rather than
unordered append allocation. Overflow retains prior coverage.

### G2 implementation

The pilot now exports `build_voxel_chunk_mesh`, a parallel CPU reference, and
`voxel_mesh_shader`, a four-pass WGSL implementation. A fixed six-tetrahedron
(Freudenthal) split uses the same diagonal on shared uniform faces. Rust owns the
tetrahedra and oriented sign-case rings; shader assembly emits those tables.
The existing adaptive 24-tetrahedron extractor and collision path are unchanged.

Vertices are shared through seven monotone edge slots and one exact-zero node
slot per grid node. Zero endpoints resolve to the node slot; repeated roots are
removed before triangulation. Isolated zero points or edges produce no triangles.
An all-zero chunk is full and emits no mesh. No density epsilon changes the field.
Vertices store an integer meter anchor relative to the chunk and a separate float
offset. Interpolation starts at the endpoint nearest zero, so large world origins
do not swallow small offsets. Vertices and indices have storage plus vertex/index
buffer usage; indexed indirect draw arguments stay in a GPU buffer.

The passes classify counts, scan 256-entry blocks, scan block totals, and emit
vertices and indices at deterministic offsets. There are no append atomics.
One bind group owns one chunk and uses eight storage bindings, with no optional
GPU features. Source potentials come directly from G1's output buffer. Neither
samples nor geometry return to the CPU between density and mesh dispatches.
Device creation, dispatch, and audit readback live in `procgen-gpu-tests`; the
pilot's packing and extraction modules have no wgpu or Bevy dependency.

`VoxelMeshConfig` sets positive vertex and triangle capacities. The fixed upper
bounds are 287,496 vertex candidates and 393,216 triangles per chunk. Scan scratch
uses 2,317,952 bytes per chunk, including block totals and offsets. The audit's
50,000-vertex/100,000-triangle slots use about 5.04 MiB each for density, scratch,
mesh buffers and small records; worst-case slots use about 15.65 MiB. These exclude
shared field parameters, pipeline objects, driver allocation, and audit readback.
G3 must budget the number of resident and in-flight slots against these costs.

Overflow records required counts, writes a zero indirect index count, and skips
all vertex/index writes. Tests fill destination buffers with sentinels and prove
that neither vertex nor index exhaustion changes them, including capacities one
short of the required count. Exact-fit capacities succeed. The consumer must use
an unpublished destination and retain the previous resident slot on failure;
publication and replacement ownership arrive in G3.

Run the meshing audit on Metal or Vulkan:

```sh
cargo test -p procgen-gpu-tests --test voxel_mesh_agreement -- --nocapture
```

Filter to `voxel_mesh_wgsl_validates_without_a_device` for Naga validation only.
The device audit requires Metal on macOS and Vulkan elsewhere. It checks empty,
full, all-zero, axis/oblique planes, exact-zero contacts, closed outward-facing
spheres, large origins, coarse spacing, the final partial scan block, worst-case
checkerboard terrain, overflow, and the saved design. Eight neighboring chunks
must agree exactly on shared face/edge/corner geometry. Repeated work, reversed
chunk order, interleaved passes, and sequential passes must reproduce exact GPU
vertices and indices. CPU extraction is also checked across Rayon schedules.

With identical input potentials, CPU/GPU indices and counts are exact; vertex
positions allow 0.00001 times sample spacing for float interpolation. With GPU
density feeding GPU extraction, 256 surface probes in the saved +X landing chunk
show a maximum 0.000900 m difference from the same mesh built with CPU density,
within G1's 0.02 m surface bound. The current CPU collision mesh differs by up to
0.122281 m, within this slice's quarter-voxel (0.25 m) geometric acceptance bound.
The collision comparison uses an independent f64 axis-ray probe of both meshes:
the existing f32 collision ray query can miss an exact triangle edge. This is a
geometry comparison, not a test of that query's edge behavior. The measured
interpolation difference applies to this patch, not every possible field. G4
must check visible ground contact when connecting the new topology to walking.

On Apple M1 Max/Metal, the saved chunk produces 4,889 vertices and 9,496 triangles.
The checkerboard reaches the exact 393,216-triangle bound, with 137,312 vertices.
Timing measures queue submission through completion, excluding pipeline creation,
buffer allocation, and readback. Local single-chunk mesh runs generally take
about 2–6 ms once warm; cold submissions can take longer. The CPU reference takes
about 2 ms for simple chunks and 12 ms for the checkerboard, so this slice does
not claim that isolated small mesh jobs always run faster on the GPU. The GPU
path avoids future CPU geometry transfer and composes directly with GPU density.
A warm saved-chunk density-plus-mesh dispatch measured 2.66 ms, versus 22.92 ms
for CPU sampling plus the new CPU extractor; mesh audit readback added 5.77 ms.
A warm 13-chunk fixture batch, including the checkerboard, took 9.94 ms with each
chunk's passes adjacent and 28.75 ms with passes interleaved across chunks. Both
schedules returned identical output. These are local observations, not budgets.
Pipeline creation measured 264 ms cold and about 4 ms with the driver cache warm.
See the [Windows/Vulkan results](windows-gpu-validation.md#windowsvulkan-validation-2026-09-14). These checks do not measure
viewer frame rate: mixed LOD, persistent residency, and rendering remain G3/G4.

Validation passes both Metal density/meshing suites, 76 pilot tests without the
inspector and 77 with it, Clippy for both pilot feature configurations and the GPU
test crate, the native build, formatting, and diff checks. `planet-design.json`
has no changes.

## G3: mixed LOD and incremental residency

Settle a bounded transition contract before integrating it: evaluate a 2:1
balanced neighborhood and explicit transition meshes against the current
arbitrary-ratio conforming CPU baseline. Preserve the meter-scale nearest source
and canonical shared density. Define which neighbor changes invalidate a mesh.

Add persistent GPU chunk slots, memory reservations, a camera-priority queue,
generation tickets, cancellation, eviction, and local replacement groups.
Acceptance: small camera moves reuse unchanged outputs; coarse/fine faces,
edges, and corners remain closed during replacement; eviction/revisit reproduces
results; stale jobs and slot reuse cannot corrupt visible coverage. Measure
changed chunks and time-to-first-local-detail on a fixed travel route.

### G3 implementation

`VoxelCoverage` enforces 2:1 spacing at faces, edges, and corners. Balancing splits
coarse neighbors and preserves requested fine leaves; it returns an error if the
leaf budget is insufficient. A sparse octree index serves neighbor queries.
`VoxelMeshKey` contains the chunk address and its finer face/edge neighbors.
Point-only contacts introduce no new samples or subdivisions and do not invalidate
a mesh. Equal/coarser neighbors also do not change that chunk's extraction.

`VoxelTransitionPlan` compiles topology without reading density. Only affected
coarse boundary cells replace G2's six tetrahedra. Each replacement cell connects
its center to a shared triangulation of its six faces. Fine-facing squares split
into four squares with the same diagonals as the fine mesh. Split edge knots also
subdivide adjacent faces, which closes edge and corner junctions. Unaffected cells
keep G2's topology. At most 5,768 cells and 276,864 tetrahedra belong to a chunk's
transition plan. Node stencils select canonical source points at the finest
incident resolution; derived interior nodes average coarse source corners.

G1's new `sample_points` entry point evaluates these integer source points through
the same potential function as the chunk grid. It does not read neighboring CPU
volumes or narrow the seed again. G3 then classifies transition tetrahedra, scans
counts, and emits indexed triangle soup with canonical shared root positions.
Regular and transition outputs have separate draw buffers but publish together.
The GPU never reads CPU-generated visual vertices or indices. The CPU reference
uses the same transition topology and interpolation, and remains available for
audits. The existing adaptive extractor and walking collision are unchanged.

The optional pilot `gpu` feature owns `VoxelGpuMesher` and `VoxelGpuWorld` through
wgpu. G2's audit now uses this production encoder rather than maintaining a second
copy. Device selection is explicit; Metal and Vulkan use the same shaders and
require eight storage bindings, without optional GPU features. G3 exposed this
path for audits; G4 enables it in the viewer.

`VoxelGpuResidency` owns slot generations, a camera-priority queue, and connected
local replacement groups. Changed chunks and incident transition changes form a
group. Unchanged keys retain their GPU slots. The scheduler finishes one group at
a time so partial groups cannot fill the pool and prevent publication. Camera
priority changes do not interrupt that group. If selection splits pending work
into several groups, it retains one and discards the other unpublished work.
It retains old coverage until every new member is ready. A failed group releases unpublished
fragments and permits independent groups to progress; it requires an explicit retry.

Cancelled jobs keep their slot until completion, including a leave/revisit before
the old job acknowledges completion. Generation tickets reject duplicate and stale
receipts. Retirement also waits for submissions that used the old draw buffers.
Borrowed draw buffers require `track_draw_submission` after submission, serialized
with publication. G4 instead holds owned draw leases through GPU completion.
GPU completion polling does not wait: each job maps two overflow flags, eight
bytes total, plus optional G4 timestamps. Visual samples, geometry, and draw
counts remain on the GPU.
Device or mapping failure stops that stream explicitly, with no CPU fallback.

Output slots stay allocated for reuse. Source grids, transition inputs, and scan
scratch live only while their job is in flight. `VoxelGpuMeshConfig::slot_bytes`
and `VoxelGpuMesher::work_bytes` provide the output cost and a conservative work
reservation. The owner checks the memory budget before allocating a destination
or submitting work. Both geometry overflow and budget failure retain old coverage.
Slot headroom includes the larger of current/final residency plus the largest
replacement group; initial empty residency needs only its requested slots.

Run on macOS/Metal or Windows/Vulkan:

```sh
cargo test -p procgen-gpu-tests --test voxel_streaming_agreement -- --nocapture --test-threads=1
```

For Naga validation only, filter to
`voxel_transition_wgsl_validates_without_a_device`. CPU tests cover all eight
refinement orientations, exact-zero planes, neighbor-index agreement with exhaustive
search, atomic local publication, delayed retirement, cancellation/ABA, insufficient
headroom, and independent progress after a group fails. GPU tests cover mixed
faces/edges/corners across three LOD levels, exact zeros, a fine feature missed by all coarse corners,
capacity sentinels, real overflow and budget failures, old coverage while receipts
are pending, unchanged slots, eviction/revisit, and exact GPU replay.

The 15-chunk sphere fixture is closed with both extractors: G3 produces 105,024
triangles versus 228,330 for the adaptive baseline. Maximum radial error is
0.029335 m versus 0.020615 m, respectively, under a 0.125 m fixture bound (one
sixteenth of its 2 m coarse voxel). Identical input potentials require exact
CPU/GPU indices and counts; position tolerance remains 0.00001 times spacing.
Shared GPU boundary positions and revisited outputs must be exact.

### G3 saved-terrain route on Metal

The GPU route covers a 256 m cube around the saved +X landing area, with 71 leaves:
2 m coarse voxels and a 64 m region of 1 m voxels. It moves from
`(4903255, 16, 16)` to `(4903255, 32, 16)`, then `(4903255, 80, 16)`, and returns.
The slot pool allows 128 slots and two jobs in flight, with a 512 MiB GPU budget.
Each slot reserves 30,000 regular vertices, 60,000 regular triangles, and 16,384
transition triangles. The cube boundary is intentional; this audit does not claim
to render the whole planet.

| Stop | Rebuilt chunks | First useful publication | Peak GPU allocation |
| --- | ---: | ---: | ---: |
| Cold residency | 71 | about 199 ms | 238 MiB |
| 16 m move | 0 | existing output reused | 234 MiB |
| 64 m move | 23 | about 129 ms | 314 MiB |
| Revisit | 23 | about 106 ms | 316 MiB |

These local Apple M1 Max measurements start after coverage selection, exclude
pipeline construction and geometry audit readback, and poll completion at 1 ms
intervals. They are residency timings, not viewer frame times or GPU timestamps.
The movement retains the other 48 slots and reproduces the original output on
revisit. Preparation includes CPU transition planning, buffer creation, and input
upload; it took about 84 ms in total across the 23 changed chunks. Command encoding
took about 5 ms total. The saved settings remain unchanged.

Planet-wide metadata selection was measured separately: the current 512-leaf
request expands to 2,304 balanced leaves while retaining meter-scale detail. The
sparse index reduced selection/balancing from about 244 ms to 46 ms. G4 must budget
that larger coverage and move selection and topology preparation off the render
thread; these timings do not meet its 2 ms CPU frame target yet. GPU generation
itself is asynchronous, but job preparation currently runs during admission.
See the [Windows/Vulkan results](windows-gpu-validation.md#windowsvulkan-validation-2026-09-14).

Validation passes the pilot suites with and without the inspector, the focused
residency regression, all three Metal density/meshing/streaming suites, Clippy
with warnings denied for the pilot and GPU tests, formatting, and the native
pilot build.

## G4: GPU exploration integration

Move coverage selection and transition-plan preparation off the render thread.
Connect resident GPU geometry directly to Bevy rendering and GPU draw counts.
Make GPU and CPU audit modes explicit. Stop whole-planet CPU mesh generation,
packing, and upload during normal GPU exploration. Keep nearby collision on its
independent CPU schedule and prioritize support ahead of walking.

Acceptance: continuous descent, flight, and walking without global replacement
waits. Report CPU scheduling time, GPU density/extraction time, queue latency,
first useful detail, resident/retiring bytes, and frame times. Diagnose these
separately. Initial targets are a 2 ms CPU scheduling budget per frame and useful
nearby refinement within 250 ms once coverage is resident; these are targets to
measure on both hosts, not claims about current performance.

### G4 implementation

The inspector enables the pilot's `gpu` feature. `--explore` selects GPU
exploration by default; `--backend cpu` keeps the existing CPU visual audit.
GPU mode starts no CPU visual build or overview upload. Collision keeps its
separate CPU worker and one-meter canonical field.

The generation worker selects the target partition, advances through closed
intermediate coverage, prepares transition plans and inputs, and encodes GPU
commands. It prioritizes nearby splits before distant coarsening. Each step
preserves 2:1 balance; coarsening checks sibling and neighbor metadata before
rebuilding an index. A CPU orbit/ground/move/revisit test reaches each requested
partition without repeating a partition. The final metadata route takes about
1.6 seconds in total on the M1 Max; this work is off the render thread.

`VoxelGpuSubmission` transfers encoded commands to the render queue owner. The
renderer submits them and arms asynchronous completion, while the worker keeps
their inputs alive. Latest camera input and immutable draw snapshots cross a
small shared mailbox. `VoxelGpuLease` pins an output allocation through snapshot
replacement and through GPU draw completion; a completed retirement cannot reuse
a leased slot. Tests retain old draw leases through real replacement and delayed
submission before permitting reuse.

The render node draws regular and transition buffers with indexed-indirect
commands. Sixteen-byte instance records carry integer chunk origins and LOD;
integer camera subtraction precedes float conversion in the vertex shader.
Frustum checks reject whole chunk bounds before encoding draws. Neutral lighting
and normal inspection use triangle derivatives. View changes update only camera
and display metadata. Visual potentials, vertices, indices, and draw counts are
never read back for normal exploration.

The viewer allows 4,096 output slots, two pending jobs, and an 8 GiB allocation
budget. A slot reserves 20,000 regular vertices, 40,000 regular triangles, and
8,192 transition triangles (about 1.91 MiB). Its retained pool is reusable but
does not shrink after travel. Overflow and budget errors stop the stream with
old coverage retained; they do not select a different backend. These remain
large reservations until G5 replaces distant voxel coverage.

The panel and CSV separate worker selection/preparation, command encoding,
submission-to-receipt latency, local publication latency, render scheduling,
draw encoding, GPU stage times, and frame times. Submission-to-receipt includes
queueing, GPU execution, and callback polling. Job completion also includes
preparation. Displayed stage counters are the latest observation of each stage,
not necessarily the same job. Resident, retiring, and total allocation bytes are
reported separately. Timestamp-capable devices read 32 extra bytes per job;
unsupported devices report timing as unavailable. Timestamps use compute-pass
boundaries: encoder timestamps produced zero intervals and stale outputs on the
tested Metal path. The native-device regression requires valid stage intervals.

Run the native route (writes a CSV and six PNG captures, then exits):

```sh
cargo run -p procgen-realtime-pilot -- \
  --design --explore --design-file planet-design.json --backend gpu \
  --explore-record g4-route.csv
```

### G4 native route on Metal

The saved 4,900 km design was exercised on the Apple M1 Max in a 2,880 by 2,000
window, with orbit, continuous descent, a ground hold, walking, flight, and a
return to orbit. The route includes screenshot capture costs. The saved octave
settings and CPU collision implementation are unchanged.

| Segment | Frame p50 | Frame p95 | Render scheduling p95 | Draw encoding p95 |
| --- | ---: | ---: | ---: | ---: |
| Orbit | 8.4 ms | 17.1 ms | 0.24 ms | 0.04 ms |
| Descent | 16.6 ms | 17.4 ms | 0.34 ms | 0.10 ms |
| Ground hold | 8.8 ms | 17.5 ms | 0.36 ms | 0.13 ms |
| Walk | 16.7 ms | 17.5 ms | 0.36 ms | 0.28 ms |
| Flight | 16.7 ms | 17.7 ms | 0.43 ms | 0.34 ms |
| Return to orbit | 16.7 ms | 18.6 ms | 0.30 ms | 0.82 ms |

Meter-scale geometry first became resident at 27.9 seconds; descent began at
5 seconds. Walking used canonical collision, and the full route completed
without an overflow or stream failure. Peak GPU allocation was about 5.3 GiB.
The largest observed frame in the final route was 142 ms; an earlier complete
route reached 307 ms. These are native viewer measurements,
not isolated compute throughput or Vulkan measurements.

The typical CPU render scheduling cost meets the 2 ms target. Local publication
can still exceed 250 ms, full target coverage can lag travel, and the retained
voxel pool remains large. G5 must reduce distant generation and memory, filter
coarse geometry, and address replacement latency and outlier frames. One-meter
samples in the nearest leaves do not imply that all visible terrain or the entire
collision box has reached that resolution.

Validation includes the pilot suites with and without the inspector, G1/G2/G3
Metal agreement suites, progressive coverage, direct-render shader validation,
native device feature/limit tests, queued submission, leased retirement, Clippy,
and a native viewer build. See the [Windows/Vulkan results](windows-gpu-validation.md#windowsvulkan-validation-2026-09-14).

## G5: distant coverage, filtering, and final budgets

Retain a cheap distant height representation and connect it to local voxel
coverage without exposed seams. Filter geometry to its resolution while keeping
nearby collision canonical. Reduce replacement pops. Profile the integrated
pipeline before adjusting workgroup sizes, parallelism, or memory budgets.

Acceptance: measured orbit-to-ground routes on Metal and Vulkan, bounded memory
through repeated travel, explicit failure handling, deterministic revisits, and
stable collision contact. Record p50/p95 frame and update latency and the hardware
used. A fast isolated density dispatch does not complete this phase.


### G5 implementation

`planet-design-300km.json` uses seed 42 and a 300,000 m radius. It keeps the saved
bands from 65,536 m through 16 m unchanged. The new broadest band uses the
original continental shape controls at 131,072 m wavelength and 4,096 m
amplitude. The height limit remains 12,000 m. This gives 14 bands, retains local
relief, and changes continental scale separately. The 4,900 km preset is intact.

Distant coverage is now a complete six-face cube-sphere height surface, with at
most 384 tiles and 32 by 32 quads per tile. Refinement extends to eight tile
widths from the surface projection, retaining useful orbital geometry under the
same cap. The CPU selects addresses; the GPU
samples the physical height field and writes split integer/residual vertices.
The kernel reuses the physical octave stack, seed, and canonical cube-sphere
mapping. Unresolved octaves fade out of both height and feedback. One continuous
distance-based footprint is shared by every tile in a snapshot, so a shared
sample does not acquire different heights from its neighboring tile levels.
Each tile adds radial skirts below the validated height envelope to cover
remaining differences between edge triangulations.

A bounded worker generates at most 32 height tiles per batch, with one batch
in flight. Complete snapshots publish after GPU completion and dissolve over
250 ms. Unchanged tiles with the same filter inputs reuse their buffers. A new
filter origin requires resampling; it is selected on camera movement, not every
render frame. Old height coverage remains visible during the build. With the
height-surface normals below, a snapshot uses at most 22,505,472 bytes
(21.46 MiB), plus shared indices and small inputs;
current, retiring and pending snapshots have a fixed bound independent of travel.
Render commands retain immutable buffers through completion. There is no visual
geometry readback. GPU exploration uses the event-loop render schedule instead
of Bevy's pipelined render thread, which stalled once during macOS teardown.
Generation remains on its worker; GPU submission is capped at eight voxel jobs
plus one height batch per frame.

Within 256 m of the ground, the GPU voxel region contains a five-by-five-by-five
cube of chunks around the surface below the camera: 160 m across, 125 chunks,
and one-meter samples throughout. Camera motion reuses unchanged chunk keys.
The pool allows 300 slots, eight jobs in flight, and a 512 MiB allocation budget.
Each slot reserves 20,000 regular vertices and 40,000 regular triangles. Uniform
local resolution needs no transition geometry; one inert transition triangle
keeps the common buffer contract. The complete local region publishes together,
so its displayed bounds never promise coverage that is still being generated.
The generic G3 mixed-LOD extractor remains available and tested.

The local region dissolves into height coverage over its outer 16 m. Reusing
chunks does not restart the whole region's activation fade. The height
surface moves up to one meter inward within that overlap so it cannot fight the
local surface for depth at the tested precision. The displacement follows the
same spatial and 250 ms temporal weight as the voxel draw. This is a rendering
join between two representations, not a watertight hybrid export or a change
to the canonical field. CPU collision continues to use the unchanged full-band
one-meter field. G5 originally used pixel discard during these transitions;
the surface-join follow-up below replaces that rendering path. Skirts and flat
shading can remain visible; this work does not add materials or smooth normals.

The panel and CSV report height tile count, retained height buffer bytes, and
height replacement time alongside voxel generation and render timings. Buffer
counters exclude driver allocations, render targets, and transient command
metadata. The voxel pool retains free slots for reuse but cannot grow past its
fixed slot and byte limits.

Run either preset through the same native route:

```sh
cargo run -p procgen-realtime-pilot -- \
  --design --explore --design-file planet-design-300km.json \
  --explore-record g5-small.csv
cargo run -p procgen-realtime-pilot -- \
  --design --explore --design-file planet-design.json \
  --explore-record g5-large.csv
cargo test -p procgen-gpu-tests --test height_mesh_agreement -- --nocapture --test-threads=1
```

The height suite checks CPU agreement on both presets, exact GPU replay under
reordered submissions, coincident same-level edges, skirt bounds, and repeated
local voxel travel and revisits. Height vertex agreement permits 2 cm plus four
f32 epsilon units times planet radius for cube-direction normalization. The
local voxel density/extraction tolerances remain unchanged. The CPU coverage
suite checks complete face area, bounded selection, deterministic addresses,
face/corner views, and one-meter local resolution on both presets.


### G5 native routes on Metal

Both fixed 90-second routes completed on Apple M1 Max at 2,880 by 2,000 pixels,
including descent, a ground hold, walking, flight and return to orbit. Each
produced six captures and exited successfully, with no stream failure. Recorded
runs now ignore live navigation and movement input. Preliminary runs that were
changed by live controls are excluded from these measurements.

| Segment | 4,900 km frame p50 / p95 | 300 km frame p50 / p95 |
| --- | ---: | ---: |
| Orbit | 8.3 / 8.7 ms | 8.3 / 8.8 ms |
| Descent | 8.3 / 8.8 ms | 8.3 / 13.4 ms |
| Ground hold | 8.3 / 8.7 ms | 8.3 / 8.7 ms |
| Walk | 8.3 / 8.7 ms | 8.3 / 9.8 ms |
| Flight | 8.3 / 8.7 ms | 8.3 / 8.8 ms |
| Return to orbit | 8.3 / 8.7 ms | 8.3 / 8.7 ms |

Peak terrain allocation was 190.8 MiB on the large planet and 229.3 MiB on the
small planet, compared with G4's 5.27 GiB on the large planet. The change in
coverage, rather than radius alone, accounts for the memory reduction. Both
held about 148 MiB at rest on the ground. Frame maxima were 126.5 ms and
122.5 ms, respectively, including screenshot and startup costs. Render
scheduling p95 stayed below 0.4 ms and draw encoding p95 below 0.16 ms in every
segment. These are local observations, not guaranteed frame bounds.

One-meter voxel coverage first became resident at 18.5 seconds on the large
planet and 15.0 seconds on the small planet, after descent began at 5 seconds.
Initial local builds took about 201 ms and 261 ms. The large route's local move
published in 63 ms; small-route updates ranged from about 34 to 195 ms.
Complete height snapshot updates reached 203 ms and 251 ms. Nearby updates
meet the 250 ms target once local coverage is resident in these runs. Initial
coverage and height replacement can still exceed it slightly. Retained height
coverage prevents those waits from exposing an empty planet.

A separate repeated-travel GPU audit visits local offsets 0, 64, 192, 0, 64,
192, 0 m on both presets. It reproduces the same voxel geometry on every return
and stays below 264.7 MiB (large) and 222.0 MiB (small) for voxel allocations.
Submission-to-publication measurements exclude geometry audit readback: 64 m
moves took about 59–70 ms, and 192 m moves about 110–122 ms in the final run.

Validation passes 84 library tests plus the binary tests with and without the
inspector, all Metal G1/G2/G3 suites, three G5 GPU tests, shader validation,
Clippy with warnings denied, formatting, and the native build. Shared coarse/fine
height samples agree exactly on Metal; maximum CPU/GPU height vertex component
errors were 1.281 m for the large planet and 0.0945 m for the small planet,
within the documented planetary direction tolerance. These are not changes to
the much tighter local voxel-density agreement bounds.

Windows/Vulkan execution is recorded in [Windows GPU validation](windows-gpu-validation.md#windowsvulkan-validation-2026-09-14).
Visual refinement of skirts remains. Startup, capture
costs and latency outliers still
need to be considered before treating the measured targets as runtime limits.
The distant surface now omits unresolved geometry, so its orbital appearance is
smoother than G4's unfiltered voxel surface. No material detail replaces those
omitted frequencies in this slice.

## Surface-join follow-up

The Windows ridge captures exposed the pixel discard pattern used for local
overlap. The GPU viewer now renders previous height, current height, and local
voxels into three separate opaque color/depth layers. A full-screen pass blends
their colors with the existing 16 m spatial weight and 250 ms replacement time.
This removes the stipple pattern without changing the generated geometry,
terrain settings, local coverage, or collision.

Each layer resolves its nearest triangle before composition. Local coverage is
stored in alpha; it does not make triangles transparent within the local layer.
The compositor applies local coverage separately to each height snapshot, using
reverse-Z depth to reject local terrain hidden behind a closer height surface.
It then blends the two complete results. A new height surface can fade in even
when it sits behind the previous surface. Missing silhouette coverage blends
over the scene background. The final depth is the nearest contributing surface;
a fully retired snapshot contributes neither color nor depth. This single final
depth is an approximation for future objects crossing a partially blended join.
There are no such objects in the current pilot.

`physical_surface_layers.rs` owns the render targets and compositor. Targets are
reused per view and replaced on resize. There are three RGBA16F/Depth32F pairs,
with a total texture payload of 36 bytes per physical pixel: 197.8 MiB at
2880 × 2000 and 111.2 MiB at 2160 × 1500. This fixed cost depends on resolution,
not distance traveled. The panel and CSV report it as `surface_target_bytes`,
separate from terrain buffers. Other viewer targets, driver padding, and brief
retention of targets during resize are outside this count.

The production compositor has an offscreen GPU test:

```sh
cargo test -p procgen-gpu-tests --test surface_composition -- --nocapture
```

It checks every pixel of uniform fixtures at two target sizes: overlap without
stipple, hidden local terrain, replacement in both depth orders, local visibility
during replacement, exact transition endpoints, missing silhouettes, and empty
layers. RGB tolerance is 0.001 for the half-float target; depth tolerance is
0.00001. The test imports the viewer-owned wgpu module directly; rendering does
not become part of the generation library.

Validation passed on Metal: the compositor test (eleven cases at two sizes),
height compute/render shader validation, all four pilot binary tests, the native
build, and Clippy with warnings denied for the pilot and both affected GPU test
targets. No generation code or agreement tolerance changed.

Metal native validation on Apple M1 Max at 2880 × 2000 completed both 90-second
routes with six captures each and exit code 0. The local recordings are
`/tmp/surface-join-large.csv` and `/tmp/surface-join-small.csv`, with adjacent PNGs
and logs. Descent, ground, walking, and return captures were inspected. The
large ridge no longer has the speckled edge seen in the original G5 descent
capture; inspected views retain terrain coverage. Still captures do not prove
continuous seam-free motion or contact.

| Measurement | 4,900 km | 300 km |
| --- | ---: | ---: |
| Overall frame p50 / p95 | 8.51 / 17.20 ms | 8.45 / 17.10 ms |
| Descent frame p95 | 8.76 ms | 8.78 ms |
| Walking frame p95 | 17.26 ms | 17.87 ms |
| Maximum frame time | 293.26 ms | 140.57 ms |
| Largest segment draw-encoding p95 | 0.127 ms | 0.201 ms |
| Largest sampled local publication | 164.03 ms | 235.67 ms |
| Largest sampled height replacement | 371.81 ms | 316.14 ms |
| Peak terrain buffers | 194.0 MiB | 229.3 MiB |
| Surface blend targets | 197.8 MiB | 197.8 MiB |
| Peak terrain buffers plus blend targets | 391.7 MiB | 427.0 MiB |

Percentiles use nearest rank. No GPU stream failure or overflow was reported.
Frame pacing moved from about 8.3 ms to 16.7 ms during each desktop run; focus
and presentation timing were not recorded. These results include startup and
captures and do not isolate the compositor's GPU cost or establish performance
parity with G5. Frame stalls and height replacement over 250 ms remain unresolved.

This remains a rendered overlap, not a watertight height/voxel mesh. Faceted
normals, skirt geometry, spatial detail changes, collision evidence gaps, and
generation latency outliers remain separate work. The original Windows G5
results above predate this compositor; this follow-up needs its own Vulkan run.

## Height-surface normals

Height tiles now carry a unit outward normal with each vertex. The CPU reference
and WGSL kernel take six central height differences along the planet's Cartesian
axes, project the gradient into the sphere's tangent plane, and account for the
displaced radius. The step is the continuous filter footprint, with a one-meter
minimum. Samples use the same spatially filtered field as the tile. Neither the
normal direction nor the difference step depends on which tile owns the vertex.
Coincident samples can therefore share shading across tile levels, cube edges,
and cube corners. Skirts inherit the normal at their top vertex to avoid a dark
curtain along the join.

Normals are generated with the immutable tile, retained in its vertex buffer,
and interpolated for neutral lighting and the normal view. There are no noise
evaluations in the draw shader. Local voxel shading retained face normals in this
slice; the follow-up below smooths that surface. This change does not move
vertices, alter triangle indices, change either saved terrain preset, or affect
CPU collision.

Each height vertex grows from 32 to 48 bytes. A tile occupies 58,608 bytes and a
384-tile snapshot occupies 21.46 MiB, versus 14.31 MiB before normals. Existing
height memory accounting derives this size from the vertex type. The compositor
targets and voxel pool are unchanged. Six additional height evaluations per
vertex increase tile generation work; measurements follow below.

The CPU test checks radial normals on a sphere and perpendicularity to independent
surface secants on displaced terrain. The height GPU suite checks finite unit
normals, CPU agreement, replay, coincident coarse/fine normals, cube edges and
corners, and skirt inheritance. Normal agreement permits a vector difference of
0.05 (about three degrees) for f32 field differences at planetary coordinates.
Existing position tolerances remain unchanged.

Validation passed: the focused CPU normal test, all three Metal height tests,
the Metal compositor test, native build, formatting, and Clippy with warnings
denied for the pilot and the affected GPU test targets.

Metal CPU/GPU normal differences measured 0.010619 on the 4,900 km preset and
0.013883 on the 300 km preset (about 0.61 and 0.80 degrees). Maximum position
errors remain 1.281006 m and 0.094482 m, respectively, as before normals.

Both 90-second native routes completed with exit code 0 on Apple M1 Max at
2880 × 2000. Recordings are `/tmp/height-normals-large.csv` and
`/tmp/height-normals-small.csv`, with adjacent PNGs and logs. Inspected orbit,
descent, walking, and return captures show smooth distant height shading. The
large preset's nearby voxel patch remains visibly faceted. No GPU stream failure
or overflow was reported.

| Measurement | 4,900 km | 300 km |
| --- | ---: | ---: |
| Frame p50 / p95 | 8.33 / 8.71 ms | 8.34 / 8.80 ms |
| Maximum frame time | 132.63 ms | 458.38 ms |
| Sampled height replacement p95 / max | 236.92 / 298.18 ms | 321.50 / 341.06 ms |
| Largest sampled local publication | 164.03 ms | 303.85 ms |
| Peak terrain buffers | 205.0 MiB | 243.6 MiB |
| Peak terrain buffers plus blend targets | 402.8 MiB | 441.3 MiB |

These runs stayed near 8.3 ms frame pacing, whereas the earlier compositor runs
shifted toward 16.7 ms. They do not establish a speedup from normals or isolate
GPU draw cost. Percentiles use nearest rank; height observations count changes
in the positive rounded CSV value, including initial coverage. Startup and
captures remain in the measurements. Initial local coverage and some height
replacements still exceed 250 ms.

The small preset's flight and return captures retain a warning that walking
paused at missing collision coverage. This is an observed contact/coverage issue
for the separate collision follow-up; completion of the visual route does not
prove continuous contact. Windows/Vulkan validation of these normals remains
pending; commands are in the Windows validation handoff.

## Local voxel normals

Regular voxel meshes now carry the outward density gradient at each surface
vertex. Central differences use the six axis neighbors in the existing density
halo. Edge roots interpolate the gradients at both endpoints with the same
fraction as the position; exact-zero roots use the node gradient. Positive
density is solid, so the outward gradient negates the density derivative.
The renderer interpolates these gradients and normalizes them per fragment.
Shared boundary samples therefore produce the same shading on adjacent uniform
chunks. No new noise evaluations, density buffers, or compute passes are needed.

A zero gradient has no direction and uses the triangle's face normal. Mixed-LOD
transition meshes retain face normals because their sample stencil has no
regular halo. The viewer's local patch uses uniform one-meter chunks. This slice
does not change positions, indices, collision behavior, or either saved preset.

Each voxel vertex grows from 32 to 48 bytes. The pool accounts for the larger
format through the vertex type and retains its 300-slot, 512 MiB limit. A regular
slot with capacity for 20,000 vertices adds 320,000 bytes. Height vertices and
blend targets keep their existing sizes.

The CPU tests check exact plane gradients and analytic quadratic gradients across
chunk boundaries. Metal tests compare CPU/GPU gradients from identical density
samples, check exact shared-boundary normals on a curved field at planetary
coordinates, and retain replay, schedule, topology, overflow, transition, and
travel checks. Each normal component permits a difference of 0.00001 times
max(abs(CPU), 1), allowing five decimal digits for interpolation. Existing
position and density tolerances are unchanged.

Validation passed: all 89 pilot CPU tests, both Metal uniform mesh tests, all five
streaming/transition tests, all three height tests, the compositor test, native
build, formatting, and Clippy with warnings denied. The streaming shader test
needed its shared frame definitions included, matching the render pipeline;
its corrected validation passed. Repeated local travel peaked at 334.9 MiB
(large preset) and 280.0 MiB (small preset), within the unchanged pool budget.

Both 90-second native routes completed with exit code 0 on Apple M1 Max at
2880 × 2000. Recordings are `/tmp/voxel-normals-large.csv` and
`/tmp/voxel-normals-small.csv`, with adjacent PNGs and logs. Ground and walking
captures show smooth local lighting in place of the previous triangle shading.
Ridge silhouettes retain their existing geometry. No GPU stream failure or
overflow was reported.

| Measurement | 4,900 km | 300 km |
| --- | ---: | ---: |
| Frame p50 / p95 | 8.39 / 16.85 ms | 8.34 / 8.97 ms |
| Maximum frame time | 579.93 ms | 124.66 ms |
| Sampled height replacement p95 / max | 290.37 / 338.50 ms | 325.21 / 399.84 ms |
| Largest sampled local publication | 219.41 ms | 252.61 ms |
| Peak terrain buffers | 249.0 MiB | 295.5 MiB |
| Peak terrain buffers plus blend targets | 446.7 MiB | 493.2 MiB |

Percentiles use the same nearest-rank method as the height-normal slice. These
serial native runs include startup and captures and do not isolate the cost of
normal generation or drawing. Frame outliers and height replacements over
250 ms remain. The larger vertex format raises terrain memory use; it remains
within the existing allocation limits.

The large walking capture shows collision coverage rebuilding; the small return
capture did not repeat the earlier missing-coverage warning. These observations
do not establish continuous contact or resolve the prior collision issue.
Skirts, spatial detail changes, and collision coverage remain separate work.
Windows/Vulkan validation is pending, with updated commands in the Windows
validation handoff.


## Height coloring

Height is now the exploration viewer's default color mode, with Neutral, LOD,
and Normals still available. A fixed six-stop ramp spans minus to plus the
configured height limit; the sidebar legend shows kilometers above the reference
radius. This is an altitude display, not water, vegetation, snow, or a sea-level
model. Terrain lighting remains active. The range does not rescale with the
visible terrain or camera distance.

The viewer owns one linear-RGB palette used for CPU packing, the legend, and the
generated WGSL declaration. Local GPU vertices derive radial altitude before
camera rebasing; height vertices carry the already-computed surface height in
the unused fourth normal component. Skirts inherit the surface height at their
top edge, and the overlap depth bias does not alter the color. Both draw paths
interpolate altitude before applying the ramp. CPU audit meshes use the same
palette at each vertex, then interpolate vertex colors through Bevy.

There are no extra noise evaluations, compute dispatches, or vertex-buffer
allocations. The frame uniform grows from 144 to 160 bytes for the reference
radius and height limit. Switching GPU color modes changes only draw state.
Saved terrain parameters and collision behavior are unchanged.

Remaining branch slices are collision continuity, visible skirt and spatial
detail transitions, and frame/height-replacement latency, followed by the
Windows/Vulkan validation of the combined branch.

Validation covers CPU/GPU palette agreement at 513 heights, including values
outside the ramp, the production render shader and compositor, and height
geometry, normals, replay, and skirt inheritance on both saved presets. The new
altitude check verifies that stored height describes the surface geometry using
the existing planetary direction precision budget. The focused CPU height test
also passes. Render shader validation now lives with the compositor test, which
assembles the same palette and frame declarations as the viewer.

Both 90-second Metal routes completed with exit code 0. Captures and logs are
adjacent to `/tmp/height-colors-small.csv` and `/tmp/height-colors-large.csv`.
Orbit and ground captures show the ramp on both surfaces. The small run exposed
a legend layout error; the corrected label row and visible diagnostics were
verified in the large run. These were visual checks with compilation and tests
partly concurrent, so their timings are not performance measurements. Native
build, formatting, and Clippy with warnings denied pass. Windows/Vulkan validation
of the ramp remains pending with the other branch follow-ups.


## Collision continuity

Collision refresh now checks both the current player center and a one-second
motion forecast, with the existing 20 m query margin. The walker's forecast uses
requested tangent movement, current velocity, and gravity when airborne.
Walking and landing keep requesting collision regardless of the height field's
estimated clearance; the estimate can exceed 32 m during a fall. Nearby flight
uses its requested velocity for the same coverage lookahead.

`PhysicalCollision` owns the retained patch and the refresh decision. The build
center advances by at most half a chunk (16 m) per axis so it still covers the
position that requested it. Completed work must cover the current player center
with a one-meter installation margin. A stale completion outside that margin is
dropped without removing retained support. One collision job remains in flight;
source sampling, 27-chunk extraction, triangle contact queries, and the movement
solver are unchanged. Arbitrarily slow work can still reach the coverage edge:
movement must stop safely there, retain its state, and resume after replacement.

The viewer clears a movement warning after the next successful update. Native
CSV rows are recorded after movement and append `collision_building`,
`collision_seconds` (last completed build), `grounded`, and `motion`.
`collision_ready` now checks player-sphere coverage at the walker center (or eye
outside walking), rather than testing whether a patch exists. `motion` records
`Idle`, `Advanced`, `MissingCoverage`, `NoLanding`, `Overlap`, `SweepLimit`, or
`Invalid`. `status` retains its GPU-stream meaning. This distinguishes a falling
player from missing coverage and records short movement failures that a later
screenshot cannot show. If the GPU statistics lock is busy, the recorder retains
the last GPU snapshot and marks `gpu_stats_fresh` false. It still writes the
current collision result, so statistics contention cannot hide a movement failure.

The deterministic CPU tests cover stale completion rejection, early requests,
240 m of walking across multiple chunks, delayed replacement on both saved
terrain presets, and a twenty-second worker stall that must stop and then resume
movement. With 1.5 seconds of simulated build latency, the fifteen-second walks
installed one replacement on the large planet and three on the small planet,
with zero missing-coverage updates. The small fixture flies forward to the first
walkable triangle before landing, as the native route does over its steep start.
All 92 CPU tests pass (89 library and three binary tests).

Both final 90-second native routes completed with exit code 0 on Apple M1 Max /
Metal. Files are `/tmp/collision-final-large.csv` and
`/tmp/collision-final-small.csv`, with adjacent screenshots and logs. Every
walking update reported `Advanced`; none reported missing coverage, overlap,
iteration exhaustion, or invalid motion. These runs validate coverage through
replacement, not a promise that the player stays grounded on steep terrain.

| Measurement | 4,900 km | 300 km |
| --- | ---: | ---: |
| Recorded walking updates | 888 | 835 |
| Walking coverage gaps or failed advances | 0 | 0 |
| Grounded / falling walking updates | 888 / 0 | 383 / 452 |
| Walking updates while replacement builds | 65 | 193 |
| Longest completed collision build | 1.172 s | 1.151 s |
| Rows retaining older GPU statistics | 20 | 9 |

The small route had 53 `NoLanding` updates while flying over its initially steep
face, before walking began. It then landed and completed the walk without an
error. The large route had no landing retries. Successful movement clears the
warning instead of carrying it into later flight/orbit screenshots. The recorder
regression test verifies that a collision failure still gets its own row when
GPU statistics are unavailable. The native build, formatting, and Clippy with
warnings denied pass. No GPU generation or mesh code changed in this slice.

Windows/Vulkan native validation remains pending in the updated handoff. The
remaining implementation slices are visible skirt/detail transitions and
frame/height-replacement latency. Faster-than-covered travel or unusually late
collision work must still stop safely rather than pass through missing terrain.
