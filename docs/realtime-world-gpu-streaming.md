# Real-time pilot: GPU generation and incremental streaming

Status: slice G1 has a working GPU density kernel; validation is recorded below.
This replaces the CPU-only visual
generation plan for the physical-planet pilot. The saved terrain settings are
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
crate owns device creation and audit readback; the viewer remains on its CPU
reference until G4.

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

## G4: GPU exploration integration

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

## G5: distant coverage, filtering, and final budgets

Retain a cheap distant height representation and connect it to local voxel
coverage without exposed seams. Filter geometry to its resolution while keeping
nearby collision canonical. Reduce replacement pops. Profile the integrated
pipeline before adjusting workgroup sizes, parallelism, or memory budgets.

Acceptance: measured orbit-to-ground routes on Metal and Vulkan, bounded memory
through repeated travel, explicit failure handling, deterministic revisits, and
stable collision contact. Record p50/p95 frame and update latency and the hardware
used. A fast isolated density dispatch does not complete this phase.
