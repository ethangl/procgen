# Real-time pilot flight viewer

The default viewer uses GPU height tiles, spherical oceans, and orbit/flight
navigation. Run `cargo run -p procgen-realtime-pilot`. It loads the unchanged
`planet-design-300km.json`; `--design-file`, `--seed`, and `--write-design`
retain their existing meanings. This replaces runtime local voxel rendering,
CPU collision generation, and walking in the physical-scale viewer only.

The intended use is planet inspection, not ground gameplay. The current design
field has one radial height per direction, so a second voxel surface and a
triangle collision index are unnecessary. Full-screen terrain raymarching is
not introduced: the existing height renderer already supplies bounded GPU
terrain generation, distant detail, and continuous orbit-to-surface coverage.

## Generation and publication

The worker selects complete six-face coverage on the CPU and generates height
vertices and filtered normals with wgpu compute. [Reusable height tiles](realtime-world-height-reuse.md)
now derive filtering from the tile layout, so movement can retain unchanged
buffers. The tile budget (384), 64-by-64-quad grids, one-meter refinement target,
stitched edges, colors, and 250 ms transitions remain. Refinement is subject to
the tile budget; this is not a promise of uniform one-meter coverage.

Each worker generation owns immutable height snapshots. At most two batches of
32 tiles are in flight. An edit invalidates unpublished results immediately;
valid drafts coalesce for 350 ms. The worker drains outstanding submissions
before taking the latest request, and revision checks reject stale publication.
The viewer switches the field and height snapshot together once coverage is
ready. Navigation continues while generation runs. Text focus and explicit
file saving retain their existing behavior.

There is no GPU voxel world, CPU collision worker, density extraction, BVH
construction, or local voxel render pass in the default viewer. The renderer
composites only previous/current height snapshots and their oceans. Two
RGBA16F/Depth32F surfaces cost 24 bytes per physical pixel, down from 36:
131.8 MiB at 2880 by 2000, excluding driver padding and other render targets.

Reusable CPU/GPU voxel generation and collision code remain in the library,
as do `--check-chunks`, `--check-residency`, `--check-surfaces`, and
`--check-explore`. `--backend cpu` is still an explicit voxel visual audit.
The `--planet`, `--stream`, capture, sweep, and replay experiments are unchanged.

## Camera behavior

Orbit dragging uses the current screen axes. Right drag looks around. In flight,
W/A/S/D move, E/Q move radially up/down, and scroll changes speed. Continuous
descent and Go to ground still place the camera near the surface.

**Keep camera 5 m above terrain** is enabled by default and is a viewer-only
preference. The library samples the canonical full-detail height at the camera
direction and raises a low camera radially. Each frame checks the endpoint;
there is no swept collision, gravity, walking support, or ocean contact.
Disabling protection allows inspection below terrain. Re-enabling it, or
publishing terrain that covers the camera, raises the camera to the clearance
height without resetting its orientation.

Fast lateral movement can cross a ridge between endpoints. Filtered or still
retiring visual meshes can differ from canonical height, so the check does not
guarantee clearance from every displayed triangle. Oceans remain traversable.
Caves and overhangs are outside this height-field renderer's scope.

## Validation

Focused tests cover altitude protection and terrain edits, unchanged safe
camera positions, disabling/re-enabling protection, stale publication, typing
focus, explicit save/reload, and orbit axes. GPU tests exercise the actual
height and ocean shaders and the two-layer compositor. Canonical CPU voxel
and collision audits remain separate from flight-viewer checks.

```sh
cargo test -p procgen-realtime-pilot --bin procgen-realtime-pilot
cargo test -p procgen-realtime-pilot --lib camera_clearance
cargo test -p procgen-gpu-tests --test surface_composition --test ocean_rendering --test height_mesh_agreement -- --nocapture
cargo clippy -p procgen-realtime-pilot --all-targets -- -D warnings
cargo run -p procgen-realtime-pilot -- --explore-record /tmp/procgen-flight-route.csv
```

The 90-second recording starts in orbit, descends at 5 seconds, moves to five
meters above terrain at 30 seconds and flies forward, rises at 75 seconds,
returns to orbit at 85 seconds, then exits. Six screenshots accompany the CSV.
The CSV replaces voxel/collision columns with `terrain_clearance_m` and
`altitude_protection`; height timings and buffer/target bytes remain. Existing
scripts for the old collision-route schema must be updated.

### Metal smoke result, 2026-09-14

The updated route completed on Apple M1 Max: 5,761 frames and six captures at
`/tmp/procgen-flight-final.csv` and `/tmp/procgen-flight-final.1.png` through
`.6.png`. It recorded forward travel near the surface, ascent, and return to
orbit. Minimum recorded radial clearance was 4.992 m; height sampling and
movement retain f32 direction arithmetic. Near-surface and ascent captures show
continuous height terrain. Surface targets reported 131.8 MiB at 2880 by 2000.
A second viewer was open during part of this run, so it is a correctness smoke
test, not a comparative frame-time benchmark.

Native default launch, automatic octave edits, continued typing after terrain
publication, Save as, and explicit Save were checked. Changing the first octave
from 5000 to 4000, then typing `.5` after generation, retained focus and updated
the terrain. The temporary file remained at 5000 until explicit Save wrote
4000.5. No repository design file changed. The UI tool then reported external
user input, so interactive reload and the protection toggle were left to the
passing state tests. The open test viewer uses `/tmp/procgen-flight-smoke.json`.

All 19 viewer tests, the camera-clearance library test with and without inspector
features, four headless binary tests, and six affected Metal GPU tests passed.
Headless height-preview and one-meter chunk audits passed. Pilot and affected
GPU-test Clippy checks passed with warnings denied; formatting and diff checks
passed. This change still needs a Windows/Vulkan native run. The clearance
check is not a rendered-triangle or swept-collision guarantee.

See [terrain draw visibility](realtime-world-visibility.md) for radial tile bounds, near-first drawing,
and the fixed down/horizon recording route.
