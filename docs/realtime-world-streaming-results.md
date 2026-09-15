# Streaming pilot: slice 3 route results

_Historical record. The code described here was removed after commit c7421fd and remains in history._

Measured on 2026-09-13 with seed 42, `PILOT_PLANET`, and `STREAM_SHELL`.
The source and run commands are in the [pilot README](../apps/realtime-pilot/README.md#streaming-and-detail-slice-3).

## Baseline

- Apple M1 Max, 64 GiB memory, macOS 26.6.2; wgpu Metal rendering.
- Optimized Cargo development profile. CPU generation; no generation kernel.
- Window: 1280 by 900 logical pixels, 2560 by 1800 physical pixels.
- Radius: 4 model lengths. Reference tangential spacing near a face center:
  about 0.1. Radial spacing: 0.04375.
- Fixed route: 12-second descent at 0.7125 model lengths/s; six seconds of
  turns, ending in two seconds of 0.12-second revolutions to force request
  reversal; 12 seconds of circular flight at radius 5.5 and speed 4; six seconds
  of retreat. Manual flight speeds are 0.5 and 4. There is no grounded movement.
- Timing begins after all initial coarse GPU coverage is ready. Image capture
  is disabled for the timing run. A separate `--screenshots` run records images.

## Timing run

| Measurement | Result | Limit or objective |
| --- | ---: | ---: |
| Recorded frames | 3,988 | 36-second route |
| Frame time p50 | 8.306 ms | 16.7 ms provisional frame budget |
| Frame time p95 | 16.137 ms | 16.7 ms |
| Frame time p99 | 37.079 ms | 16.7 ms |
| Longest frame | 158.717 ms | 16.7 ms |
| Maximum measured installation work | 1.091 ms | 2 ms admission target |
| Maximum reserved upload data in a frame | 509.70 KiB | 512 KiB |
| Peak managed buffer reservations | 63.24 MiB | 128 MiB |
| Peak active jobs | 2 | 2 |
| Peak queued requests, including startup | 4 | 6 |
| Process maximum resident set | 218.69 MiB | Measured separately from buffer reservations |
| Process peak memory footprint | 434.89 MiB | Includes engine and driver-related process costs |
| Cancelled jobs or partial uploads | 10 | Obsolete work must not install |
| Rejected completed worker results | 2 | Must not install |
| Region installations, including six initial faces | 50 | — |
| Frames waiting for requested detail | 441 | Resident coverage retained |

The hard application job, queue, buffer, and upload limits held. The 60 FPS
frame-pacing objective **was not met**. Large stalls occurred both during uploads
and with no active region jobs or upload work. This trace does not identify
their rendering or operating-system cause; profiling is still required. Frame
time measures the preceding frame interval, while installation time measures
work in the current update. These counters cannot isolate GPU cost. Do not
substitute average FPS for these results.

## Correctness evidence

The CSV stores requested and resident serial tickets for each face. Every
recorded replacement used its then-current request ticket. No recorded frame
lost resident face coverage. All three detail levels appeared, and obsolete
work was cancelled and rejected during the turn stress. Coverage in the trace
means resident resources; it is not by itself a geometry proof.

The geometry test independently welds coarse, medium, and fine region products
by their canonical cell identities. Every level shares the same collar
positions and normals. Three arrangements exercise every face at every level;
the assembled meshes have no open edges or unbalanced winding. Because border
geometry is identical for all levels, neighbor detail does not change the join.
Reduction also checks each surviving triangle against its source orientation.
Clusters that reverse or flatten a triangle retain their fine vertices, with
neighboring triangles rechecked until stable. This prevents the cave triangle
flips found during the initial visual run; it does not repair source topology.
The native image run inspects the same construction during camera movement.
Coarse cave silhouettes and shading remain visibly faceted; this reduction is
not a screen-space error guarantee or a final terrain appearance.

The scheduler test withholds the final GPU-ready receipt, verifies that old
coverage remains, injects an obsolete completed worker result after a
fine-to-medium-to-fine request reversal, and verifies retirement accounting.
All 24 CPU tests, strict Clippy, and formatting checks pass. Slice-1 field and
slice-2 contour fingerprints remain unchanged.

## Limits and reproduction

This is bounded render-mesh streaming over a resident CPU source. Density
paging, arbitrary planet scale, collision, and manifold extraction remain out
of scope. The initial source has a separate 1 GiB conservative work reservation;
no region jobs run concurrently with that preparation. The managed buffer cap
is not a cap on all engine allocations, GPU allocator pools, window buffers,
thread stacks, or screenshot readback. Retired products retain their reservation
until render assets disappear and the GPU queue confirms prior work is complete.

Only Metal was run here. Vulkan and the Windows/RTX 5070 timing baseline remain
unverified; no cross-platform performance claim follows from these measurements.

```sh
cargo run -p procgen-realtime-pilot -- --stream --seed 42 \
  --record /tmp/procgen-stream/timing.csv
cargo run -p procgen-realtime-pilot -- --stream --seed 42 \
  --record /tmp/procgen-stream/visual.csv --screenshots
```

Each run writes a `.summary.txt` beside its CSV. The image run also writes four
phase PNGs. This session's traces are `guarded-timing.csv` and `guarded-visual.csv`
in `/tmp/procgen-stream`. Use the commands to recreate them; temporary artifacts
are not repository fixtures. The macOS process memory measurements used
`/usr/bin/time -l` around the built executable.
