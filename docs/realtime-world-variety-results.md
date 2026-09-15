# Variety and reproduction: slice 5 results

_Historical record. The code described here was removed after commit c7421fd and remains in history._

Measured on 2026-09-13 on Apple M1 Max, 64 GiB, macOS 26.6.2.
Rust: `rustc 1.92.0 (ded5c06cf 2025-12-08)`, optimized Cargo development
profile. Geometry and seed checks use the canonical CPU implementation; native
captures use wgpu Metal at 1280-by-900 logical pixels.

## Scope and reproduction

Slice 5 adds three resolved spherical presets, parameter inspection, versioned
case files, actual-camera replay, and a bounded CPU seed sweep. All presets keep
the radius-4 world, 64-by-64-by-16 shell, fixed fine collision source, and
streaming budgets from the earlier slices. See the
[implementation notes](../apps/realtime-pilot/README.md#variety-captures-and-seed-checks-slice-5)
for format limits, controls, thresholds, and commands.

The sweep used source build `8945aae0af60d8d1`. A subsequent single-case rerun
and native captures used `2ac8f6ee5fc8004a`, which adds single-case audit selection
and toolchain provenance without changing generation. Later failure-report
refinements attach a position and tick to missing-support and rest failures
and propagate native error exits to the command line.
Build IDs identify source inputs; they do not assert identical machine code.

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --sweep /tmp/procgen-variety/sweep --samples 2 --sample-seed 20260913
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --sweep /tmp/procgen-variety/rerun --case /tmp/procgen-variety/sweep/hills-42.case.json
cargo run -p procgen-realtime-pilot -- --stream --preset ridges --seed 42 \
  --record /tmp/procgen-variety/ridges-walk.csv --walk-route --screenshots
cargo run -p procgen-realtime-pilot -- --stream \
  --replay /tmp/procgen-variety/ridges-walk.replay.json \
  --record /tmp/procgen-variety/replay.csv --screenshots
```

Sweep output directories must be empty. Local evidence is in
`/tmp/procgen-variety/{sweep,rerun,invalid,captures}`; these temporary files are
not repository fixtures. Saved cases contain every generation parameter and the
full seed, so they can be copied before those temporary directories are removed.

## Seed sweep

Each preset used fixed seeds 0, 42, and 4,294,967,338. Sample seed 20,260,913
added world seeds 10,872,306,431,114,053,753 and 11,109,603,410,324,702,304.
All **15 cases passed geometry, placement recreation, walking clearance, and
stationary-support checks**. Thirteen were flagged as expensive: five hills,
four ridges, and four basins. No sampled case produced an invalid-case failure.

Each case checks fine topology and three mixed-detail arrangements, then walks
4,320 steps at 1/120 second. All cases had no open edges, unbalanced edges, or
degenerate triangles. Known nonmanifold edges remain reported separately. The
minimum triangle clearance across all routes was 0.0350117, above the sphere
radius of 0.035. All stationary phases had zero drift.

Preparation took 496–816 ms, below the 1,000 ms threshold. The expensive flags
came from walking steps over the 4 ms threshold; the largest was 18.329 ms.
These are CPU wall-clock samples, which include host scheduling effects. They
do not establish a GPU bottleneck or a rendered frame budget.

Seed 42 gives the following comparison. Radius is measured at extracted
vertices and includes cave surfaces.

| Measurement | Hills | Ridges | Basins |
| --- | ---: | ---: | ---: |
| Fine triangles | 106,486 | 123,834 | 83,220 |
| Nonmanifold edges | 467 | 975 | 87 |
| Accepted placements | 223 | 224 | 224 |
| Minimum / maximum radius | 3.679 / 4.075 | 4.080 / 4.513 | 3.634 / 4.089 |
| Preparation | 653.8 ms | 775.5 ms | 507.7 ms |
| Mesh and population checks | 330.7 ms | 413.5 ms | 237.0 ms |
| Complete CPU walking route | 4,176.3 ms | 5,236.9 ms | 2,982.3 ms |
| Maximum walking step | 7.088 ms | 14.834 ms | 12.083 ms |
| Minimum triangle clearance | 0.035040 | 0.035012 | 0.035044 |
| Outward displacement | 2.136 | 2.118 | 2.150 |

The hills-42 single-case rerun preserved mesh fingerprint
`2c967459bb7ff7d8` and route fingerprint `e33a275cce22cb55`. These hash quantized
positions, not raw float bits. Its longest step changed from 7.088 to
17.495 ms, while the geometry and route stayed identical. This confirms the
saved-case reproduction path and shows why an expensive flag needs further
measurement before a cause is assigned.

As a negative control, a copy of hills-42 with `planet.band.below = 0.01`
failed at the parameter stage, saved its case and result, and exited with status
1. The error identifies a band that cannot enclose the solid/empty field ends.
This intentionally invalid input is separate from the 15 sampled cases.
Contact failures save a static camera view at the failing position and record
the simulation tick; a parameter failure has no surface location.

## Native comparison and replay

Flight and walking captures were taken for each preset at seed 42, with four
phase images per route. At flight scale, ridges have more broken relief and a
larger radial extent; hills have moderate relief; basins have wider smooth
areas. At ground scale, the ridges route crosses sharper local slopes, hills
have gentler uneven ground, and basins have a much flatter foreground. The
same diagnostic materials and lighting were used throughout. These images show
parameter-driven shape variety; they do not establish finished terrain art.

The six routes completed with no collision failures. The three walking routes
retained positive clearance; their recorded minima were 0.035042, 0.035012,
and 0.035041 for hills, ridges, and basins respectively. Basins had one frame
with discarded simulation time under the existing long-frame clamp; the other
two walking captures had none. Screenshot runs include image readback stalls,
and some overlapped compilation/tests. They are functional and visual evidence,
not replacement timing measurements for the earlier streaming reports.

The completed ridges walking replay contained 3,207 poses. Every output position
and forward vector exactly matched the preceding original pose at that playback
time, and the resolved scenario was unchanged. All six faces remained resident
through the original routes and the replay. The replay uses walking detail
demand but does not run the collision walker, so its contact columns are empty.

An initial replay process ended after about 6.8 seconds without a summary or
logged error. Its cause was not established. A separate rerun completed the
36-second capture, saved all artifacts, and returned status 0. The pose
comparison above uses that completed `replay-check` run. Native error exits now
propagate to the command line; a completion summary and replay artifact remain
necessary evidence that a route finished.

## Validation and remaining limits

The pilot's 28 tests pass, including repeated CPU evaluation, large-seed JSON
round trips, replay time ordering, and the existing source, LOD, collision, and
placement tests. Formatting and Clippy checks pass for the pilot.

Windows/Vulkan has not been measured. Strict 60 FPS frame pacing remains open.
The CPU audit does not exercise asynchronous GPU scheduling; camera replay does
not reproduce physics or job timing. Fixed fine collision, the radius-4 shell,
known nonmanifold extraction, and visual-only rocks/posts remain the bounds of
this pilot. Ridges increase both geometry cost and nonmanifold counts, so they
are a useful stress case before any larger-world or solid-volume claim.
