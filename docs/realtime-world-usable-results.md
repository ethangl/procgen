# Usable terrain: slice 4 results

_Historical record. The code described here was removed after commit c7421fd and remains in history._

Measured on 2026-09-13 with seed 42, `PILOT_PLANET`, and `STREAM_SHELL`.
Hardware: Apple M1 Max, 64 GiB, macOS 26.6.2, wgpu Metal. Optimized Cargo
development profile, 1280-by-900 logical window, 2560-by-1800 physical pixels.
Radius remains 4 model lengths, with approximately 0.1 tangential spacing near
face centers and 0.04375 radial spacing. No generation kernel is involved.

## Scope

The fine contour source and its collision index remain resident. Nearby
collision patches and visible rock/post instances are bounded consumers of
that source. A kinematic sphere walks against two-sided triangle distances;
this does not turn the nonmanifold source into a solid-volume physics body.
See [the implementation notes](../apps/realtime-pilot/README.md#usable-terrain-slice-4)
for controls, movement limits, placement rules, and reservations.

Recording begins after initial coarse GPU coverage is ready; walking also
waits for collision preparation and a clear landing. Source/index preparation
is outside the route interval. The native runs record total frame intervals,
render installation work,
walking work, population work, GPU residency tickets, and managed buffer
reservations. The walking route also records minimum sphere-to-triangle
distance, grounded state, delayed frames, and collision failures. Frame time
is the preceding frame interval; per-system measurements are the current
update. They are not a GPU profiler or a claim that these costs sum to total
frame time. Screenshot runs are separate from timing runs.

## Native timing

| Measurement | Walking route | Flight route with population |
| --- | ---: | ---: |
| Frames in 36 seconds | 4,296 | 4,298 |
| Frame p50 | 8.331 ms | 8.298 ms |
| Frame p95 | 8.885 ms | 11.391 ms |
| Frame p99 | 11.205 ms | 16.269 ms |
| Longest frame | 55.382 ms | 42.458 ms |
| Walking work p95 / max | 2.383 / 4.166 ms | — |
| Population work p95 / max | 0.010 / 0.140 ms | 0.007 / 0.045 ms |
| Render installation max | 0.489 ms | 0.906 ms |
| Reserved upload per frame max | 509.44 KiB | 509.70 KiB |
| Peak managed reservations | 68.91 MiB | 71.24 MiB |
| Peak active jobs / queued requests | 2 / 4 | 2 / 4 |
| Process maximum resident set | 216.81 MiB | 220.95 MiB |
| Process peak memory footprint | 416.50 MiB | 429.47 MiB |
| Installations including initial six faces | 14 | 50 |
| Cancellations | 0 | 9 |

Both routes retained all six resident faces and installed no obsolete tickets.
The walking trace contains 8 replacements, zero collision failures, and zero
frames with discarded simulation time. Minimum triangle distance was 0.035041,
above the sphere radius of 0.035. After landing, contact was grounded except
for two initial settling frames. The turn-in-place and final rest phases had
zero position displacement; outward/return displacements were about 2.18/2.17
model lengths. Nearby faces stay fine while walking, including when looking
away. The flight
route supplies cancellation stress; walking does not intentionally churn its
contact region. This demonstrates contact continuity on this route, not on every
seed, unresolved sheet, or cave configuration.

The application buffer, job, and upload limits held. Both p99 frame times fit
the provisional 16.7 ms frame budget, but longest frames exceeded it: a strict
60 FPS frame-pacing claim is still **not established**. An earlier walking run
stalled for about 424 ms at first decoration arrival while using a separate
shader variant. To remove that potential compilation cost, population now uses
the overview/terrain vertex layout and dither variant. The stall did not recur
in the final timing run; this is not an isolated GPU profiling result. Desktop
timing
outliers still vary between runs and require profiling before stronger claims.

## Correctness

All 26 CPU tests pass, along with strict Clippy and formatting. Earlier field
and contour fingerprints remain unchanged. New coverage checks two-sided
high-speed sweeps against a thin triangle, face/edge/vertex distances, missing
coverage, stable resting support during render replacement, walking clearance,
placement recreation after eviction, upper seed bits, and bounded nearest
accepted-landmark queries. Seed 42 accepts 199 rocks and 24 landmarks;
accepted integer candidate IDs have fingerprint `0x015f2282`.

The collision index occupies 1,900,464 bytes (about 1.81 MiB). The scheduler
reserves 8 MiB for collision and population within its existing 128 MiB managed
budget. The nearby patch has at most 8,192 triangle IDs. At most eight new
population instances install per frame, from at most 408 candidates. Two
shared meshes and materials serve all instances. Render installation retains
its 512 KiB admission limit and 2 ms time target. These are application buffer
reservations, not a limit on engine/driver allocation or process footprint.

## Platform status and reproduction

Only the Metal baseline was available for this run. Windows/Vulkan on the
RTX 5070 remains unverified, so slice 4's two-platform exit criterion is **not
complete**. No cross-platform timing or collision claim is made.

```sh
cargo test -p procgen-realtime-pilot --no-default-features
cargo clippy -p procgen-realtime-pilot --all-targets -- -D warnings
cargo fmt -p procgen-realtime-pilot -- --check
cargo run -p procgen-realtime-pilot -- --stream --seed 42 \
  --record /tmp/procgen-usable/walk.csv --walk-route
cargo run -p procgen-realtime-pilot -- --stream --seed 42 \
  --record /tmp/procgen-usable/flight.csv
cargo run -p procgen-realtime-pilot -- --stream --seed 42 \
  --record /tmp/procgen-usable/visual.csv --walk-route --screenshots
```

Each recording writes a CSV and summary; the visual run also writes four PNGs.
Use equivalent output paths on Windows. Process memory measurements on macOS
use `/usr/bin/time -l` around the built executable. Temporary traces are not
repository fixtures.

This session's final timing traces are `/tmp/procgen-usable/contact-focus.csv` and
`/tmp/procgen-usable/final-flight.csv`; the separate images use `final-visual.csv`.
