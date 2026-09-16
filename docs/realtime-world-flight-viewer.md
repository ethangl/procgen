# Real-time pilot flight viewer

The default viewer uses GPU height tiles as the far field, a GPU voxel world as
the near field, spherical oceans, and orbit/flight navigation. Run
`cargo run -p procgen-realtime-pilot`. It loads the unchanged
`planet-design-300km.json`; `--design-file`, `--seed`, and `--write-design`
retain their existing meanings. CPU collision generation and walking remain
removed from the physical-scale viewer.

The intended use is planet inspection, not ground gameplay. A triangle collision
index is not maintained. Full-screen terrain raymarching is not introduced: the
existing height renderer already supplies bounded GPU terrain generation,
distant detail, and continuous orbit-to-surface coverage.

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

The same worker drives a `VoxelGpuWorld` next to the height stream. It keeps a
camera-centered band of mixed-LOD chunks: `select_voxel_band` asks the octree
selection for 384 leaves, balances them 2:1, then drops every leaf whose cell
spacing exceeds 16 m. A subset of a balanced partition is still balanced, so
nothing is rebalanced. Chunks at the band edge have no coarser neighbor and mesh
their outer faces as ordinary cells; that edge dissolves into the height tiles
rather than stitching to them. There is no altitude gate. The band empties on its
own once the camera is far enough that nothing within two chunk spans is finer
than the cap, which happens a few kilometers above the design's height envelope.
Measured over both presets, four directions, and clearances from 5 m to 50 km,
the band held up to 1,224 chunks. Replaying the height audit's travel route on
both presets held at most 1,262 slots resident, in flight, or retiring at once,
with a worst regular mesh of 8,403 vertices and 16,418 triangles and a worst
transition mesh of 16,251 vertices and 5,417 triangles. The world reserves 1,536
slots of 2,781,000 bytes, 3.978 GiB, inside a 5 GiB budget. The slot count is
bound by the reservation a coverage change makes, not by peak residency.

The worker advances one closed replacement group at a time: when the world is
settled and the current coverage differs from the target band, it takes a single
`VoxelCoverage::step_toward` and waits for settlement before the next step. An
empty coverage has nothing to refine, so a band returning from orbit restarts
from the root chunk. A design is published once the height stream is ready and
the world is settled on its *current* coverage, not necessarily on the final
target; the band then refines live while the viewer draws.

The renderer draws each visible lease into its own Local layer and the compositor
places it over the height surface. Both surface-join distances now follow the
band's coarsest resident chunk: the dissolve width is that chunk's span, floored
at the previous fixed 16 m, and the height surface is pushed radially inward by
that chunk's cell spacing **plus the volume term's amplitude** (zero when the
term is disabled, which restores the old rule exactly), floored at the previous
fixed 1 m. The second part is there because the volume term can move the voxel
surface *below* the height surface as well as above it, and without it the
height surface shows through every undercut. That part is a true bound, not an
estimate: the term's shaped noise passes through the design's
`x / sqrt(1 + x*x)` before the amplitude scales it, so the term stays strictly
inside plus or minus `amplitude_m` whatever the sharpness. Bounding it that way
was necessary, not decorative. The shape transform has unit deviation rather
than a unit bound, so an unbounded term made the amplitude a one-sigma scale: a
6 m amplitude at sharpness 0.5 measured -3.3 m to +26.0 m over the 300 km
design's surface samples, and a negative sharpness puts that long tail on the
undercut side, where the bias rule cannot survive it. Bounded, the same shipped
term measures -2.9 m to +5.7 m inside its 6 m, and a unit test walks both signs
of sharpness at a 64 m amplitude to confirm the bound holds and is approached.
The amplitude part is now larger than it needs to be and is kept unchanged on
purpose: the far field carries a projection of the term, so the two surfaces
differ by the projection's measured first-order error, 2.375 m for that 6 m
term, rather than by the whole amplitude. That measured number is what a later
tune should read. The first part of the rule remains provisional for its own
reason: one coarse cell is a guess at how far a 16 m extraction can stand above
the height surface, not a measured bound, and route evidence or a per-chunk bias
should replace it. No fragment discard was added: the band's bounding box can
contain surface the band does not cover, so a discard there would open holes.
**Show local voxels** disables the pass and zeroes that coverage weight. There is
no CPU collision worker and no BVH construction. Three
RGBA16F/Depth32F surfaces cost 36 bytes per physical pixel: 197.8 MiB at 2880 by
2000, excluding driver padding and other render targets.

Each lease's origin record now carries its chunk level in its fourth word, so LOD
coloring separates the band's spacing rings from the height tiles underneath
instead of painting the whole near field one color. What the two systems still
share is the field: the voxel mesher extracts the same single-valued height, so
where the band is one meter and the tiles have also refined to one meter, the
silhouette agrees and only the coloring differs. What differs is everything the
band adds away from that case: distinct 1, 2, 4, 8 and 16 m cell sizes across the
near field, and a surface that stays voxel-extracted out to the cap instead of
ending at a 256 m altitude gate. A GPU test renders the local pipeline into its
own layer and checks that it writes pixels and that zero coverage removes them,
because the panel's chunk counts are CPU-side and cannot show this. The near
field carries shapes a radial height cannot express, because the density field
carries one bounded 3D detail term faded out with height above the surface.

The far field now carries that term too. `PlanetDesignField::surface_height`
evaluates the term's unfaded shape on the band's own surface, where the fade is
one, and adds it to the filtered height: `h + w * shape(dir * (radius + h))`,
with `w` the same footprint smoothstep the octaves use, so a tile too coarse to
resolve the 24 m wavelength carries exactly the bare height and coarse tiles
stitch through their shared footprints unchanged. Filtering is correct here and
forbidden in the density path, where coincident samples at different levels must
stay bit-identical. The projection is first order. The true zero crossing along
the radial ray solves `h - a + volume(P(a), h - a) = 0`, and evaluating at
`a = h` ignores the term's own variation over that displacement. Bisecting the
canonical potential along the ray on the 300 km design with the shipped 6 m term
measures the residual at 0.554 m over eight surface directions and 2.375 m over
104; with the term disabled the projected surface is bit-identical to the height
surface. The bisection resolves the ray parameter to a millimeter, but the f32
position it evaluates steps by about three centimeters at 300 km, which is the
floor of the measurement and reads 0.044 m on the disabled run.

Reusable CPU/GPU voxel generation and collision code remain in the library.
There is no CPU visual backend and no `--backend` flag.

## Coloring

**Material** is the default coloring and the first mode in the panel. It is a fragment
rule over the existing shading, not a generation change: nothing about height tiles,
the voxel band, or the field moves. Each fragment takes `slope = dot(normal, up)` with
`up` the normalized planet-space position, and its altitude above the reference radius,
and blends four materials with smoothsteps so nothing bands. Soil holds below 28
degrees and becomes rock above 42. Snow comes in from 55 percent of the configured
height limit and is full by 65, multiplied by a slope factor that is one below 35
degrees and zero above 50, so cliffs stay rock through the snow line. Shore sand is
full within 4 m above sea level and gone by 8 m, only when the ocean is enabled and
only where the face is gentler than the rock threshold; it has no lower bound, so
ground just under the waterline reads as sand too. The directional light and ambient
term are applied afterwards exactly as Height mode applies them.

The four colors and the four angles are starting values, not measured ones. They live
once in `physical_color.rs` as `MaterialRule` and feed two consumers: the CPU
`material_color`, which draws the panel legend's four swatches, and the generated WGSL
`material_color` the terrain shader calls. A GPU test dispatches the generated shader
over a grid of slope cosines and altitudes with the ocean on and off and compares it to
the CPU function within 1e-6 per component, the same way the height palette is checked.

**Height**, **Neutral**, **LOD**, and **Normals** remain as inspection modes and are
unchanged. Height maps altitude through the six-stop ramp; LOD reads each lease's chunk
level, so the band's spacing rings separate from the height tiles underneath.

## Camera behavior

Orbit dragging uses the current screen axes. Right drag looks around. In flight,
W/A/S/D move, E/Q move radially up/down, and scroll changes speed. Continuous
descent and Go to ground still place the camera near the surface.

Base flight speed stays at 8 m/s through 64 m terrain clearance, then increases
continuously by 0.6 m/s per meter of additional clearance, capped at 2,000,000
m/s before the scroll multiplier. The panel shows the resulting speed. Speed
does not depend on the design's height limit: the former switch at that limit
plus 100 m could slow high-altitude flight by more than 100 times during descent.
Movement-system tests exercise all six keys on both sides of that old boundary.

**Keep camera 5 m above terrain** is enabled by default and is a viewer-only
preference. The library samples the projected surface at the camera direction,
the same surface the tiles draw, and raises a low camera radially. The inspector's
clearance readout uses that query too, so the number and the protection cannot
disagree about where the ground is. Each frame checks the endpoint;
there is no swept collision, gravity, walking support, or ocean contact.
Disabling protection allows inspection below terrain. Re-enabling it, or
publishing terrain that covers the camera, raises the camera to the clearance
height without resetting its orientation.

Fast lateral movement can cross a ridge between endpoints. Filtered or still
retiring visual meshes can differ from canonical height, so the check does not
guarantee clearance from every displayed triangle. Oceans remain traversable.
Protection now sees the volume term through the projection, so the gap it keeps
is bounded by that projection's measured error, 2.375 m for the shipped 6 m
term, rather than by the term's whole amplitude. It is still a single radial
query against a single-valued surface: it cannot see an overhang the band
extracts above that surface, and it can hang in air the term has carved out
below it. Caves and overhangs are outside this height-field renderer's scope.

## Validation

Focused tests cover altitude protection and terrain edits, unchanged safe
camera positions, disabling/re-enabling protection, stale publication, typing
focus, explicit save/reload, and orbit axes. The far-field projection is checked
against a bisected band crossing, against exact equality with bare height when
the term is disabled or filtered out, and for schedule independence. GPU tests
exercise the actual height and ocean shaders on both presets with the term off
and on, the three-layer compositor, and the resident local leases and their draw
records. Canonical CPU voxel and collision audits remain separate from
flight-viewer checks.

```sh
cargo test -p procgen-realtime-pilot --bin procgen-realtime-pilot
cargo test -p procgen-realtime-pilot --lib camera_clearance
cargo test -p procgen-gpu-tests --test surface_composition --test ocean_rendering --test height_mesh_agreement -- --nocapture
cargo clippy -p procgen-realtime-pilot --all-targets -- -D warnings
cargo run -p procgen-realtime-pilot -- --explore-record /tmp/procgen-flight-route.csv
```

The 90-second recording starts in orbit, descends at 5 seconds, moves to five
meters above terrain at 30 seconds and flies forward, rises at 75 seconds,
returns to orbit at 85 seconds, then exits. Six screenshots accompany the CSV. The
route uses the default coloring, so the captures are material-colored. Existing
scripts for the old collision-route schema must be updated: `terrain_clearance_m`
and `altitude_protection` replace the collision columns, and the local voxel counts,
bytes, and stage timings follow them.

The header `PhysicalRecord::new` writes carries 40 columns in this order:

1. `seconds`, `phase` — elapsed time and the route stage index.
2. `frame_ms` — viewer frame time.
3. `height_tiles`, `height_bytes`, `height_update_ms` — resident height tiles, their
   GPU buffer bytes, and the time to apply the last snapshot.
4. `selection_ms`, `scheduler_ms`, `draw_ms` — coverage selection, residency
   scheduling, and draw encoding.
5. `x_m`, `y_m`, `z_m` — camera position in meters.
6. `surface_target_bytes` — the three-layer surface target payload.
7. `status` — the panel's status line.
8. `gpu_stats_fresh` — false when the last snapshot is reused because its lock is
   busy; camera clearance stays current either way.
9. `height_build_ms`, `height_wait_ms` — preparation, submission waits and GPU
   completion, and waiting for the preceding fade. Both are elapsed times, not GPU
   timestamps.
10. `terrain_clearance_m`, `altitude_protection` — radial clearance and whether the
    camera protection is enabled.
11. `height_generated_tiles`, `height_reused_tiles` — cumulative tile work.
12. `height_drawn_tiles`, `height_tested_tiles` — per-frame visibility results.
13. `local_resident`, `local_drawn`, `local_target`, `local_in_flight`,
    `local_retiring` — voxel band slot counts.
14. `finest_spacing_m`, `coarsest_spacing_m` — the band's cell spacing range.
15. `gpu_bytes`, `local_resident_bytes`, `local_retiring_bytes` — voxel GPU bytes,
    total and by residency state.
16. `preparation_ms`, `encoding_ms`, `completion_ms`, `submission_latency_ms`,
    `publication_ms` — voxel generation stage timings.
17. `gpu_density_ms`, `gpu_extraction_ms` — GPU timestamps where the device reports
    them.

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
