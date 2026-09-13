# Surface quality

This follows the five-slice real-time world proof of concept. The visual target
is neutral terrain with better geometry and surface quality. This work does not
use the older world-heightmap pipeline.

## Slice 1

The streaming inspector starts with a gray, rough PBR material and the existing
fixed sun, ambient light, exposure defaults, and tone mapping. The initial
whole-planet placeholder also uses gray. Once region coverage is ready, the
inspector can compare three normal sources on the same mesh:

- **Averaged:** area-weighted triangle normals, with the existing shared fine
  border normals. This is the previous shading method.
- **Triangle:** the rasterized triangle's geometric normal. This exposes facets
  without changing positions, triangles, collision, or LOD selection.
- **Density:** central differences of the final composed density, including
  height, detail, and cave subtraction, sampled at the actual LOD vertex
  positions. The positive-solid field makes the outward normal oppose its
  gradient. Normals interpolate across triangles; this is not per-pixel field
  evaluation or added geometry.

The finite-difference offset is 0.001 model lengths, below the shell's 0.04375
radial spacing. Density clamping can produce a zero gradient at vertices away
from the surface. Zero or cancelling interpolated normals appear magenta;
inspection does not substitute a radial or mesh normal. A cave boundary or a
crease can also have no unique smooth normal.

Neutral shading can be replaced by LOD colors, normal directions, or a comparison
of averaged mesh and density normals. Agreement is dark when aligned, yellow
when perpendicular, and red when opposed. Direction colors encode
`(normal + 1) / 2` in planet coordinates before tone mapping. Triangle edges can
be drawn over any view. The edge overlay uses barycentric coordinates in the
fragment shader and does not require polygon-line GPU features.

View changes update a material uniform. They do not rebuild the mesh, change
terrain parameters, or create a shader variant for each mode. The shader keeps
the existing visibility-range dither so replacement behavior remains active.
Bevy 0.18 rebuilds its shared visibility-range table when a range changes;
the renderer refreshes all participating mesh references so unchanged instances
cannot retain old table indices and remain partly dithered after replacement.
Density normals are computed in background region jobs, using parallel sampling
for larger products. Shared border positions receive the same field queries.

## Use and comparison

```sh
cargo run -p procgen-realtime-pilot -- --stream --preset ridges --seed 42
cargo run -p procgen-realtime-pilot -- --stream --preset ridges --seed 42 \
  --normals triangle --wireframe
cargo run -p procgen-realtime-pilot -- --stream \
  --replay /tmp/procgen-variety/captures/ridges-walk.replay.json \
  --normals density --surface agreement \
  --record /tmp/procgen-surface/agreement.csv --screenshots
```

`--normals` accepts `averaged`, `triangle`, or `density`. `--surface` accepts
`neutral`, `lod`, `normals`, or `agreement`. `--wireframe` enables triangle
edges. Slice 4 adds `--surface-detail plain|textured`, with `textured` as the
default. These options require `--stream`. The UI exposes the same settings.

Generation cases and camera replay files keep their existing format. Each new
recording saves its surface settings in a separate `.view.json` and repeats
them in the summary. Recording locks these UI controls; use another replay run
to compare settings on the same camera path. The source build identifies the
fixed lighting and shader. The initial whole-planet placeholder uses averaged
neutral shading; comparisons begin after region coverage is ready.

The mesh buffer now expands triangle corners to carry barycentric coordinates.
The reservation is 156 bytes per triangle per copy, including positions,
averaged normals, density normals plus LOD, UVs, and indices. The scheduler
reserves both CPU and GPU copies. Slice 1 used a 128 MiB managed limit;
slice 3 increases it to 256 MiB for refined geometry. The 512 KiB upload limit
stays fixed. Region memory also includes the added
normal vector. More vertices and CPU field queries are a measured inspection
cost, not a claim of free rendering detail.

## Following slices

1. Neutral surface inspection and normal comparison: complete.
2. Repair demonstrated extraction defects, including ambiguous cells and
   nonmanifold surfaces: complete; see the repair record below.
3. Finer nearby geometry with valid joins and collision support: complete;
   see the bounded refinement record below.
4. Restrained surface roughness and normal detail with stable mapping: complete;
   see the material record below.

Normal selection cannot repair topology, change a silhouette, or increase the
64-by-64-by-16 source resolution. The comparison tools should identify which
later changes have a visible effect before adding more detail.

## Measured inspection

Checked on 2026-09-13 on Apple M1 Max / Metal, 64 GiB, macOS 26.6.2,
with the optimized development profile and a 1280-by-900 logical window.
Source build `f1e819edff9e91d3` supplies the rendered comparisons. A later
allocation-only adjustment reserves the exact corner count up front.

The comparison uses ridges, seed 42, and two poses from the slice-5 captures:
the last recorded ground pose at or before 1.5 seconds, held for 18 seconds,
then the flight pose at or before 1.5 seconds, held until 36 seconds. It keeps
the walking replay's radial camera-up rule throughout. These two held views
make each mode's screenshots directly comparable after LOD work settles.
Temporary captures, replay inputs, and CSVs are in `/tmp/procgen-surface`.

The three neutral runs completed with 4,212 averaged, 4,107 triangle, and
4,261 density frames. Agreement with triangle edges completed with 3,332
frames, exposing localized disagreement around the visible slopes. Every recorded pose matched its source pose exactly.
All six faces remained resident, and no changed resident ticket differed from
its requested ticket. Peak managed reservations were 95,855,920 bytes
(91.42 MiB), with at most 498,264 upload bytes in a frame, within the existing
128 MiB and 512 KiB limits. These screenshot runs overlapped CPU checks and
include readback stalls; they do not establish frame pacing or a GPU cost.

At ground scale, averaged normals soften large facets. Triangle normals expose
substantial triangle-to-triangle changes in slope. Density normals create
stronger local changes in lighting while keeping the exact same silhouette;
this exposes the difference between the field and its current mesh
approximation. It is evidence for investigating extraction and resolution,
not evidence that density shading alone improves the surface. At flight scale,
the normal choices change relief shading while the planet boundary stays fixed.

An early comparison exposed persistent dither after region replacement. Refreshing
mesh references when the shared range table changes removed that pattern while
preserving the fade. The completed comparisons above include that fix.

The live inspector also switched between averaged and density normals and
toggled triangle edges while retaining the same six installed regions and
zero generation jobs. It was restored to the neutral averaged view.

All 29 pilot tests pass, including outward normals on an unmodified sphere,
finite normals and exact shared-border agreement across LODs, and existing
mesh, contact, determinism, and replay checks. The saved hills-42 CPU audit
retained mesh fingerprint `2c967459bb7ff7d8` and walking-route fingerprint
`e33a275cce22cb55`, with zero rest drift and minimum clearance 0.035040.
Its longest walking step was 19.3 ms and remains flagged as expensive.
Windows/Vulkan validation and strict frame pacing remain open.


## LOD density ordering correction

Wireframe inspection exposed a reduction error: the blue coarse level could
contain more triangles than the green medium level. Both levels independently
started from fine geometry. Rejected 4-by-4 coarse collapses restored fine
vertices, while smaller 2-by-2 medium collapses could still succeed.

Coarse now starts from the medium result. A rejected collapse keeps the
preceding level, and accepted collapses must preserve surviving triangle
orientation relative to both that level and the fine source. Fixed borders
remain unchanged. Medium and fine geometry remain unchanged. This is a render
reduction correction, not a source-field or collision change.

Ridges, seed 42, measured triangles:

| Face | Previous blue | Corrected blue | Green | Fine |
| --- | ---: | ---: | ---: | ---: |
| +X | 17,530 | 14,750 | 15,970 | 21,236 |
| -X | 16,776 | 14,168 | 15,482 | 20,350 |
| +Y | 17,146 | 14,308 | 15,462 | 20,582 |
| -Y | 17,916 | 15,048 | 16,396 | 22,142 |
| +Z | 16,598 | 13,638 | 14,698 | 19,474 |
| -Z | 16,130 | 13,392 | 14,896 | 20,050 |

All six faces of hills, ridges, and basins at seed 42 now satisfy
`coarse triangles <= medium triangles <= fine triangles`. Regression coverage
checks this order and verifies that each medium vertex maps to one coarse
vertex, so the hierarchy also holds locally. Existing orientation, shared
normal, mixed-LOD seam, and contact tests pass. Earlier captures in this report
predate this correction.

A 36-second native Metal flight with the corrected hierarchy, LOD colors, and
triangle edges completed all three levels across 3,175 recorded frames. It had
no missing resident faces, obsolete installations, or logged errors. Peak
managed memory was 89,800,720 bytes, and the largest frame upload was 498,264
bytes, both within the existing limits. Screenshot capture was enabled, so this
run is a coverage check rather than a frame-pacing benchmark.


## Slice 2: extraction repair

The original extractor joined all edge crossings in a cell at one QEF vertex.
That joined separate sheets and gave hills, seed 42, 467 edges with more than
two incident triangles. Surface-quality slice 2 fits each closed contour cycle
separately. Each sampled face connects its two crossings directly. A face with
four crossings uses its bilinear saddle determinant to select the pairing;
an exact tie selects the diagonal with the lower shared sample IDs. Adjacent
cells therefore make the same decision, including across cube faces.

Separate cell vertices remove 463 of the 467 bad edges. The remaining four
join the same two sheets twice across an ambiguous face. They are distinct
face segments, but would share one indexed dual edge. Each segment now gets
its own shared midpoint between its two edge roots, and the incident triangles
split at that point. This retains both connections without joining them into
one edge. Cell and part addresses keep these vertices distinct during region
assembly and border welding.

The cycle approach follows the uniform-grid discussion in
[Schaefer, Ju, and Warren, *Manifold Dual Contouring*, section III](https://people.engr.tamu.edu/schaefer/research/dualsimp_tvcg.pdf).
This pilot uses explicit bilinear face decisions and retains its fixed shell;
it does not implement that paper's adaptive octree algorithm.

Extraction now checks closed edge incidence, balanced winding, nonzero triangle
area, and one circular triangle neighborhood per vertex. It returns a topology
error if the result fails. Vertex checks catch pinched sheets that edge counts
alone can miss. The seed sweep reports these vertex defects and treats them as
failures in both fine and mixed-level meshes.

Medium still reduces fine, and coarse reduces medium. Blocks containing
multiple cell parts remain at the preceding level. Candidate reductions also
check the complete mesh's vertex neighborhoods; clusters around a failed link
are rejected, along with the existing orientation checks. Rejection repeats
until the candidate is valid. The fixed four-cell border and LOD density
ordering remain intact. Working-memory reservations now include the complete
mesh used for these checks; the 128 MiB managed limit and 512 KiB frame upload
limit remain unchanged.

This repairs demonstrated connectivity defects. It does not add samples or
recover features below the 64-by-64-by-16 shell resolution. Cell-boundary cycles
are the chosen interior reconstruction; this is not an exact solution of all
trilinear interior saddle cases. Manifold checks do not detect geometric
self-intersections or establish solid-body physics. Collision keeps its existing
two-sided fine-triangle contract, now using the repaired source. Changed support
triangles can change accepted placements and walking paths; those fingerprints
are expected to change. The field, presets, normal controls, and lighting stay
the same.

### Validation

The CPU suite covers all 256 corner-sign cases with equal and varied magnitudes,
all 48 cube rotations/reflections, planar and exact-zero cases, separate corner
sheets, and 32 ambiguous shell fixtures spanning cube seams and corners. A
pair of tetrahedra touching at one vertex verifies that closed edge counts
cannot hide a pinched vertex. Existing tests cover schedule invariance, LOD
ordering, orientation, exact border positions and normals, mixed-level joins,
contact, placement recreation, and replay serialization. All 33 tests pass.

The fixed CPU sweep uses hills, ridges, and basins with seeds 0, 42, and
4,294,967,338. All nine cases pass fine and mixed topology, the 36-second walking
route, contact clearance, and stationary support. Each has zero nonmanifold
edges and vertices and zero rest drift. Source build `5c7d6b2ae6ef176f` produced
this sweep; the later extraction-time topology gate repeats checks already
required by that audit. Records are in `/tmp/procgen-surface-slice2/sweep`.

| Preset, seed 42 | Previous fine triangles | Repaired fine triangles | Nonmanifold edges | Nonmanifold vertices |
| --- | ---: | ---: | ---: | ---: |
| Hills | 106,486 | 106,502 | 0 | 0 |
| Ridges | 123,834 | 123,854 | 0 | 0 |
| Basins | 83,220 | 83,232 | 0 | 0 |

Hills-42 now has quantized position fingerprint `8d28732d9c56f3b0` and 224
placements. Its pinned integer triangle fingerprint changes because cell parts
and added face vertices change canonical indices. Source and collision
preparation in the sweep took 574–995 ms; mixed-mesh audits took about
1.2–2.3 seconds. All nine cases remain marked expensive because at least one
walking step exceeded 4 ms. These runs establish geometry and contact checks,
not frame pacing. Windows/Vulkan validation remains open.

Native Metal checks on the same Mac completed a 36-second ridges-42 flight
with LOD colors and triangle edges (3,974 frames), then a neutral walking route
(4,280 frames), using build `548ecc3ee02e7593`. Both retained all six resident
faces, exercised all three LODs, installed no obsolete tickets, and logged no
errors or collision failures. Walking minimum recorded clearance was 0.035012.
The flight wireframe and ground screenshots show continuous terrain; large
facets remain visible at ground scale, as expected at the unchanged resolution.

Peak managed reservation was 130,071,816 bytes (about 124 MiB) in both runs,
with a maximum frame upload of 486,408 bytes. Both remain below their limits,
but the additional topology work leaves less memory headroom and can delay
replacement jobs. Screenshot readbacks were enabled; these runs do not establish
frame pacing. CSVs, summaries, view settings, replays, and screenshots are in
`/tmp/procgen-surface-slice2`. Formatting, the full headless test suite, and
Clippy with warnings denied pass.


## Slice 3: fine geometry

Fine render regions now split interior edges and place new vertices with fresh
queries of the final density field. The base contour remains 64-by-64-by-16.
Each edge has one shared new vertex. Its two incident triangles must both be
outside the fixed four-cell face collar to permit a split. One-, two-, and
three-edge triangle patterns fill the transition without hanging vertices.
The collar's positions, triangles, and normal stencil remain unchanged at every
LOD, including cube seams and corners.

New positions start at edge midpoints. The search direction is the normalized
sum of the endpoint mesh normals, and the search radius is one quarter of the
edge length. A sign bracket gets ten bisections of the composed density.
Stationary normals and missing brackets retain linear interpolation. A proposed
position that reverses or flattens any child triangle is rejected for the
shared edge; rejection repeats until all children retain parent orientation.
Original contour vertices stay fixed. This is bounded geometric refinement,
not an exact distance projection or a method for discovering unsampled caves.

The canonical refined CPU surface is prepared once for all six faces and kept
resident for collision and placement. Fine render products copy exactly its
triangles on nearby faces; coarse and medium use their existing base-contour
reductions. Collision does not switch during rendering fades, cancellation,
or retirement. This slice adds useful geometry to the bounded pilot without
claiming demand-paged density or arbitrary planet scale. General finer density
extraction remains separate work.

The refined surface and its BVH cost more memory. Managed reservations increase
from 128 to 256 MiB, with collision/population reservations increasing from 8 to
32 MiB. Region jobs reserve the larger of reduction scratch and fine output.
The source-preparation reservation stays 1 GiB and the upload limit stays
512 KiB per frame. These are application buffer reservations, not process-RSS
or driver limits.

Collision patches admit up to 32,768 triangle IDs. Closest-point queries visit
the existing BVH near-first and reject nodes farther than the current best
result. Membership in the patch's sorted IDs keeps the same candidate set.
This avoids a complete triangle scan on every conservative-advancement step;
collision continues to use exact triangle distances rather than density values.

### Validation

The sphere fixture shows over 90% reduction in midpoint radial error while
preserving original vertices, outward winding, a closed manifold, and identical
results with one and four workers. All eight split patterns preserve area,
winding, and the expected boundary. Mixed-LOD tests check closed edges and
vertex links, density ordering, and exact border positions and normals. They
also compare every fine render triangle directly with its collision triangle.
The BVH nearest search matches an exhaustive patch search. Existing contact,
placement, cancellation, GPU-readiness bookkeeping, and replay tests pass.


The fixed sweep again covers hills, ridges, and basins with seeds 0, 42, and
4,294,967,338. All nine pass fine and mixed topology, contact clearance, and
stationary support, with zero nonmanifold edges or vertices and zero rest drift.
The minimum clearances range from 0.035012 to 0.035037. Source and walking
fingerprints change because collision now includes the new geometry.

| Preset, seed 42 | Base triangles | Refined fine triangles | Resident source bytes | Collision index bytes |
| --- | ---: | ---: | ---: | ---: |
| Hills | 106,502 | 341,600 | 22,824,536 | 6,927,104 |
| Ridges | 123,854 | 398,504 | 26,170,752 | 7,382,336 |
| Basins | 83,232 | 266,360 | 20,369,432 | 6,325,184 |

Each 36-second walking route takes 0.15–0.25 seconds of total CPU walking work
in the audit, compared with several seconds in the preceding slice. Some
individual steps still have scheduling outliers, up to 15.4 ms in this run;
the sweep overlapped compilation, so it does not establish a frame-time bound.
Preparation now takes 2.4–4.6 seconds. All nine cases remain flagged expensive
because preparation exceeds the unchanged 1-second threshold. CPU records are
in `/tmp/procgen-surface-slice3/sweep`, using build `31dec0cb57994b24`.

All 37 tests pass, including a recording-finalization regression, as do Clippy
with warnings denied and formatting checks.
Windows/Vulkan validation remains open.


Native ridges-42 flight with LOD colors and edges completed 4,048 frames, and
neutral walking completed 4,251 frames. Both retained all six resident faces,
exercised all three LODs, installed no obsolete tickets, and recorded no
collision failures. Walking minimum clearance was 0.035012. Peak managed
reservation was 242,982,880 bytes across those runs; the largest frame upload
was 500,448 bytes. Both stay below their limits.

The same-camera comparison uses the two held poses from slice 1. Ground views
show new local relief and a changed silhouette; wireframe flight shows the
extra edges within fine faces. Large original triangles and narrow inherited
triangles remain apparent, so this is a bounded improvement rather than a
uniform high-resolution surface. Neutral shading and lighting stay fixed.
The older comparison predates the slice-2 topology repair as well; the slice-2
wireframe captures provide the closer geometry baseline.

The initial comparison also exposed an exit-frame recording bug: after saving
the replay, a final redraw tried to save a second replay with only one pose.
Completed recordings now ignore later frames. A regression verifies that CSV,
replay, and summary files remain unchanged after completion. The failed first
comparison is retained for evidence; `comparison-fixed` is the repeated run.

The corrected comparison completed successfully with 4,216 frames and 4,216
ordered replay poses, all taken from the input's two held views. It retained
complete coverage with no obsolete installations or collision failures.
Its peak managed reservation was 262,192,232 bytes (about 250 MiB), below the
256 MiB limit but with limited headroom; its largest upload was 479,232 bytes.
Build `1e9c92949d6d0387` includes the recording fix. All native artifacts are in
`/tmp/procgen-surface-slice3`. Screenshot readbacks were enabled, so these runs
establish coverage and visual comparisons rather than a frame-pacing bound.

## Slice 4: surface material detail

Neutral shading now adds a small normal perturbation and roughness variation.
The material stays gray under the existing fixed lighting. `--surface-detail
plain` restores the slice-3 material; `textured` is the new default. The same
setting is available in the inspector, saved in `.view.json`, and locked during
recording. LOD, normal-direction, and agreement overlays bypass material detail
so they continue to inspect the underlying surface. Triangle edges can still be
drawn over either neutral material.

The shader uses the existing `procgen-noise` gradient noise and its analytic
derivative. It composes the crate's WGSL source, which already includes the
canonical `procgen-core` hash. The pilot build identity now includes these WGSL
files so a shader-library change is visible in recording provenance.

Two fixed bands provide the material grain:

| Band | Lattice cells per model length | Slope strength | Roughness amplitude | Noise key |
| --- | ---: | ---: | ---: | --- |
| Coarser grain | 48 | 0.045 | 0.055 | `0x53555246` |
| Finer grain | 192 | 0.025 | 0.025 | `0x47524149` |

The stream meshes have identity transforms and contain planet coordinates.
Both bands sample that continuous 3D position. They need no texture UVs,
cube-face projection, per-region origin, tangent frame, or LOD-dependent seed.
The same planet position has the same pattern across region borders and mesh
replacement. Geometry displacement during a LOD change can still change the
sampled surface position. The fixed material keys are independent of the world
seed; this is one neutral material applied to all worlds.

The pixel footprint is the square root of the summed squared screen derivatives
of position. This bounds the footprint in diagonal and grazing views. Each band
fades smoothly between 0.12 and 0.4 lattice cells per pixel; a fully suppressed
band skips its noise query. Screen derivatives are evaluated before visibility
dither and divergent branches. This is conservative distance filtering, not
temporal antialiasing.

The analytic noise gradient is projected onto the selected normal's tangent
plane. Its combined slope is capped at 0.08, limiting normal tilt to about
4.6 degrees. The shading normal changes; the normal used for shadow bias stays
at the selected base normal. Roughness uses bounded noise contrast around 0.9,
remaining within 0.82–0.98 and returning to 0.9 when both bands fade out.

This change does not alter density, meshes, silhouettes, collision, placements,
streaming budgets, or mesh buffer sizes. It adds at most two noise evaluations
per neutral terrain fragment, with no texture allocation. The initial overview
and the separate population props retain their existing plain materials.

### Validation

All 37 CPU tests pass. Clippy passes with warnings denied, both with the
inspector and without default features. Formatting and diff checks pass. The
CLI rejects an unknown detail mode and rejects surface options without
`--stream`. Native Metal pipeline compilation succeeds.

Ridges, seed 42, was captured with the same two held poses used in slice 3.
The plain ground and flight images match the previous slice pixel-for-pixel
outside the sidebar. Textured shading adds visible grain at ground level and
less contrast from flight, with the same silhouette and large terrain forms.
The held textured flight images at 19.5 and 31.5 seconds are identical outside
the sidebar. This verifies a stationary pattern; it does not establish freedom
from aliasing under all camera motion. Large inherited triangles and facets
remain visible.

The matched plain/textured replays and a separate moving flight run all retained
six resident faces, exercised all three LODs, and installed no obsolete tickets.
CSV frame counts match saved replay pose counts. All runs completed successfully.

| Capture, build `be14227f3508e100` | Frames | Peak managed bytes | Largest frame upload |
| --- | ---: | ---: | ---: |
| Textured, held poses | 4,013 | 262,192,232 | 479,232 |
| Plain, held poses | 3,988 | 262,192,232 | 479,232 |
| Textured, moving flight | 4,085 | 240,756,416 | 500,448 |

Final build `97bc32d6655f7a8d` reads baseline roughness from the material and
adds the noise delta. Its repeated textured replay (`verified`) completed
4,185 frames with the same peak reservation and largest upload as the held
captures above, complete coverage, and no obsolete installations.

These remain below the 256 MiB managed limit and 512 KiB upload limit. Captures
and view settings are in `/tmp/procgen-surface-slice4/final`. Screenshot
readbacks and some concurrent CPU checks prevent a useful GPU timing comparison.
Windows/Vulkan validation and a controlled GPU cost measurement remain open.
