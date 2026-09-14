# Real-time pilot: physical scale and octree LOD

Status: slice 1 implements physical broad terrain, an octave editor, and bounded
CPU height previews. Slice 2a adds octree addresses and canonical density chunks
with one-meter finest spacing. Slice 2b adds bounded camera-driven density
residency and a headless travel audit. Slice 2c adds conforming chunk meshes
and independent one-meter collision support. Slice 3 adds the integrated native
viewer, precise physical points, bounded complete mesh replacement, and walking.
Generation latency and Windows/Vulkan validation remain open. This phase belongs
only to the independent real-time pilot.

GPU generation is now required for visual terrain. The next implementation
sequence is [GPU generation and incremental streaming](realtime-world-gpu-streaming.md).
The complete CPU mesh path below remains the reference baseline, not the target
architecture for normal exploration.

## Requirements and source evidence

- Give planet radius, elevation, feature wavelength, and local detail independent
  physical dimensions. All new generation lengths are meters; the UI may display
  kilometers. Increasing radius must not also enlarge mountains or the player.
- Target nominal one-meter voxel spacing at the nearest LOD, with coarser distant
  chunks. McKendrick 21:53–22:13 explicitly describes one-meter-cubed voxels and
  32-meter regions (the transcription labels one sentence `21:60`). At
  18:20–18:41 he notes the cube/sphere mapping changes voxel density, so this is
  not a claim that mapped cells are perfect one-meter cubes everywhere.
- Use an octree for chunk subdivision and residency. McKendrick 24:14–25:58
  describes doubled region sizes, reduced distant density, and octree storage.
  At 26:34 onward he also describes cheaper non-voxel distant terrain. We cannot
  assume that six voxel LODs alone can cover a planet at the new scale.
- Separate broad elevation from the local density band. McKendrick 20:26–21:18
  describes a roughly 128-meter band moved by a broad elevation offset. Those
  dimensions are evidence, not an instruction to scale all current constants.
- Begin the broad terrain stack at continental wavelengths, then add finer
  bands. Expose each band's amplitude and shape for user tuning. This is our
  explicit composition of Murray's concepts, not recovered source-game code.

The old radius-4 pilot remains an independent comparison. Its nominal source
spacing would be about 100 meters tangentially and 44 meters radially if assigned
one kilometer per model unit. Its render refinement is not one-meter sampling.

## Slice 1: physical dimensions and octave editor

`cargo run -p procgen-realtime-pilot` opens the GPU orbit/descent viewer with
`planet-design-300km.json`. Design and Octaves tabs provide live controls.
The earlier standalone height-preview editor is removed; its CPU preview
generator and headless audits remain. `--seed U64` explicitly selects the
unchanged starter: a 2,000 km radius, 18 bands from 1,048.576 km to 8 m,
4,000 m first amplitude, 0.55 amplitude decay, and a 12,000 m height bound.
Zero elevation is the reference sphere; there is no water simulation.

`PlanetDesignConfig` owns meter-based dimensions and ordered octave records.
It validates to an immutable CPU field. Each record contains enable state,
wavelength, amplitude, sharpness, warp, slope damping, altitude damping, and ridge
damping. Wavelengths must strictly decrease. The supported editing envelope is
radius 100–8,000 km, wavelength 8 m–16,000 km, and 1–24 octave records.

The field reuses the normalized gradient basis and calibrated sharpness transform
from the relief correction. Unlike the old six-band composition, amplitudes are
explicit meter coefficients; there is no gain or octave-count normalization.
In broad-to-fine order, accumulate unshaped basis derivatives for slope, ridge,
and warp feedback. Altitude damping reads the preceding sum divided by the
configured height bound. Apply each band's amplitude and damping to its shaped
value. Finally bound the total as `h / sqrt(1 + (h / height_limit_m)^2)`.
Amplitude is therefore a coefficient before feedback and bounding, not a promise
that a peak will equal that value. Changing one band can affect later bands
through feedback. Disabled and zero-amplitude bands have no feedback effect.

The headless preview generator retains two bounded height previews:

- **Planet:** a welded cube-sphere grid, with physical radius and elevation.
- **Local patch:** a curved tangent patch at editable latitude/longitude, spanning
  128 m to 100 km. Coordinates are relative to its center surface. Curvature is
  evaluated without subtracting two large planet radii.

Both use power-of-two grids, up to 256 quads per side. They retain physical
dimensions with no height exaggeration. The grid is a diagnostic
height mesh, not a voxel volume. Globe sample spacing uses a conservative
reference-sphere estimate; patch spacing is nominal tangent spacing. Neither is
an exact bound on distance along steep terrain or on warped noise bandwidth.

Unresolved noise fades smoothly between four and two samples per wavelength.
The fade multiplies height and derivative feedback; fully filtered bands cost no
noise query. The headless report includes each band's applied preview weight. A globe
cannot show meter-scale detail; a small patch can audit those bands.
The full-resolution elevation percentiles are sampled separately over 65,536
approximately equal-area directions, so they describe the selected field over
the planet, not the visible patch or filtered mesh extrema.

The headless `--solo` option evaluates a band alone, including a disabled band
selected explicitly. It does not isolate that band's marginal effect in the stack.

The GPU viewer applies valid edits 350 ms after the last change. Every draft
change invalidates older pending results, including when the draft is invalid.
A single worker drains bounded GPU submissions before replacing a design;
complete height and local voxel coverage, plus nearby collision when needed,
publish as one design revision. The camera is not reset. New collision requests
carry their immutable field, so old completions cannot replace new support.

**Save controls**, **Load**, and **Copy JSON** exchange a strict version-1 file
containing seed, radius, height bound, and every octave value. Save writes the
valid editable controls, including edits not yet displayed. It only runs on an
explicit action and overwrites the current loaded file. The displayed path is
read-only; Load and Save as use explicit path dialogs. Camera and coloring remain
viewer preferences. Loading validates
the entire design and applies it automatically. Status uses a fixed area, and
editable controls have stable IDs so updates cannot interrupt typing.

Headless examples:

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --design --seed 42 --check --write-design /tmp/planet-design.json
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --design --design-file /tmp/planet-design.json --check --patch-span 128 --solo 17
cargo run -p procgen-realtime-pilot -- \
  --design --design-file /tmp/planet-design.json
```

The last band has an 8-meter wavelength, rather than a one-meter wavelength:
a one-meter sampling target does not imply one-meter features can be resolved.
The preview uses the existing f32 CPU noise primitive in its bounded editing
domain. Slice 2a adds integer meter addressing and a stable radial-altitude
calculation for density samples. The noise primitive remains unchanged; its
precision and any future GPU mirror still need agreement checks before ground
collision depends on them.

## Slice 2: bounded octree density generation

Replace whole-shell residency with addressed chunks. Start with nominal 32³-meter
nearest regions and one-meter finest voxel spacing. Define power-of-two spatial
addressing, shared samples and halos, world/chunk transforms, and a local origin
representation before implementing traversal. Test shared-face, edge, corner,
and parent/child agreement, including at the maximum supported planet radius.

Then implement a bounded working set driven by the camera, local density-band
generation over broad elevation, cancellation, eviction, and coarse/fine mesh
boundaries. Keep near collision tied to the canonical density/surface, independent
of render replacement. Settle the CPU expression order and coordinate contract
before adding any WGSL mirror. Do not retain a planet-wide fine source or use
radius scaling as a substitute for addressing.

Acceptance: nearest active samples meet the meter-scale contract; distant chunks
coarsen; budgeted residency stays bounded during travel; revisiting an evicted
location recreates the same field and surface; shared boundaries remain valid.
CPU remains canonical. GPU work, when introduced, must support Metal and Vulkan.

### Slice 2a: integer addresses and density chunks (implemented)

Voxel storage now uses one planet-centered **Cartesian octree**, rather than the
six surface-face charts used by the coarse preview. This is a deliberate pilot
choice: regular world-space cells give exact meter edges, share face/edge/corner
samples without chart transforms, and keep storage independent of terrain shape.
The terrain function still evaluates a sphere. Interior/empty octree nodes will
be culled by the working-set stage; the root does not allocate its descendants.
The cube-sphere mapping remains useful for the cheap height overview.

The root spans 16,777,216 meters, centered on the origin. It encloses the current
maximum radius (8,000 km) and relief bound. Root LOD is 19; LOD zero is finest.
Each subdivision produces eight children with half the physical span and spacing.
Every chunk has 32 cells per axis. Thus LOD 0 spans 32 m with 1 m spacing, LOD 1
spans 64 m with 2 m spacing, and so on. Chunk indices are unsigned integers from
the root minimum; world sample positions are signed integer meters. Half-open
chunk ownership handles negative coordinates and zero without ambiguous cells.

Each volume includes 33 endpoint samples plus one halo sample on each side:
35³ = 42,875 f32 potentials, or **171,500 bytes of density payload**. Halo samples
are addressed before any float conversion. Neighboring chunks and parent/child
chunks therefore query the same world position at coincident nodes. Relative
positions subtract integer origins before conversion to render coordinates.
Integer addressing has a pinned fingerprint across LODs and negative positions.

`sample_voxel_chunk` is a bounded, parallel CPU job. It samples the existing saved
design's **complete** octave stack, independent of chunk LOD. The public density view is broad
surface elevation minus radial altitude, clamped to +/-4 m, positive for solid.
It is not a signed-distance function. Since slice 2c, the same buffer retains
the unsaturated potential for mesh interpolation; public density queries still
clamp to +/-4 m. Sampling now evaluates the full height function outside the
relief envelope too, because coarse edge roots need the potential magnitudes.
This establishes height-derived density; local caves and overhangs are not yet
part of the physical design contract.

Radial altitude uses `(dot(p,p) - radius²) / (length(p) + radius)`, with compensated
f32 products and sums in the numerator. This avoids cancellation between rounded
planet radii. Tests compare it with an independent f64 oracle at axis, oblique,
and corner directions, radii through 8,000 km, and +/-20 km offsets, within 2 mm.
There is no f64 or 64-bit integer requirement in the sampling implementation.
The existing f32 noise function and its precision limits remain unchanged.

Full-stack density keeps identical values at coincident samples across LODs.
The preview's octave fading is not silently applied to collision/source density.
Slice 2c keeps this full-band policy for mesh potentials and interpolates derived
face/cell centers from the finest incident source grid. It adds no octave fade.

Run a bounded headless audit with a saved design:

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --design --design-file /tmp/planet-design.json --check-chunks
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --design --check-chunks --chunk-lod 1 --chunk-point 2000197,0,0
```

Without a point, the command chooses the +X surface and quantizes that probe to
integer meters once on the host. Explicit points use integer `x,y,z` meters.
The JSON reports the root, selected address and children, sample spacing, payload
size, density range, and neighboring/parent agreement. It compares 3,675 face and
halo samples with an X neighbor and 4,913 coincident samples with the parent.
At most two density payloads remain live: the neighbor is released before the
parent is sampled. The 343,000-byte paired payload bound excludes the shared
field, thread pool, allocator overhead, and process memory. The root has neither
a neighbor nor parent, so those comparison counts are zero.

Validation: all 56 tests pass (53 library, three headless binary), including eight
new address/density/audit tests. Both Clippy configurations, build, and formatting
checks pass. Headless audits cover the default saved preset, radii 100 km and
8,000 km, seeds 0/42/u64::MAX, negative/oblique positions, LODs 0/1/8/19, and
invalid inputs. Every checked coincident density matches exactly on the CPU;
the largest paired density payload is 343,000 bytes. No GPU density mirror or
new rendered mesh is claimed by these checks.

### Slice 2b: camera-driven residency (implemented)

`VoxelResidency` owns an immutable design field, a camera-selected octree, and
asynchronous CPU density jobs. Selection refines the nearest boxes first within
two chunk spans of the camera, down to LOD zero. Coarser boxes win equal-distance
ties, followed by address order. A hard leaf cap stops refinement while retaining
the parent, so distant surface coverage remains present. The default cap is 512 leaves; this is a
bounded pilot working set, not a promise of uniform detail throughout the reach.
Selection reruns only when the integer camera position changes.

Culling uses the box's nearest/farthest radius against the global relief bound,
including the density clamp, sample halos, and a 16 m float-rounding margin.
It removes only proven interior or exterior boxes. Corner density signs alone
cannot prove emptiness with unresolved noise. Distant leaves remain cheap address
metadata; only leaves within a 4,096 m camera reach allocate density. The existing
height globe remains the distant visual source for the later viewer integration.

The default owner admits at most two workers, and each update processes at most
two completions and two admissions. Each worker allocates its final density
buffer once and evaluates parallel rows. It checks cancellation before each row.
Eviction releases obsolete resident chunks immediately. Cancelled workers retain
their slot and reservation until completion. Request serials reject obsolete
results, including a leave-and-return to the same address. Replacing/dropping the
owner cancels and joins workers before releasing their reservations; its field
cannot change beneath a running job.

The worst-case density payload reservation is `(max_leaves + max_jobs) * 171500`:
**88,151,000 bytes (84.1 MiB)** with the defaults. This includes completed results
waiting in worker channels and cancelled jobs. It excludes leaf metadata, the
shared field, thread stacks, Rayon, allocator overhead, and process memory.
The travel audit adds one canonical reference volume at a time (171,500 bytes).
No GPU upload or mesh storage is included in this density-only budget.

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --design --design-file planet-design.json --check-residency
```

The audit visits orbit, the +X surface, rapid travel positions, the opposite
hemisphere, the original surface, and orbit again. It reports leaf fingerprints,
spacing, live/peak reservations, scheduling counters, and CPU update time. Every
settled density sample is checked against a fresh canonical chunk. A changed
revisit address set or density mismatch fails the audit. This is a headless
consumer of the residency owner; the editor and old `--stream` renderer have not
yet been connected to these chunks.

Validation with the saved seed-42, 4,900 km preset reached 1 m spacing on both
hemispheres, retained 256 leaves, and installed 128 nearby chunks at each ground
stop. Peak density payload was 21,952,000 bytes (20.9 MiB), with at most two jobs.
All 16,464,000 checked samples matched exactly; the revisited leaf fingerprint
matched, and returning to orbit released every density chunk. Cancellation and
rejection counts depend on worker timing. Tests also force completion order and
leave/return races, compare one-worker and four-worker results, check conservative
shell coverage through 8,000 km, and verify eviction/recreation at bounded memory.
All 63 tests pass with inspector features (62 without); the build, both Clippy
configurations, formatting, and CLI rejection checks pass. On this macOS run,
the audit took about 15 seconds including canonical rechecks, and the slowest
CPU residency update was 1.17 ms. These are local measurements, not frame-time
guarantees or Windows/Vulkan results.

Leaves are not balanced to a 2:1 neighbor ratio. Slice 2c accepts arbitrary
dyadic ratios through shared face subdivisions. Slice 2b establishes density
residency; its measurements do not include meshing, collision, or frame time.

### Slice 2c: voxel meshes and nearby collision (implemented)

The physical voxel path now uses **conforming tetrahedral extraction**. This is
an explicit change from the original pilot's dual-contouring candidate. Shared
face triangles provide a direct boundary contract for arbitrary chunk LOD ratios
and a canonical surface for collision. The original shell/QEF experiment remains
available for comparison; this new path does not fit feature vertices with QEFs.

Each source voxel is decomposed around its center and face centers. At a mixed
boundary, the coarse face subdivides to the incident fine grid. All face corners
also split incident edge lines, so face seams, edge seams, and chunk corners use
the same triangles. Half-meter integer node identities represent derived centers;
source voxel corners still use their exact one-meter-or-coarser lattice. Centers
interpolate the finest incident source volume. They do not introduce extra noise
samples or a half-meter source LOD. Shared edge-root identities weld the output,
and triangle ownership records its source chunk. A coarse cell is included when
fine boundary samples cross it even if its own eight corners have one sign.

The volume stores one unsaturated potential per sample and derives its existing
+/-4 m density view. Clamping before interpolation would move a coarse edge's root
toward its midpoint; the mesh now preserves the magnitude information it needs.
Every LOD uses the full octave stack. This preserves coincident field values and
is the explicit filtering policy for this slice. It does not claim to eliminate
coarse-grid aliasing. Mesh roots interpolate from the endpoint nearest zero to
retain small radial curvature at large radii. Exact-zero roots share node IDs;
there is no arbitrary density epsilon or position quantization.

`build_voxel_surface` consumes the actual resident volumes, with a local integer
origin inside the octree root. It rejects overlapping/duplicate input chunks,
invalid volumes, exceeded capacities, and invalid topology. Each open edge must
be on an intentional outer support boundary; shared chunk edges must have two
oppositely oriented incident triangles. Interior vertex links and zero-area
triangles are checked too. Cancellation discards the complete batch result.
The later renderer must install complete replacements and retain prior visual
coverage while a job runs; this headless slice does not add GPU upload handling.

Generation has explicit caps: 512 source chunks, 524,288 active cells, 3,145,728
shared faces, 4,194,304 surface vertices, and 8,388,608 triangles. These bound
geometry and scratch data independently of world size. They are count limits,
not a claim that the density-only 84.1 MiB reservation includes mesh storage.
The audit reports allocated mesh vector payload separately; tree-node allocation,
thread stacks, allocator overhead, and temporary construction data are excluded.
A replay audit briefly holds both complete mesh outputs for exact comparison.

`VoxelCollision` builds a separate 3x3x3 set of finest chunks around the local
origin: a 96 m support box, with 1 m source cells. Its 27 source payloads total
4,630,500 bytes during construction and are released after extraction. The
collision owner retains the resulting surface independently of render residency.
It supplies finite-segment queries and swept spheres in local meters. Sweeps use
exact triangle distance, a 1 mm contact skin, and bounded conservative advancement;
density magnitude is never used as a distance bound. Missing coverage, overlap,
and iteration exhaustion return errors. Queries are two-sided and do not classify
the solid interior. Player locomotion and collision-support replacement remain
part of the integrated viewer work.

```sh
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --design --design-file planet-design.json --check-surfaces
```

This meshes the saved design's resident set, leaves for orbit to evict it, then
regenerates the original location with reversed source order. Both positions and
triangle identities must match exactly. It also builds fine collision, checks
the landing ray against the mixed mesh within 1 mm, and stops a 0.4 m-radius
sphere on that surface. General coarse render geometry can differ from fine
collision; the audit's landing probe is at nearby fine support.

Validation used the saved seed-42, 4,900 km preset and copies at 100 km/seed 0
and 8,000 km/seed u64::MAX. All three passed topology, exact eviction/revisit,
reversed-order, landing-ray, and sphere-sweep checks. The saved preset used 128
resident chunks and produced 632,495 vertices and 1,263,460 triangles, with
1,528 intentional outer boundary edges and no invalid interior topology. Its
allocated mesh vector/boundary-key payload was 71,315,392 bytes (68.0 MiB), plus
21,952,000 bytes of source density. Fine collision used 186,308 triangles and
8,922,160 bytes of mesh payload. Local CPU construction took about 3.1 seconds
for the mixed mesh and 0.9 seconds for collision, excluding resident generation.
This is a relatively dense, serial extraction baseline; neither these times nor
the mesh counts are a real-time frame-rate claim. Windows/Vulkan is unmeasured.

All 68 tests pass with inspector features (67 without), along with the build,
both Clippy configurations, formatting, and invalid CLI-option checks.
Tests cover closed surfaces across mixed faces, edges, and corners, a small fine
feature whose coarse corners miss the crossing, 32:1 transitions, exact-zero
planes, coarse-plane altitude, large-origin curvature, reversed input order,
cancellation, source validation, and two-sided collision/coverage failures.
This slice provides the CPU surface and query contracts. Continuous visible
planet-to-ground travel, walking, render replacement, and measured frame/upload
budgets still require slice 3.

## Slice 3: integrated planet-to-ground viewer

Connect the editable design to octree terrain, nearby collision, and cheap distant
coverage. Give the player and camera physical dimensions and speeds. Show meters
or kilometers for altitude and distinguish reference altitude from ground clearance.
Filter geometric and material detail to their actual resolution. Preserve the
neutral inspection views and saved-preset workflow.

Acceptance: continuous orbit-to-ground travel, stable walking, no exposed LOD
seams, deterministic revisits, and measured memory, upload, generation, and frame
budgets on macOS/Metal and Windows/Vulkan. This does not add climate, tectonics,
biomes, or dependencies on the older world pipeline.

### Slice 3 implementation and use

```sh
cargo run -p procgen-realtime-pilot -- \
  --design --explore --design-file planet-design.json
cargo run -p procgen-realtime-pilot --no-default-features -- \
  --design --design-file planet-design.json --check-explore
```

The default exploration viewer includes the live design controls described above.
It changes a file only when Save controls or `--write-design` is requested.
`--backend cpu` remains an explicit visual audit with a fixed design per run.
The original `--stream` experiment remains separate.

**Orbit** resets to three reference radii from the center. Left drag rotates the
view; scroll changes clearance. **Descend continuously** follows the current
radial direction toward the ground without a teleport, then switches to flight.
**Go to ground** is a direct shortcut to about five meters above the same radial
terrain location. In flight, W/A/S/D move along the view, E/Q move radially, right
mouse drag changes direction, and scroll adjusts the speed factor. Speed reduces
near the ground; outward flight stays within the orbit control's six-radius
clearance envelope. **Walk** lands on nearby collision support; W/A/S/D then move a
0.4 m-radius sphere at 4 m/s with a 1.7 m eye height. Movement stops when collision
support is missing. There is no jump, step-climbing, or capsule controller.

Reference altitude is distance above the configured reference sphere. Ground
clearance is a ray against the independent collision mesh and is unavailable
outside that support. The separate field-clearance estimate uses the analytic
height function; it is not a collision measurement. Zero elevation still does
not imply water.

`MeterPosition` stores integer meters and a centered sub-meter offset. Camera
translation rebases before adding the next step. Surface vertices also retain
this representation until a consumer chooses its local origin. Triangle winding,
area, and normals use triangle-local differences. Complete distant coverage
exposed sub-centimeter intersections that collapsed when stored relative to one
large camera origin; keeping the integer anchor fixes that failure without
changing density, dropping triangles, or relaxing topology checks. The renderer
converts shared vertices relative to one generation origin and translates that
origin relative to the current camera anchor. GPU positions remain f32; tiny
remote triangles can become subpixel or collapse in the display representation.
The CPU surface retains their geometry.

`PhysicalTerrain` uses the 512-leaf residency selection and reuses its
nearby density chunks. For each complete surface, it also samples distant leaves
on the terrain worker, then releases those temporary densities. This deliberately
closes the entire planet instead of placing a separate height surface across an
open voxel boundary. Initial coverage is a cheap 64-quad cube-sphere height
preview with spacing-filtered octaves. Subsequent complete voxel replacements
retain the canonical full-band potentials from slice 2c. Neutral shading adds no
material noise; LOD colors and averaged normals are available for inspection.
Geometric octave filtering for mixed voxel levels remains future work.

One terrain worker builds a complete replacement at a time. Camera movement
coalesces into the next request; the previous complete surface stays visible.
The viewer requests new geometry after movement exceeds a clearance-dependent
threshold, with a 16 m minimum. Worker-side packing produces indexed pieces of
at most 3,584 triangles. Main-thread admission is limited to 512 KiB per frame
and a 2 ms admission window; one piece can exceed that time window. Every piece
must be prepared in Bevy's render world before the new mesh is revealed. The
old mesh is then removed, and the renderer acknowledges GPU retirement before
another terrain build starts. Replacement is atomic and can visibly pop; there
is no morph or dither transition in this mode.

Collision has its own worker and retains the previous 96 m support cube during
replacement. Results that no longer cover the camera are discarded. A static
triangle BVH replaces the full scan in rays and swept-sphere queries, with
canonical triangle-order tie-breaking. Walking uses radial gravity, bounded
substeps, sliding, and short support sweeps. Missing support leaves the prior
movement state unchanged. The collision mesh is independent of visual source,
mesh upload, and retirement.

Density payload during a complete build is bounded by 512 chunks (83.7 MiB),
plus the existing density worker reservations while they run. Mesh extraction
retains its cell, face, vertex, and triangle caps. At most one displayed mesh and
one replacement are held by the renderer. Collision owns one retained patch and
at most one replacement. The panel reports source, mesh, collision, and displayed
buffer payloads, CPU generation and upload time, walking update time, and frame
time. These counters exclude allocator, extraction tree, driver, and process
memory overhead; the 512 KiB admission limit is not a GPU execution-time limit.

The headless exploration audit builds closed orbit and ground surfaces, releases
visual residency, walks on independent collision, replaces that collision patch,
and compares motion with the retained patch. It also checks idle stability.
The 100 km/seed 0, saved 4,900 km/seed 42, and 8,000 km/u64::MAX presets exercise
small and large coordinate scales. Focused tests cover centimeter translation,
40 m of walking across chunk boundaries, missing support, indexed versus complete
ray queries, closed distant coverage, and tiny intersections at an 8,000 km
mesh origin. Existing deterministic revisit and input-focus tests still run.

This is the first integrated CPU baseline. With corrected octant allocation, a
complete ground mesh before the relief retune had 4,396,166 triangles, 83.7 MiB
of source density, and 320 MiB of allocated CPU mesh payload with anchored
vertices. Complete builds take seconds, not frames;
nearby visual updates can lag travel. The leaf cap does not promise uniform
one-meter detail throughout the collision cube, and coarse geometry can differ
from fine collision. Those limits, visible replacement pops, geometric filtering,
and the Windows/Vulkan run remain open against the slice's acceptance targets.

Initial validation before the allocation correction: 73 tests passed with
inspector features and 72 without. Both Clippy
configurations, the native build, formatting, and CLI mode checks pass. All three
exploration presets pass closed coverage, walking, collision handoff, and zero
idle drift. The saved preset also passes the original exact eviction/revisit and
reversed-source audit, with zero landing-ray difference. The 100 km and 8,000 km
complete ground builds used 2,527,034 and 3,407,596 triangles respectively. Measured
CPU build times were roughly 13–19 seconds across these runs; these are local
observations, not hard timing bounds. Collision used about 14.4 MiB including its
BVH. Handoff differences stayed below 0.12 mm and idle drift was zero.

Native Apple M1 Max/Metal checks exercised the initial preview, complete voxel
replacement, orbit, ground travel, diagnostics, and landing with 1.70 m clearance.
Observed steady frames were about 8–11 ms and CPU upload admission stayed below
0.4 ms in that check; startup and generation can produce larger frame spikes.
Continuous descent was started on the final geometry build, but the Mac locked
before its final near-ground state could be inspected. Windows/Vulkan remains
unmeasured. The commands above provide the same audit and viewer on that host.

### Ground relief and octant allocation correction

The saved 4,900 km preset exposed an allocation defect at the +X landing point.
Several octants had zero distance to the camera. Address-only tie-breaking spent
the leaf budget refining one octant while another touching octant retained
262,144 m source spacing. Selection now refines coarser boxes first at equal
distance. The default cap increased from 256 to 512 leaves because the balanced
256-leaf set stopped at 32 m spacing there. Surface capacities increased in
proportion. The corrected selection gives 1 m source spacing across the landing
point, including offsets through 32 m in the measured tangent direction.

A regression checks all six axis landings at radii 100 km, 4,900 km, and 8,000 km,
including samples on each side of both octant boundaries. The saved-preset full
exploration audit passes closed coverage and collision handoff with zero idle
drift. Its complete ground build took 29.7 seconds on the M1 Max; orbit took
2.0 seconds. These measurements replace the earlier 256-leaf performance figures
above for this preset. Source and mesh payloads exclude extraction scratch,
allocator, driver, and renderer replacement storage.

The corrected saved-preset seam/revisit audit reproduces the mesh exactly with
reversed source order and zero collision-ray difference. The 8,000 km/u64::MAX
exploration audit also passes, with 4,997,724 ground triangles, one-meter finest
source spacing, a 0.033 mm collision-handoff difference, and zero idle drift.
After the correction, 74 tests pass with inspector features and 73 without.

The allocation correction left the saved noise settings unchanged. Before the
subsequent relief retune, their amplitude-to-wavelength ratio was 1/256 in every band: 4 m at 1,024 m wavelength and 0.5 m at 128 m wavelength.
A 65-by-65 full-field probe over 128 m around +X measured 4.44 m of total elevation
range, a fitted grade of 2.52%, and 1.75 m of remaining range after subtracting the
best-fit plane. This is gentle terrain even at correct visual spacing. The global
height sample spans approximately -9.0 to +11.2 km; at 4,900 km radius, the maximum
sampled elevation is only 0.23% of the radius. Orbital silhouette smoothness is
therefore expected. Stronger local relief requires tuning middle and fine bands,
separately from this allocation correction.

### Saved preset relief retune

The first retune of `planet-design.json` (seed 42, radius 4,900 km, height limit
12 km) gave middle and fine bands more amplitude. Bands 0–3 retain their continental
contributions. Band 4 (131,072 m wavelength) increases from 512 to 1,024 m;
band 5 (65,536 m) increases from 256 to 1,024 m. Bands 6–17 increase eightfold,
from 1,024 m amplitude at 32,768 m wavelength down to 0.5 m at 16 m wavelength.
This ramp adds hills and local relief without changing the broadest bands,
noise shaping, erosion controls, or the physical scale. It changes this saved
preset only; the explicit `--seed` starter preset retains its existing settings.

A 65-by-65 full-field probe around the +X landing point measures:

| Patch span | Previous elevation range | Retuned elevation range |
| --- | --- | --- |
| 32 m | 0.85 m | 3.77 m |
| 128 m | 4.44 m | 20.17 m |
| 1,024 m | 21.89 m | 97.34 m |
| 8,192 m | 314.52 m | 1,771.15 m |

Over 128 m, the fitted grade rises from 2.52% to 8.71%. The remaining range after
subtracting the best-fit plane rises from 1.75 m to 13.91 m, confirming increased
local variation rather than just a larger regional slope. The global sample
range changes from approximately -9.0/+11.2 km to -9.6/+11.2 km. Amplitudes are
coefficients before feedback damping and the soft height bound; they are not
promised peak-to-valley ranges. These measurements describe this landing area,
not a global slope guarantee.

The retuned preset passes the complete orbit/ground mesh and walking audit.
The ground surface has 5,027,708 triangles, one-meter finest source spacing,
and 320 MiB of allocated CPU mesh payload; it built in 32.2 seconds on the M1
Max. Collision handoff differs by 0.016 mm and idle drift is zero.

### Stronger saved-preset relief

After visual inspection, bands 4–17 received another fourfold amplitude increase.
The four broadest band coefficients, seed, radius, height limit, and shaping
controls remain unchanged. Current amplitudes are 4,096 m at wavelengths
131,072/65,536/32,768 m, then halve with each wavelength down to 2 m amplitude
at 16 m wavelength. For example, the 1,024 m band now has 128 m amplitude and
the 128 m band has 16 m amplitude. The 12 km soft height bound still compresses
the combined elevation, so fourfold coefficients do not guarantee fourfold relief.

The same 65-by-65 +X probes measure the stronger result:

| Patch span | First retune elevation range | Stronger elevation range |
| --- | --- | --- |
| 32 m | 3.77 m | 13.33 m |
| 128 m | 20.17 m | 69.08 m |
| 1,024 m | 97.34 m | 413.05 m |
| 8,192 m | 1,771.15 m | 6,426.48 m |

Over 128 m, the fitted grade is 28.68% and the range after removing that plane
is 51.83 m. The global sample spans approximately -11.2/+11.5 km. This is a
substantial increase in local and regional relief; orbital silhouette changes
remain small at the physical planet radius.

The stronger preset passes the complete orbit/ground mesh and walking audit.
The ground mesh has 5,510,386 triangles, one-meter finest source spacing, and
320 MiB of allocated CPU mesh payload; it built in 37.2 seconds on the M1 Max.
Collision handoff differs by 0.079 mm and idle drift is zero.

## Slice 1 validation

The 45 library tests and three binary tests pass with inspector features disabled.
Coverage includes physical scale invariance, independent octave controls,
filtered/disabled feedback, bounded extremes, both halves of the seed, schedule
invariance, a closed globe, local curvature, and versioned preset round trips.
The original pilot's field fingerprints and terrain/streaming tests still pass.

Headless seed-42 checks generated the 196,608-triangle globe and a
32,768-triangle, 128-meter patch. The globe's reference spacing is 31.25 km;
bands 0–3 are fully present, band 4 nearly faded out, and finer bands absent.
The patch at 128 quads has nominal one-meter height-sample spacing, and solo
band 17 (8-meter wavelength) is fully present. This confirms preview filtering,
not completion of the voxel sampling or collision contract.

Native macOS/Metal inspection exercised planet and patch views, per-band disable,
automatic regeneration, Save, and Load. The scene has its own viewport so the
sidebar does not cover terrain. Clipboard support is enabled through the existing
eGUI integration for numeric/path paste and JSON export. Windows/Vulkan visual
validation remains open.
