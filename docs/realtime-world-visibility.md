# Terrain draw visibility

The GPU viewer rejects off-screen height tiles using radial boxes and draws
remaining tiles from near to far. This changes draw submission only: coverage,
noise filtering, one-meter refinement, tile reuse, oceans, and navigation are
unchanged. Both snapshots use the same visibility rule during an LOD dissolve.

## Bounds and ordering

`physical_visibility.rs` builds a box aligned with each tile's radial direction.
A cone through the tile's farthest cube-patch corner bounds its angular extent.
The box includes the entire configured height range, spherical curvature, and
16 f32 epsilons of the outer radius for CPU/GPU and camera-relative rounding.
Its sides therefore scale with tile width instead of adding the entire planet's
height range on every axis. The box also contains triangle interiors, including
collapsed stitching edges, because it is convex and contains their vertices.

The bound is immutable and shares ownership with its GPU buffer. It is computed
once when that buffer is created, reused with the buffer, and retired with it.
There is no GPU readback or new generation work during camera rotation. Bounds
use host double precision for construction and center rebasing, then Bevy's
float frustum test. This is viewer math, not a CPU/WGSL generation contract.

Visible tiles are sorted by squared distance from the camera to the box's
nearest point. Equal priorities retain address order. This allows nearer
terrain to populate depth before more distant tiles. The renderer continues to
draw both sides of terrain, including when altitude protection is disabled.

These bounds remain conservative: a tile can contain any permitted elevation.
They do not reject terrain hidden behind another hill or behind the planet.
GPU occlusion, backface culling, depth prepasses, and changes to the two surface
layers are outside this change.

## Repeatable audit

```sh
cargo run -p procgen-realtime-pilot -- --visibility-record /tmp/visibility.csv
cargo test -p procgen-realtime-pilot --bin procgen-realtime-pilot
cargo clippy -p procgen-realtime-pilot --all-targets -- -D warnings
```

The 35-second route starts in orbit, then holds the same location while looking
down and at the horizon at 5 m clearance, followed by the same pair at 500 m.
It writes four screenshots. Live controls are disabled for the recording.
For each view, allow the first two seconds for tile publication and blending.

The Status tab and both recording routes report `height_drawn_tiles` and
`height_tested_tiles`, summed across the current and previous snapshots. A
triangle count is drawn tiles times 8,192 grid triangles, including degenerate
edge triangles. `draw_ms` measures CPU encoding, not GPU execution. Frame time
includes presentation and can be limited by the display refresh rate.

Tests cover every cube face and tile level at the minimum and maximum planet
radii, height extremes, stitched mesh vertices and triangle interiors, and
frustum rejection without losing visible extreme heights.

## Metal check, 2026-09-14

The merged tile-reuse commit `f3ab553` was measured with the same recording and
draw counters added, before changing bounds or draw order. Both runs used the
unchanged `planet-design-300km.json` (SHA-256
`3db53f5ee94021d629dc2c1302e2b8fed297c810e2e798b5b3ff37ea550d86c8`).

| Fixed view | Drawn tiles before | Drawn tiles after |
| --- | ---: | ---: |
| 5 m, down | 344 | 192 |
| 5 m, horizon | 304 | 104 |
| 500 m, down | 324 | 168 |
| 500 m, horizon | 290 | 96 |

Every view retained 384 resident tiles. Camera positions, generated tile counts,
and reused tile counts matched between runs. All four images matched pixel for
pixel in the terrain region to the right of the inspector (4,360,000 pixels per
image). The horizon views submitted about 66% fewer tiles and grid triangles.
CPU draw encoding medians fell from 0.061 to 0.042 ms at 5 m and from 0.071 to
0.043 ms at 500 m, measured two to six seconds into each fixed view.

Frame cadence changed between 120 and 60 Hz within both runs, so these results
do not establish an FPS improvement or isolate GPU execution time. The 5 m
view at this location faces a nearby slope; the 500 m view shows more distant
terrain. Other flight locations can have different costs.

Artifacts are `/tmp/procgen-visibility-before.*`,
`/tmp/procgen-visibility-after.*`, and `/tmp/procgen-visibility-results.json`.
All 23 viewer tests, Clippy with warnings denied, formatting, and the build
without default features passed. Windows/Vulkan has not been run.

The 90-second moving route also completed orbit, descent, near-ground flight,
ascent, and return to orbit. Camera positions remained finite; minimum radial
clearance was 4.974 m. Captured ground and orbit views showed complete terrain.
It generated 3,142 tiles and reused 15,290; artifacts are
`/tmp/procgen-visibility-flight.*`.
