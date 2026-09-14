# Windows GPU validation

## Procedure

Validate the real-time pilot on Windows with the NVIDIA RTX 5070 through
wgpu/Vulkan. Read `AGENTS.md`, `docs/realtime-world-gpu-streaming.md`, and
`apps/realtime-pilot/README.md` before starting.

Run these commands from the repository root in PowerShell:

```powershell
cargo test -p procgen-gpu-tests --test voxel_density_agreement -- --nocapture
cargo test -p procgen-gpu-tests --test voxel_mesh_agreement -- --nocapture
cargo test -p procgen-gpu-tests --test voxel_streaming_agreement -- --nocapture --test-threads=1
cargo test -p procgen-gpu-tests --test height_mesh_agreement -- --nocapture --test-threads=1
cargo run -p procgen-realtime-pilot -- --design --explore --design-file planet-design-300km.json --backend gpu --explore-record g5-small-vulkan.csv
cargo run -p procgen-realtime-pilot -- --design --explore --design-file planet-design.json --backend gpu --explore-record g5-large-vulkan.csv
```

Each viewer route runs for 90 seconds, writes a CSV and six adjacent PNG
captures, and exits. Live navigation is disabled during recording.

Inspect test output, screenshots, and timings. Check CPU/GPU agreement,
deterministic revisits, bounded terrain allocations, complete terrain coverage,
and ground contact. Distinguish measured evidence from anything the captures or
telemetry cannot establish. Record frame and update latency, failures, and clean
shutdown for both presets. Include GPU, driver, OS, window resolution, and tested
commit in the results.

Update this document with the Windows results and any remaining acceptance
gaps. Keep raw recordings available locally; do not add
them to Git by default. Do not loosen tolerances merely to pass tests. If a check
fails, preserve its exact output and investigate before changing code. Keep
unrelated terrain tuning and visual polish out of this validation work. Do not
commit, push, or create a branch unless asked.

## Windows/Vulkan validation, 2026-09-14

Tested commit: `d03793be14194dd14c2567769ecf5adbe06e948f`, starting from a clean
checkout. The host is Windows 11 Pro 10.0.26200 (build 26200), AMD Ryzen 5 9600X,
and NVIDIA GeForce RTX 5070 (12 GB) with driver 616.92 (Windows 32.0.16.1692).
Rust and Cargo are 1.98.1, target `x86_64-pc-windows-msvc`. Commands use the
repository's optimized dev/test profiles, with debuginfo. Neither preset,
production code, nor test tolerances changed.

The four test commands above passed all 12 tests: density 2, mesh 2,
streaming 5, and height 3. Device output identifies the RTX 5070 and
Vulkan. The first density command could not download dependencies through the
sandbox; its exact output is retained. The approved retry downloaded them and
passed. This was a setup failure, not a failed agreement check.

| Check | Windows result |
| --- | --- |
| G1 density | 1,071,875 CPU/GPU comparisons; maximum error within 32 m of the surface 0.002197 m; far-potential maximum 1 m within the relative bound |
| G1 deterministic samples | All 78,174 coincident halo/parent comparisons exact; repeat and batch-order checks passed |
| G2 extraction | Exact CPU/GPU counts and indices with identical inputs; shared boundaries, schedule changes, exact zeros, capacity sentinels and overflow checks passed |
| G2 saved surface probes | Maximum difference from CPU-density mesh 0.000875 m; from canonical collision 0.122357 m, below 0.25 m |
| G3 streaming | Six transition fixtures closed their internal seams; failure retention, cancellation, retirement, unchanged slots and exact revisit checks passed |
| G5 height | Maximum vertex component error 2.239990 m at 4,900 km and 0.131104 m at 300 km; both within the existing radius-dependent bound |
| G5 deterministic height | Reordered submissions, same-level shared edges, coincident coarse/fine samples and skirt bounds passed |
| G5 repeated local travel | Offsets 0, 64, 192, 0, 64, 192, 0 m passed on both presets; returns reproduced the audited vertices and draw counts exactly |

GPU-density extraction produced 4,885 vertices and 9,488 triangles in the saved
patch, versus 9,496 triangles with CPU density. Near-zero density differences can
change topology; exact CPU/GPU topology is required only with identical input
potentials. The independent surface probes above passed without changing bounds.

G1 pipeline creation took 61.69 ms. The warm 11-chunk density batch took 0.59 ms
for dispatch/completion and 0.91 ms for audit readback; CPU sampling took
171.47 ms. G2 pipeline creation took 328.70 ms; warm saved density plus mesh took
0.30 ms versus 16.75 ms on CPU, with 3.37 ms audit readback. These are isolated
queue/completion measurements, excluding compilation and allocation from the
warm intervals, not viewer frame measurements.

The G3 route published cold coverage in 133.96 ms, reused every slot on the
16 m move, rebuilt 23 chunks in 98.31 ms on the 64 m move, and reproduced the
revisit in 78.47 ms. Peak allocation was 316.00 MiB. The separate G5 repeated
travel audit peaked at 264.7 MiB for the large preset and 222.0 MiB for the small
preset. Its updates took 20.6–47.2 ms and 23.0–52.7 ms, respectively, excluding
geometry audit readback. These tests establish bounded allocation and repeatable
local output over the audited route, not all possible travel.

### Native routes

Both presets completed 90 seconds, saved six PNGs, and returned exit code 0.
The measured window and all captures are 2,160 by 1,500 physical pixels
(1,440 by 1,000 logical pixels at 150% scaling) on a 3,840 by 2,160 desktop.
The reported runs are `g5-large-vulkan` and `g5-small-confirmed-vulkan`. Both
explicitly set `WGPU_BACKEND=vulkan` and
`RUST_LOG=warn,bevy_render::renderer=info`; their logs identify RTX 5070,
driver 616.92, and Vulkan. Live route controls remained disabled.

An earlier small-preset route also completed, but its default warning-only log
did not identify the selected adapter. It is retained as `g5-small-vulkan`,
separate from the explicitly selected Vulkan measurements below. Its peak frame
was 314.7 ms and peak terrain allocation 229.3 MiB. The repeat changed only
backend selection and logging.

Percentiles use nearest rank over recorded rows. Segment labels follow the
scheduled route actions, not proof that each requested movement succeeded.
The final exit row is excluded from the segment table. These desktop runs include
startup, captures and other desktop work; focus and presentation timing were
not logged. Frame pacing changed between roughly 6.25 ms and 16.67 ms within
runs, so these figures do not isolate terrain cost or establish a host speedup.

| Segment | 4,900 km frame p50 / p95 | 300 km frame p50 / p95 |
| --- | ---: | ---: |
| Orbit, 0–5 s | 6.23 / 6.68 ms | 6.24 / 6.53 ms |
| Descent, 5–30 s | 6.24 / 6.59 ms | 6.25 / 6.64 ms |
| Ground hold, 30–60 s | 6.52 / 17.22 ms | 16.65 / 17.35 ms |
| Walk, 60–75 s | 6.29 / 16.94 ms | 16.67 / 17.44 ms |
| Flight, 75–85 s | 6.24 / 6.50 ms | 16.68 / 17.43 ms |
| Return to orbit, 85–90 s | 6.25 / 6.49 ms | 16.49 / 17.33 ms |

The large route recorded 11,454 rows and ended at 90.002 s; the small route
recorded 8,066 rows and ended at 90.010 s. Overall frame p50/p95 was
6.27/16.84 ms and 6.48/17.21 ms, respectively. Frame maxima were 221.7 ms
and 196.2 ms during startup. Later outliers reached 121.5 ms and 90.8 ms
during walking, away from the scheduled capture times. The CSV cannot assign
those stalls to a specific CPU, driver or presentation cause.

| Measurement | 4,900 km | 300 km |
| --- | ---: | ---: |
| First complete height snapshot observed | 0.614 s | 0.568 s |
| First 125 one-meter chunks observed | 18.452 s | 14.842 s |
| Initial local publication | 103.45 ms | 99.10 ms |
| Later useful local updates, p50 / p95 | 22.69 / 22.69 ms (1 observation) | 67.16 / 101.59 ms (15 observations) |
| Height updates, p50 / p95 / max | 74.22 / 201.09 / 370.76 ms (61 observations) | 75.73 / 201.47 / 289.66 ms (71 observations) |
| Largest segment render-scheduling p95 | 0.229 ms | 0.349 ms |
| Largest sampled render-scheduling cost | 14.773 ms | 11.675 ms |
| Largest segment draw-encoding p95 | 0.118 ms | 0.176 ms |
| Peak terrain allocation | 188.9 MiB | 229.3 MiB |
| Peak voxel allocation | 162.6 MiB | 200.6 MiB |
| Peak retained height buffers | 28.6 MiB | 28.6 MiB |
| Terrain allocation at ground hold | 147.8 MiB | 147.8 MiB |
| Retained allocation after return to orbit | 174.6 MiB | 195.9 MiB |

Update observations count changes in each positive, rounded CSV timing value,
not repeated rows carrying the same last value. Local update percentiles exclude
initial coverage and the empty-coverage publication on return to orbit. Height
updates include initial coverage. The reporting mailbox can skip completions or
merge equal rounded values; these are sampled update distributions, not a full
per-job trace. First-detail times include the descent until local detail is
requested, whereas publication timing starts with the local replacement.

GPU stage timestamps were available once voxel jobs completed. Maximum sampled
density/extraction intervals were 0.031/0.134 ms on the large route and
0.025/0.136 ms on the small route. Maximum sampled submission-to-receipt times
were 3.464/4.106 ms; job completion times were 6.492/17.609 ms. Each field carries
its latest stage observation, which need not refer to the same job. These
intervals do not include complete height replacement or explain the frame stalls.

Both routes retained 384 height tiles after the first snapshot, with no later
zero-tile row. The displayed local region contained 125 one-meter chunks near
ground. Peak resident and retiring voxel bytes each reached 133.5 MiB; the peaks
occur at different times and must not be added. Terrain accounting stayed below
the configured limits and returned to a retained reusable pool after travel.
Counters exclude driver allocations, render targets and transient metadata;
they are not total process or device memory measurements.

### Visual evidence and remaining acceptance gaps

All twelve captures from the explicitly selected Vulkan runs were inspected,
as were the six from the first small run. Orbit and return views show a complete
visible planet. The ground views show local terrain without a large missing
region. The large preset has visible stippled overlap edges along the ridge in
captures 2 and 3. Both presets retain faceted surfaces and abrupt changes in
visible detail. Six stills per route do not establish seam-free coverage at every
frame, continuous replacement quality, or a watertight height/voxel join.

Both walking captures report walking on one-meter collision terrain, with
retained collision and measured eye-to-ground clearance of 1.81 m (large) and
2.40 m (small). Walking mode first appears at 60.022 s and 60.930 s, respectively,
and the recorded positions change during walking. The small preset therefore
has about 0.93 s between the walk request and observed walking mode. Its flight
capture has a field clearance estimate of 113.01 m, outside the 24 m collision
ray. The large flight capture instead faces close terrain at 0.21 m measured
clearance. It does not demonstrate unobstructed flight. The nearby flight path
uses collision sweeps; a clear view is not guaranteed by the scheduled action.

No GPU stream failure or overflow appears in either route's CSV or log, and both
processes exited without a shutdown error or forced termination. The CSV records
navigation mode and whether a collision object exists, but omits grounded state,
clearance, collision coverage and collision errors; its status is the GPU stream
status. Thus clean exit and walking rows do not prove continuous contact, absence
of drift or penetration, or zero collision failures. Continuous contact and
unobstructed flight remain acceptance evidence gaps.

Nearby local publication met the 250 ms target in these observations, but height
replacement exceeded it on both presets. Typical render scheduling met 2 ms;
outliers did not. Profiling these stalls and visual overlap edges remains work,
without changing terrain settings or loosening numerical tolerances. The viewer
also warned that `VK_LAYER_KHRONOS_validation` was unavailable, with a loader
registry warning. wgpu/Naga checks passed, but these runs do not include the
optional Vulkan SDK validation layer. Two-host execution is complete; G5 is not
an unconditional visual, contact or frame-budget acceptance.

Raw logs, three CSVs, eighteen PNGs, environment details and analysis results
remain in the ignored local directory `target/windows-gpu-validation/`. No raw
recordings are added to Git. `analyze.ps1` in that directory reproduces the
summaries; no branch, commit or push was made for this validation.

## Surface cleanup: Windows validation pending

The results above predate the smooth surface compositor, height-surface normals,
local voxel normals, height coloring, collision continuity, and stitched height
tiles, bounded height batch overlap, and background PNG encoding. Run the render
and terrain tests, then repeat the two routes from the repository root:

```powershell
$env:WGPU_BACKEND = "vulkan"
$env:RUST_LOG = "warn,bevy_render::renderer=info"
cargo test -p procgen-realtime-pilot --no-default-features physical_collision -- --nocapture
cargo test -p procgen-realtime-pilot --no-default-features height_ -- --nocapture
cargo test -p procgen-realtime-pilot --bin procgen-realtime-pilot physical_ -- --nocapture
cargo test -p procgen-gpu-tests --test surface_composition -- --nocapture
cargo test -p procgen-gpu-tests --test voxel_mesh_agreement --test voxel_streaming_agreement -- --nocapture --test-threads=1
cargo test -p procgen-gpu-tests --test height_mesh_agreement -- --nocapture --test-threads=1
cargo run -p procgen-realtime-pilot -- --design --explore --design-file planet-design.json --backend gpu --explore-record target/surface-join-large-vulkan.csv
cargo run -p procgen-realtime-pilot -- --design --explore --design-file planet-design-300km.json --backend gpu --explore-record target/surface-join-small-vulkan.csv
```

Record the tested commit and adapter, test results, clean exit, frame timings,
and both terrain buffer bytes and the new `surface_target_bytes` column. Inspect
ridge edges, ground views, and return to orbit for stipple, missing coverage, or
stale surfaces. Check for smooth distant shading and lighting seams across tile
edges, smooth ground shading, and lighting seams across local chunk boundaries.
The viewer now starts in Height mode. Check that its kilometer legend stays
fixed during travel and that LOD joins have no cracks or vertical curtains.
Height skirts have been removed; the surface uses balanced, stitched tile edges. Switch
to Neutral and back once in an interactive run. Record results here. Existing
position and density tolerances are unchanged.
Height normals permit a vector difference of 0.05. Voxel normal components from
identical density samples permit a difference of 0.00001 times max(abs(CPU), 1).
Do not retune terrain settings or tolerances during this validation.

The collision follow-up appends `collision_building`, `collision_seconds`,
`grounded`, and `motion` to each CSV row. `collision_ready` now means the retained
patch covers the player sphere, not just that a patch exists. For rows where
`walking` is true, count each `motion` outcome and report any `MissingCoverage`,
`Overlap`, `SweepLimit`, or `Invalid` with its time and build state. Report landing
failures separately. A falling player can have valid coverage while `grounded`
is false. Do not infer uninterrupted contact from the GPU `status` column.

`gpu_stats_fresh` is false on rows that retain the last GPU-statistics snapshot
because its lock was busy. Collision results on those rows are current and must
be included when counting movement failures.


The latency follow-up keeps at most two 32-tile height batches in flight and can
build during the preceding fade. CSV columns `height_build_ms` and
`height_wait_ms` separate build time from the wait to publish after that fade.
Report both, plus the existing total `height_update_ms`. They are elapsed times,
not GPU execution timestamps. PNG encoding now runs on the I/O pool; verify all
six images exist after each clean route exit. Compare frame p95/p99 by route phase
and report capture windows separately (4, 25, 55, 73, 83, and 89 seconds). Keep
startup and capture outliers visible rather than dropping them from the report.
