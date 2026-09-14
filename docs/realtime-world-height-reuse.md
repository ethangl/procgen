# Reusable GPU height tiles

The flight viewer keeps terrain buffers when movement leaves their tile and
neighbor detail unchanged. Terrain generation no longer takes a camera-dependent
filter. The default remains GPU height terrain, oceans, and orbit/flight;
local voxels and collision are not part of this viewer.

## Stable products and shared boundaries

A height tile is identified by its cube-face address, its coarse-edge stitching
mask, and four corner detail spans. Design revisions own separate tile streams,
so buffers from a previous design cannot be reused after an edit.

The coverage selector retains the existing 384-tile budget, 64-by-64-quad grids,
one-meter refinement target, and balanced neighboring levels. Once it chooses
a layout, corner detail comes from the coarsest tile meeting that corner. A
fine corner in the middle of a coarse edge interpolates that edge's endpoints.
These constraints are resolved from coarse to fine, including across cube faces.

Each tile interpolates its four corner spans with integer arithmetic. At a
shared sample, fine and coarse tiles therefore obtain exactly the same filter
footprint. The existing edge collapse then gives the same vertices and straight
segments on both sides. Normal samples hold that local footprint fixed, so the
same position also gets the same normal. Normals describe the locally filtered
field; they do not include the derivative of changing filter width across a tile.

The footprint is interpolated corner spacing divided by `HEIGHT_DETAIL_RATIO` (8), with a
0.25 m minimum. The ratio retains finer bands where the fixed tile budget and
corner constraints make geometry coarser than its ideal distance-based LOD.
This replaces the previous distance/512 filter. It is an intentional rendering
change: distant filtered heights and lighting are not identical to the earlier
viewer, and orbit can retain more fine relief. Seed, noise controls, height
bounds, ocean settings, color ramp, and saved design files are unchanged.

## Residency and publication

A new target reuses matching buffers from the displayed snapshot. Only missing
or changed identities are generated. An unchanged target requires no generation,
even if the camera moved. Detail changes around a retained address can still
invalidate its buffer when its stitching or corner spans change.

Generation still permits two batches of 32 tiles in flight. Complete snapshots
publish after completion and use the existing 250 ms compositor fade. Both
snapshots are independently closed; blending does not hide open tile seams.
Queue-completion ownership still protects submitted buffers from early release.

This is reuse of the current resident cover, not a permanent planet cache.
Leaving a region can release its buffers. Returning regenerates the same
products deterministically. Current, previous, and pending snapshots remain
bounded by three covers (222.8 MiB before sharing), plus at most 12.38 MiB of
batch output and small metadata. A GPU vertex still occupies 48 bytes; a tile
request now has 48 bytes of metadata. Two surface blend targets are unchanged.

The Status tab and route CSV add cumulative `height_generated_tiles` and
`height_reused_tiles` for the current design generation. They count work admitted
for new target snapshots. A target identical to the current snapshot adds
neither count because it starts no work. GPU height-build timings still include
submission and completion waits; they are not hardware timestamps.

## Validation

```sh
cargo test -p procgen-realtime-pilot --lib height_
cargo test -p procgen-realtime-pilot --bin procgen-realtime-pilot
cargo test -p procgen-gpu-tests --test height_mesh_agreement --test height_reuse -- --nocapture
cargo clippy -p procgen-realtime-pilot --all-targets -- -D warnings
cargo run -p procgen-realtime-pilot -- --explore-record /tmp/procgen-reuse-route.csv
```

CPU tests check exact shared footprints through complete mixed-level layouts at
radius extremes, cube seams/corners, retained identities during small movement,
neighbor-driven invalidation, deterministic revisit, and changed designs.
GPU tests compare canonical CPU results, replay/reordered batches, isolated
stitching masks, and actual selected layouts with exact shared positions and
normals. Existing edit invalidation, stale-result, typing, save/reload, and
bounded submission tests remain in place.

A scratch investigation is in `/tmp/procgen-reuse-audit`. It is not required to
build or run the viewer. Its pre-change snapshot is main `306392b`; its saved
300 km design has SHA-256
`3db53f5ee94021d629dc2c1302e2b8fed297c810e2e798b5b3ff37ea550d86c8`.

### Metal results, 2026-09-14

The 90-second native route ran sequentially on the baseline and this change,
using that unchanged design. It completed orbit, descent, near-ground flight,
ascent, and return to orbit. Positions stayed finite; minimum measured radial
clearance was 4.974 m in both runs. Captured orbit, ground, and ascent views
showed closed terrain. Ground views were similar; orbit showed finer relief.

| Measurement | Baseline | Reuse |
| --- | ---: | ---: |
| Completed height updates | 118 | 50 |
| Median height build, all updates | 137 ms | 33 ms |
| Median height build, near-ground flight | 139 ms | 36 ms |
| Peak resident height buffers | 222.8 MiB | 143.9 MiB |

The new route generated 3,164 tiles and reused 16,036 (83.5% of admitted tile
work). Near-ground flight generated 422 and reused 3,034 (87.8%). Timing
medians count each changed completion measurement once, not once per frame.
CSV files and six images per run are at
`/tmp/procgen-reuse-baseline-final.*` and `/tmp/procgen-reuse-final.*`.

These are smoke measurements, not hardware compute benchmarks. Frame cadence
was about 120 Hz in the baseline and changed to about 60 Hz during the new run,
including its final stationary orbit. The cause was not isolated, so this run
does not establish an overall frame-rate improvement. Cold launch and a global
design edit still generate a full cover; cold launch measured 183 ms versus
205 ms here. Returning to evicted regions also requires generation.

All nine focused library tests, 19 viewer tests, and four Metal GPU tests
passed. The library tests also passed without default features. Clippy,
formatting, the headless preview check, and native launch passed. Windows/Vulkan
remains untested for this change.
