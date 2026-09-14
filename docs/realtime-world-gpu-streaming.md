# Real-time pilot: GPU generation and incremental streaming

Status: G1 through G4 are implemented and validated on Metal. Physical exploration
uses GPU generation and direct drawing by default; `--backend cpu` selects the
CPU visual audit. G5 and Windows/Vulkan execution remain open.
This replaces the CPU-only visual generation plan for the physical-planet pilot. The saved terrain settings are
held fixed while this work proceeds. The earlier world pipeline is unrelated.

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
latency bounds. Vulkan execution remains pending on the Windows host.

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
Vulkan execution remains pending on the Windows host. These checks do not measure
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
Vulkan execution remains pending on the Windows host.

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
and a native viewer build. Vulkan execution remains pending on the Windows host.

## G5: distant coverage, filtering, and final budgets

Retain a cheap distant height representation and connect it to local voxel
coverage without exposed seams. Filter geometry to its resolution while keeping
nearby collision canonical. Reduce replacement pops. Profile the integrated
pipeline before adjusting workgroup sizes, parallelism, or memory budgets.

Acceptance: measured orbit-to-ground routes on Metal and Vulkan, bounded memory
through repeated travel, explicit failure handling, deterministic revisits, and
stable collision contact. Record p50/p95 frame and update latency and the hardware
used. A fast isolated density dispatch does not complete this phase.
